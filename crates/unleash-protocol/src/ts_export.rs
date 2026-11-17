//! # Générateur d'export TypeScript pour l'interface utilisateur
//!
//! Responsabilité exclusive : Générer les définitions TypeScript (`.ts`) de tous les DTOs
//! exposés au renderer GUI (Tauri/React) à partir des attributs `#[derive(TS)]`.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};
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

fn insert_ts_type<T: TS + 'static>(
    files: &mut BTreeMap<String, String>,
    config: &Config,
    name: &str,
) {
    let decl = T::decl(config);
    let trimmed = decl.trim();
    let export_decl = if trimmed.starts_with("export ") {
        trimmed.to_owned()
    } else {
        format!("export {trimmed}")
    };
    let deps: BTreeSet<String> = T::dependencies(config)
        .into_iter()
        .filter(|dep| dep.type_id != TypeId::of::<T>() && dep.ts_name != name)
        .map(|dep| dep.ts_name)
        .collect();
    let imports = deps
        .into_iter()
        .map(|dep| format!("import type {{ {dep} }} from \"./{dep}\";"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = if imports.is_empty() {
        export_decl
    } else {
        format!("{imports}\n\n{export_decl}")
    };
    let content = format!(
        "// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).\n// Ne pas modifier manuellement.\n\n{body}\n"
    );
    files.insert(format!("{name}.ts"), content);
}

/// Génère l'ensemble des fichiers TypeScript avec leur nom et contenu formaté.
#[must_use]
pub fn generate_all_ts_files() -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let config = Config::default();

    insert_ts_type::<OperationKind>(&mut files, &config, "OperationKind");
    insert_ts_type::<OperationName>(&mut files, &config, "OperationName");
    insert_ts_type::<ProtocolVersion>(&mut files, &config, "ProtocolVersion");
    insert_ts_type::<CommandEnvelope>(&mut files, &config, "CommandEnvelope");
    insert_ts_type::<CommandReceipt>(&mut files, &config, "CommandReceipt");
    insert_ts_type::<CommandReceiptInspection>(&mut files, &config, "CommandReceiptInspection");
    insert_ts_type::<ReceiptState>(&mut files, &config, "ReceiptState");
    insert_ts_type::<LineageStatus>(&mut files, &config, "LineageStatus");
    insert_ts_type::<EventCursor>(&mut files, &config, "EventCursor");
    insert_ts_type::<EventEnvelope>(&mut files, &config, "EventEnvelope");
    insert_ts_type::<EventCausation>(&mut files, &config, "EventCausation");
    insert_ts_type::<EventCausationKind>(&mut files, &config, "EventCausationKind");
    insert_ts_type::<Sensitivity>(&mut files, &config, "Sensitivity");
    insert_ts_type::<QueryEnvelope>(&mut files, &config, "QueryEnvelope");
    insert_ts_type::<QueryResponse>(&mut files, &config, "QueryResponse");
    insert_ts_type::<HelloRequest>(&mut files, &config, "HelloRequest");
    insert_ts_type::<HelloResponse>(&mut files, &config, "HelloResponse");
    insert_ts_type::<ClientKind>(&mut files, &config, "ClientKind");
    insert_ts_type::<SubscribeEventsRequest>(&mut files, &config, "SubscribeEventsRequest");
    insert_ts_type::<SubscribeEventsFrame>(&mut files, &config, "SubscribeEventsFrame");
    insert_ts_type::<ClientUploadSegment>(&mut files, &config, "ClientUploadSegment");
    insert_ts_type::<ErrorCode>(&mut files, &config, "ErrorCode");
    insert_ts_type::<RetryClass>(&mut files, &config, "RetryClass");
    insert_ts_type::<ReasonCode>(&mut files, &config, "ReasonCode");
    insert_ts_type::<IpcError>(&mut files, &config, "IpcError");

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
