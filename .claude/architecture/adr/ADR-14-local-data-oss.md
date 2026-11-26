# ADR-14 — Données locales, open source, plateformes et distribution

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : licence et marque (Q-06), matrice de plateformes et accessibilité (Q-11), distribution signée et mises à jour (Q-16), emplacements et rétention précisés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Le dossier `.claude/` est la référence documentaire et héberge depuis R7 l’outillage optionnel de développement ; il n’est jamais le stockage runtime du produit, dont les secrets et états privés ne doivent pas suivre le dépôt.

Exigences : INV-046 INV-049 INV-055 REQ-CORE REQ-SEC. Risques/hypothèses/questions : R-15 R-21 R-25 ; H-15 ; Q-06 Q-08 Q-11 Q-12 Q-16.

## Options et compromis

Option retenue : **Config/data/cache/runtime distincts, documentation versionnée ; licence open source Apache-2.0 (Q-06, R7)**.

Alternatives considérées : Tout placer dans .claude ; configuration cachée dans un service cloud.

Séparer permet sauvegarde/purge et évite couplage Claude Code ; cloud obligatoire change produit ; tout au dépôt fuit l’opérationnel.

## Décision

Paths OS explicites, profils utilisateur, worktrees sous configuration séparée, artefacts content-addressed. Publication OVPT Unleash sous Apache-2.0 avec notices, SBOM, guide de migration et politique de sécurité ; signature, artefacts et mises à jour selon l’amendement R7.

## Amendement R7

Cet amendement tranche Q-06, Q-11 et Q-16 ([révision R7](../../brief/revision-2026-11-16-r7.md)). Ce qui reste relève des preuves EXT-OS, EXT-OSS et EXT-DIST ou d’actes d’OVPT, pas d’un arbitrage.

### Emplacements, rétention et profil de distribution

1. La séparation du [cycle de vie](../runtime-lifecycle.md) est adoptée : configuration, données et SQLite, artefacts et preuves, sauvegardes, runtimes navigateur, worktrees, logs, cache, IPC et secrets. Aucun état runtime, secret, worktree ni journal d’exécution dans `.claude/` ou dans le dépôt ; le produit ne lit jamais `.claude/` à l’exécution et ne dépend pas de Claude Code.
2. Rétention : politique `unleash.retention.default.v1` de Q-12, canonique dans le [modèle de stockage](../storage-model.md).
3. Les artefacts officiels signés embarquent un profil de distribution non secret (identité de distribution, clients OAuth publics de Q-08, canal de mise à jour). Les builds depuis les sources n’en ont pas et fonctionnent avec des clients `bring_your_own` ; une version modifiée ne réutilise pas ce profil.

### Licence et marque (Q-06)

L'[ADR-20](ADR-20-contribution-signoff.md) remplace la règle DCO du point 4.

4. Licence Apache-2.0 unique pour tout le dépôt : code, présentation, schémas, exemples et dossier. Fichiers racine `LICENSE`, `NOTICE`, `CONTRIBUTING.md` (DCO, sans CLA ; sa section « Marques » porte la politique de marque, à laquelle renvoie `NOTICE`) et `SECURITY.md` ; notices tierces (`THIRD-PARTY-LICENSES`) générées par artefact publié depuis le SBOM. En-tête SPDX dans chaque fichier source ; crates `license = "Apache-2.0"` et `publish = false` en V1.
5. Les dépendances distribuées respectent l’allow-list de `deny.toml` ; GPL, AGPL, LGPL liée statiquement, SSPL, BUSL et licences non commerciales sont refusées, toute autre licence exige une dérogation écrite. `cargo deny` bloque la CI ; chaque artefact publié fournit son SBOM CycloneDX et ses notices générées.
6. La licence ne concède aucun droit sur les noms OVPT et Unleash. Une version modifiée distribuée porte un autre nom affiché et retire le profil de distribution officiel. Une recherche d’antériorité du nom précède la première publication publique.

### Plateformes et accessibilité (Q-11)

7. Matrice qualifiée V1 : macOS 15, 26 et 27 sur Apple Silicon, minimum 15.0, daemon en agent utilisateur launchd (agent `SMAppService` déclaré dans le bundle signé, ou LaunchAgent amorcé dans le domaine `gui/<uid>` pour l’archive) ; Ubuntu 24.04 et 26.04 LTS x86_64, GUI sur la WebKitGTK de la distribution, daemon en unité `systemd --user` (survie au logout seulement avec l’option linger explicite) ou en mode foreground explicite. Aucun service root ([cycle de vie](../runtime-lifecycle.md#service-utilisateur)). Mac Intel, Windows, Linux aarch64 et autres distributions sont hors matrice qualifiée : aucune promesse.
8. Base active sur APFS ou ext4 local ; volume réseau ou synchronisé refusé. Machines de référence minimales des budgets : Mac Apple Silicon M1 8 Go sur APFS interne ; PC x86_64 4 cœurs 8 Go sur SSD ext4.
9. Accessibilité : WCAG 2.2 niveau AA comme référentiel de recette du contenu GUI, sans revendication de certification ; lecteurs d’écran VoiceOver (macOS) et Orca (Ubuntu) ; IME japonais et chinois, texte RTL. La TUI ne porte aucun état par la seule couleur. Terminaux qualifiés : Terminal.app sur macOS 15, 26 et 27 ; iTerm2 et Ghostty sur macOS 27 ; terminal par défaut d’Ubuntu 24.04 et 26.04 ; tmux ≥ 3.4 en local et via SSH ; tailles 80×24 minimum et 120×40 de référence. Environnement et limites : [conception terminal](../../design/cli-tui-design.md#matrice-terminal-qualifiée-v1).

### Distribution signée et mises à jour (Q-16)

10. macOS : certificat Developer ID Application d’OVPT, Hardened Runtime, entitlements minimaux, notarisation agrafée sur l’application et le DMG. Artefacts : DMG de l’application et archives de la CLI, de la TUI et du daemon. Ubuntu : paquet `.deb` et archive d’installation utilisateur sans root.
11. Chaque release est construite en CI depuis un tag signé et publie ses empreintes SHA-256 signées et un SBOM par artefact.
12. Les mises à jour sont signées et appliquées sur commande de l’utilisateur, jamais silencieusement, par l’installeur/updater signé de l’application (GUI) et du paquet CLI, hors protocole daemon : aucune opération IPC n’est ajoutée. Une installation `.deb` se met à jour par le gestionnaire de paquets. L’updater vérifie signature et empreinte avant tout remplacement et conserve la version précédente jusqu’au démarrage réussi de la nouvelle ; une migration de schéma est précédée d’une sauvegarde.
13. Désinstallation : une installation utilisateur (archive, application macOS) se retire sans root ; un paquet `.deb` se retire par le gestionnaire de paquets. Binaires et service utilisateur sont retirés ; données, secrets et sauvegardes sont conservés sauf purge explicite, qui énonce ses limites (sauvegardes externes, snapshots, fournisseurs).
14. Restent à fournir par OVPT avant publication : adhésion Apple Developer de l’entité, certificats et clés de signature, domaine inversé des identifiants de plateforme (bundle id, label launchd, service du coffre), fixés au gate EXT-DIST.

## Conséquences, complexité et impact V1

La licence est fixée par Q-06 (amendement R7) ; dépendances GUI/natives qualifiées avant release. Rétention et suppression ont limites sauvegardes/fournisseurs.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Inspection des artefacts publiés sans secret, backup/restore, fonctionnement sans root, installation utilisateur sans root par archive, désinstallation conservant les données et corpus Rust ; gates EXT-OS, EXT-OSS et EXT-DIST de la [readiness checklist](../../plans/readiness-checklist.md).

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../runtime-lifecycle.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

