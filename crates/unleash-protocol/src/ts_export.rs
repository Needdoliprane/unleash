//! # Générateur d'export TypeScript pour l'interface utilisateur
//!
//! Responsabilité exclusive : Générer les définitions TypeScript (`.ts`) de tous les DTOs
//! exposés au renderer GUI (Tauri/React) à partir des attributs `#[derive(TS)]`.

#![allow(clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use ts_rs::{Config, TS};

use crate::envelopes::{
    ClientKind, ClientUploadSegment, CommandEnvelope, CommandReceipt, CommandReceiptInspection,
    EventCausation, EventCausationKind, EventCursor, EventEnvelope, HelloRequest, HelloResponse,
    LineageStatus, ProtocolVersion, QueryEnvelope, QueryResponse, ReceiptState, Sensitivity,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use crate::errors::{ErrorCode, IpcError, ReasonCode, RetryClass};
use crate::operations::{OperationKind, OperationName};

/// Génère l'ensemble des fichiers TypeScript avec leur nom et contenu formaté.
#[must_use]
pub fn generate_all_ts_files() -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let config = Config::default();

    macro_rules! add_type {
        ($t:ident) => {
            let decl = $t::decl(&config);
            let trimmed = decl.trim();
            let export_decl = if trimmed.starts_with("export ") {
                trimmed.to_string()
            } else {
                format!("export {trimmed}")
            };
            let content = format!(
                "// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).\n// Ne pas modifier manuellement.\n\n{export_decl}\n"
            );
            files.insert(format!("{}.ts", stringify!($t)), content);
        };
    }

    add_type!(OperationKind);
    add_type!(OperationName);
    add_type!(ProtocolVersion);
    add_type!(CommandEnvelope);
    add_type!(CommandReceipt);
    add_type!(CommandReceiptInspection);
    add_type!(ReceiptState);
    add_type!(LineageStatus);
    add_type!(EventCursor);
    add_type!(EventEnvelope);
    add_type!(EventCausation);
    add_type!(EventCausationKind);
    add_type!(Sensitivity);
    add_type!(QueryEnvelope);
    add_type!(QueryResponse);
    add_type!(HelloRequest);
    add_type!(HelloResponse);
    add_type!(ClientKind);
    add_type!(SubscribeEventsRequest);
    add_type!(SubscribeEventsFrame);
    add_type!(ClientUploadSegment);
    add_type!(ErrorCode);
    add_type!(RetryClass);
    add_type!(ReasonCode);
    add_type!(IpcError);

    // Génération de l'index.ts réexportant l'ensemble
    let mut index_content = String::from(
        "// Ce fichier a été généré automatiquement par unleash-protocol.\n// Ne pas modifier manuellement.\n\n",
    );
    for filename in files.keys() {
        let base = filename.trim_end_matches(".ts");
        let _ = writeln!(index_content, "export * from \"./{base}\";");
    }
    files.insert("index.ts".to_string(), index_content);

    files
}

/// Écrit tous les fichiers TypeScript générés dans le répertoire cible.
///
/// # Errors
/// Renvoie [`std::io::Error`] si la création du dossier ou l'écriture d'un fichier échoue.
pub fn write_ts_bindings(target_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target_dir)?;
    let files = generate_all_ts_files();
    for (filename, content) in files {
        fs::write(target_dir.join(filename), content)?;
    }
    Ok(())
}
