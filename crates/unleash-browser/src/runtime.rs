//! Épinglage strict du runtime navigateur (`Chrome for Testing` + `chromedriver`),
//! vérification `SHA-256` et inventaire local passif pour `GetHealth`.
//!
//! Aucun téléchargement implicite ni lancement caché de navigateur n'est jamais effectué ici.
//! Safari est explicitement refusé en mode `mediated` V1 ([Q-18]).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unleash_domain::Sha256Digest;

use crate::error::BrowserError;
use crate::policy::ExecutionMode;

/// Moteur navigateur ciblé par le contrat d'exécution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngineKind {
    /// Pilote de référence V1 (`Q-18`) : `Chrome for Testing` + `chromedriver` du même build.
    ChromeForTesting,
    /// Profil optionnel hors recette V1 (`geckodriver`).
    FirefoxOptional,
    /// Safari / `safaridriver` : explicitement non pris en charge en `mediated` V1.
    SafariUnsupported,
}

/// Plateforme qualifiée de la matrice V1 (`Q-11` / `ADR-14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetPlatform {
    /// Apple Silicon macOS (15, 26, 27).
    MacOsArm64,
    /// Linux `x86_64` (Ubuntu 24.04 / 26.04 LTS).
    LinuxX86_64,
}

/// Binaire épinglé par chemin, version exacte attendue et empreinte `SHA-256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedBinary {
    /// Chemin absolu vers le binaire préparé sous mandat (sous `runtimes/`).
    pub path: PathBuf,
    /// Version exacte attendue (ex. `"156.0.8078.4"`).
    pub expected_version: String,
    /// Empreinte `SHA-256` attendue du binaire exécutable.
    pub expected_sha256: Sha256Digest,
}

/// Référence complète d'un runtime navigateur épinglé (`RuntimeRef`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedBrowserRuntime {
    /// Moteur retenu.
    pub engine: BrowserEngineKind,
    /// Plateforme cible.
    pub platform: TargetPlatform,
    /// Binaire du navigateur (ex. `Google Chrome for Testing`).
    pub browser: PinnedBinary,
    /// Binaire du pilote `WebDriver` (ex. `chromedriver`).
    pub driver: PinnedBinary,
    /// Version du mapping d'opérations `unleash-browser`.
    pub mapping_version: String,
}

/// Résultat vérifié de l'inspection passive d'un runtime épinglé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInspection {
    /// Moteur inspecté.
    pub engine: BrowserEngineKind,
    /// Plateforme déclarée.
    pub platform: TargetPlatform,
    /// Version vérifiée du couple navigateur/driver.
    pub verified_version: String,
    /// Empreinte `SHA-256` vérifiée du navigateur.
    pub browser_sha256: Sha256Digest,
    /// Empreinte `SHA-256` vérifiée du driver.
    pub driver_sha256: Sha256Digest,
    /// Liste des stratégies de locators V1 effectivement admises.
    pub supported_locators: Vec<String>,
    /// Confirme qu'aucun processus n'a été lancé et qu'aucun téléchargement n'a eu lieu.
    pub passive_inspection_only: bool,
}

/// Paramètres de recherche en lecture seule pour l'inventaire local (`GetHealth`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalInventorySearchPaths {
    /// Répertoire de cache Playwright éventuel (ex. `~/Library/Caches/ms-playwright`).
    pub playwright_cache_dir: Option<PathBuf>,
    /// Répertoire applicatif système éventuel (ex. `/Applications`).
    pub applications_dir: Option<PathBuf>,
    /// Répertoire `runtimes/` géré par Unleash.
    pub unleash_runtimes_dir: Option<PathBuf>,
    /// Liste de répertoires `PATH` pour détecter la présence éventuelle de `chromedriver`.
    pub executable_search_dirs: Vec<PathBuf>,
}

/// Entrée détectée lors d'un inventaire passif du poste local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedBrowserArtifact {
    /// Libellé du composant détecté.
    pub label: String,
    /// Chemin complet observé sur disque.
    pub path: PathBuf,
    /// Version lue dans les métadonnées locales si disponibles (sans exécuter le binaire).
    pub version: Option<String>,
    /// Empreinte `SHA-256` calculée sur le fichier si accessible.
    pub sha256: Option<Sha256Digest>,
}

/// Résultat factuel de l'inventaire local des runtimes navigateur pour `GetHealth`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRuntimeInventory {
    /// Binaires `Chrome for Testing` trouvés localement.
    pub chrome_for_testing: Vec<DetectedBrowserArtifact>,
    /// Binaires `chromedriver` trouvés localement.
    pub chromedrivers: Vec<DetectedBrowserArtifact>,
    /// Indique si au moins un couple appairé `Chrome for Testing` + `chromedriver` est présent.
    pub paired_chrome_driver_ready: bool,
    /// Note diagnostique factuelle en cas d'élément manquant.
    pub status_note: String,
}

/// Calcule l'empreinte `SHA-256` d'un fichier régulier sans charger le fichier complet en RAM.
///
/// # Errors
/// - [`BrowserError::RuntimeMismatch`] si le fichier est absent, n'est pas un fichier régulier
///   ou ne peut pas être lu.
pub fn compute_file_sha256(
    path: &Path,
    component_label: &str,
) -> Result<Sha256Digest, BrowserError> {
    let metadata = std::fs::metadata(path).map_err(|err| BrowserError::RuntimeMismatch {
        details_ref: component_label.to_owned(),
        message: format!(
            "Binaire '{component_label}' introuvable à '{}': {err}",
            path.display()
        ),
    })?;

    if !metadata.is_file() {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: component_label.to_owned(),
            message: format!(
                "Le chemin '{}' pour '{component_label}' n'est pas un fichier régulier",
                path.display()
            ),
        });
    }

    let file = File::open(path).map_err(|err| BrowserError::RuntimeMismatch {
        details_ref: component_label.to_owned(),
        message: format!(
            "Impossible d'ouvrir '{component_label}' à '{}': {err}",
            path.display()
        ),
    })?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 65_536];

    loop {
        let read_bytes = reader
            .read(&mut buffer)
            .map_err(|err| BrowserError::RuntimeMismatch {
                details_ref: component_label.to_owned(),
                message: format!(
                    "Erreur de lecture sur '{component_label}' à '{}': {err}",
                    path.display()
                ),
            })?;
        if read_bytes == 0 {
            break;
        }
        let slice = buffer
            .get(..read_bytes)
            .ok_or_else(|| BrowserError::RuntimeMismatch {
                details_ref: component_label.to_owned(),
                message: "Tranche de lecture hors limites".into(),
            })?;
        hasher.update(slice);
    }

    let raw_bytes: [u8; 32] = hasher.finalize().into();
    Ok(Sha256Digest::from_raw_bytes(raw_bytes))
}

/// Vérifie de façon purement passive l'épinglage strict du runtime navigateur avant ouverture de contexte.
///
/// Refuse explicitement Safari en mode `mediated` (`CapabilityMissing`), vérifie la présence,
/// l'empreinte `SHA-256` et la parité de version entre navigateur et driver, sans jamais
/// lancer de processus ni déclencher de téléchargement implicite.
///
/// # Errors
/// - [`BrowserError::CapabilityMissing`] si Safari est demandé en mode `mediated`.
/// - [`BrowserError::RuntimeMismatch`] si un binaire manque, que son `SHA-256` diverge ou que
///   les versions du navigateur et du driver divergent.
pub fn verify_pinned_runtime(
    runtime: &PinnedBrowserRuntime,
    mode: ExecutionMode,
) -> Result<RuntimeInspection, BrowserError> {
    if runtime.engine == BrowserEngineKind::SafariUnsupported && mode == ExecutionMode::Mediated {
        return Err(BrowserError::CapabilityMissing {
            details_ref: "safari_mediated_unsupported".into(),
            message: "Safari/safaridriver n'est pas pris en charge en mode mediated V1 (Q-18)"
                .into(),
        });
    }

    if runtime.browser.expected_version.trim().is_empty()
        || runtime.driver.expected_version.trim().is_empty()
    {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: "runtime_version_empty".into(),
            message: "Les versions attendues du navigateur et du driver doivent être épinglées"
                .into(),
        });
    }

    if runtime.engine == BrowserEngineKind::ChromeForTesting
        && runtime.browser.expected_version != runtime.driver.expected_version
    {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: "chrome_chromedriver_version_parity".into(),
            message: format!(
                "Chrome for Testing ({}) et chromedriver ({}) doivent appartenir au même build",
                runtime.browser.expected_version, runtime.driver.expected_version
            ),
        });
    }

    let browser_actual = compute_file_sha256(&runtime.browser.path, "browser_binary")?;
    if browser_actual != runtime.browser.expected_sha256 {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: "browser_binary_sha256".into(),
            message: format!(
                "Empreinte SHA-256 divergente pour le navigateur : attendu {}, observé {}",
                runtime.browser.expected_sha256, browser_actual
            ),
        });
    }

    let driver_actual = compute_file_sha256(&runtime.driver.path, "driver_binary")?;
    if driver_actual != runtime.driver.expected_sha256 {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: "driver_binary_sha256".into(),
            message: format!(
                "Empreinte SHA-256 divergente pour le driver : attendu {}, observé {}",
                runtime.driver.expected_sha256, driver_actual
            ),
        });
    }

    if let Some(observed_version) = read_adjacent_chrome_version(&runtime.browser.path)
        && observed_version != runtime.browser.expected_version
    {
        return Err(BrowserError::RuntimeMismatch {
            details_ref: "browser_bundle_version".into(),
            message: format!(
                "Version observée du bundle ({observed_version}) différente de la version épinglée ({})",
                runtime.browser.expected_version
            ),
        });
    }

    Ok(RuntimeInspection {
        engine: runtime.engine,
        platform: runtime.platform,
        verified_version: runtime.browser.expected_version.clone(),
        browser_sha256: browser_actual,
        driver_sha256: driver_actual,
        supported_locators: vec![
            "role_name".into(),
            "label".into(),
            "test_id".into(),
            "css".into(),
        ],
        passive_inspection_only: true,
    })
}

/// Inventorie en lecture seule les binaires navigateur/driver disponibles localement.
///
/// Ne télécharge rien et n'exécute aucun binaire.
#[must_use]
pub fn inventory_local_runtimes(paths: &LocalInventorySearchPaths) -> LocalRuntimeInventory {
    let mut chrome_for_testing = Vec::new();
    let mut chromedrivers = Vec::new();

    if let Some(pw_dir) = &paths.playwright_cache_dir
        && let Ok(entries) = std::fs::read_dir(pw_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(bin) = find_cft_binary_in_dir(&path) {
                let version = read_adjacent_chrome_version(&bin);
                let sha256 = compute_file_sha256(&bin, "chrome_for_testing").ok();
                chrome_for_testing.push(DetectedBrowserArtifact {
                    label: "Google Chrome for Testing".into(),
                    path: bin,
                    version,
                    sha256,
                });
            }
            if let Some(drv) = find_driver_binary_in_dir(&path, "chromedriver") {
                let sha256 = compute_file_sha256(&drv, "chromedriver").ok();
                chromedrivers.push(DetectedBrowserArtifact {
                    label: "chromedriver".into(),
                    path: drv,
                    version: None,
                    sha256,
                });
            }
        }
    }

    if let Some(runtimes_dir) = &paths.unleash_runtimes_dir
        && let Ok(entries) = std::fs::read_dir(runtimes_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(bin) = find_cft_binary_in_dir(&path) {
                let version = read_adjacent_chrome_version(&bin);
                let sha256 = compute_file_sha256(&bin, "chrome_for_testing").ok();
                chrome_for_testing.push(DetectedBrowserArtifact {
                    label: "Unleash Pinned Chrome for Testing".into(),
                    path: bin,
                    version,
                    sha256,
                });
            }
            if let Some(drv) = find_driver_binary_in_dir(&path, "chromedriver") {
                let sha256 = compute_file_sha256(&drv, "chromedriver").ok();
                chromedrivers.push(DetectedBrowserArtifact {
                    label: "Unleash Pinned chromedriver".into(),
                    path: drv,
                    version: None,
                    sha256,
                });
            }
        }
    }

    for dir in &paths.executable_search_dirs {
        let candidate = dir.join("chromedriver");
        if candidate.is_file() {
            let sha256 = compute_file_sha256(&candidate, "chromedriver").ok();
            chromedrivers.push(DetectedBrowserArtifact {
                label: "chromedriver".into(),
                path: candidate,
                version: None,
                sha256,
            });
        }
    }

    let paired_ready = !chrome_for_testing.is_empty() && !chromedrivers.is_empty();
    let status_note = if paired_ready {
        "Chrome for Testing et chromedriver détectés localement (vérifier parité version + SHA-256)"
            .into()
    } else if chrome_for_testing.is_empty() && chromedrivers.is_empty() {
        "Aucun Chrome for Testing ni chromedriver détecté dans les emplacements inspectés".into()
    } else if chromedrivers.is_empty() {
        format!(
            "{} Chrome for Testing détecté(s), mais aucun chromedriver appairé présent pour EXT-BROWSER",
            chrome_for_testing.len()
        )
    } else {
        "chromedriver détecté sans binaire Chrome for Testing associé".into()
    };

    LocalRuntimeInventory {
        chrome_for_testing,
        chromedrivers,
        paired_chrome_driver_ready: paired_ready,
        status_note,
    }
}

fn find_cft_binary_in_dir(dir: &Path) -> Option<PathBuf> {
    let mac_candidates = [
        dir.join("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        dir.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        dir.join("chrome-linux64/chrome"),
        dir.join("chrome"),
    ];
    mac_candidates.into_iter().find(|p| p.is_file())
}

fn find_driver_binary_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates = [
        dir.join(name),
        dir.join("chromedriver-mac-arm64").join(name),
        dir.join("chromedriver-linux64").join(name),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Lit la version dans `Info.plist` adjacent (macOS `.app`) ou un fichier `VERSION` voisin sans lancer de processus.
fn read_adjacent_chrome_version(binary_path: &Path) -> Option<String> {
    // Cas macOS: .../Contents/MacOS/Google Chrome for Testing -> .../Contents/Info.plist
    if let Some(contents_dir) = binary_path.parent().and_then(Path::parent) {
        let info_plist = contents_dir.join("Info.plist");
        if let Ok(xml) = std::fs::read_to_string(&info_plist)
            && let Some(ver) = extract_plist_string_value(&xml, "CFBundleShortVersionString")
        {
            return Some(ver);
        }
    }
    if let Some(parent) = binary_path.parent() {
        let version_file = parent.join("VERSION");
        if let Ok(raw) = std::fs::read_to_string(version_file) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn extract_plist_string_value(xml: &str, key_name: &str) -> Option<String> {
    let needle = format!("<key>{key_name}</key>");
    let key_idx = xml.find(&needle)?;
    let rest = xml.get(key_idx + needle.len()..)?;
    let open_tag = "<string>";
    let start_idx = rest.find(open_tag)? + open_tag.len();
    let after_open = rest.get(start_idx..)?;
    let end_idx = after_open.find("</string>")?;
    let val = after_open.get(..end_idx)?.trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_file(dir: &Path, name: &str, bytes: &[u8]) -> (PathBuf, Sha256Digest) {
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        let digest = Sha256Digest::from_bytes(bytes);
        (path, digest)
    }

    #[test]
    fn safari_rejected_in_mediated_mode() {
        let dir = tempfile::tempdir().unwrap();
        let (b_path, b_sha) = write_temp_file(dir.path(), "safari", b"safari");
        let (d_path, d_sha) = write_temp_file(dir.path(), "safaridriver", b"driver");
        let runtime = PinnedBrowserRuntime {
            engine: BrowserEngineKind::SafariUnsupported,
            platform: TargetPlatform::MacOsArm64,
            browser: PinnedBinary {
                path: b_path,
                expected_version: "18.0".into(),
                expected_sha256: b_sha,
            },
            driver: PinnedBinary {
                path: d_path,
                expected_version: "18.0".into(),
                expected_sha256: d_sha,
            },
            mapping_version: "v1".into(),
        };
        let err = verify_pinned_runtime(&runtime, ExecutionMode::Mediated).unwrap_err();
        assert!(matches!(err, BrowserError::CapabilityMissing { .. }));
    }

    #[test]
    fn runtime_pinning_rejects_missing_and_mismatched_sha_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let (b_path, b_sha) = write_temp_file(dir.path(), "chrome", b"chrome-bin");
        let (d_path, d_sha) = write_temp_file(dir.path(), "chromedriver", b"driver-bin");

        let valid = PinnedBrowserRuntime {
            engine: BrowserEngineKind::ChromeForTesting,
            platform: TargetPlatform::MacOsArm64,
            browser: PinnedBinary {
                path: b_path.clone(),
                expected_version: "156.0.8078.4".into(),
                expected_sha256: b_sha,
            },
            driver: PinnedBinary {
                path: d_path.clone(),
                expected_version: "156.0.8078.4".into(),
                expected_sha256: d_sha,
            },
            mapping_version: "v1".into(),
        };
        let ok = verify_pinned_runtime(&valid, ExecutionMode::Mediated).unwrap();
        assert!(ok.passive_inspection_only);
        assert_eq!(ok.verified_version, "156.0.8078.4");

        // SHA-256 divergent
        let mut bad_sha = valid.clone();
        bad_sha.driver.expected_sha256 = Sha256Digest::from_bytes(b"other");
        let err_sha = verify_pinned_runtime(&bad_sha, ExecutionMode::Mediated).unwrap_err();
        assert!(matches!(err_sha, BrowserError::RuntimeMismatch { .. }));

        // Version divergente entre Chrome et chromedriver
        let mut bad_ver = valid.clone();
        bad_ver.driver.expected_version = "155.0.0.0".into();
        let err_ver = verify_pinned_runtime(&bad_ver, ExecutionMode::Mediated).unwrap_err();
        assert!(matches!(err_ver, BrowserError::RuntimeMismatch { .. }));

        // Binaire manquant
        let mut missing = valid;
        missing.driver.path = dir.path().join("absent-chromedriver");
        let err_missing = verify_pinned_runtime(&missing, ExecutionMode::Mediated).unwrap_err();
        assert!(matches!(err_missing, BrowserError::RuntimeMismatch { .. }));
    }

    #[test]
    fn local_inventory_reports_missing_paired_chromedriver() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir
            .path()
            .join("chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents");
        std::fs::create_dir_all(bundle.join("MacOS")).unwrap();
        std::fs::write(
            bundle.join("Info.plist"),
            "<dict><key>CFBundleShortVersionString</key><string>156.0.8078.4</string></dict>",
        )
        .unwrap();
        std::fs::write(bundle.join("MacOS/Google Chrome for Testing"), b"cft-bin").unwrap();

        let inv = inventory_local_runtimes(&LocalInventorySearchPaths {
            playwright_cache_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        });
        assert_eq!(inv.chrome_for_testing.len(), 1);
        assert_eq!(
            inv.chrome_for_testing
                .first()
                .and_then(|c| c.version.as_deref()),
            Some("156.0.8078.4")
        );
        assert!(inv.chromedrivers.is_empty());
        assert!(!inv.paired_chrome_driver_ready);
    }
}
