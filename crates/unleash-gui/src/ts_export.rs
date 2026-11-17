// SPDX-License-Identifier: Apache-2.0
//! Générateur TypeScript (`ts-rs`) pour les DTOs propres à la façade `unleash-gui`.
//!
//! Ces types complètent les 26 définitions canoniques d'`unleash-protocol` (situées dans
//! `apps/gui-renderer/src/generated/`) sans les dupliquer ni les modifier : toute référence à
//! `EventCursor`, `EventEnvelope`, `ErrorCode`, `ReasonCode` ou `RetryClass` importe directement
//! depuis `../generated/`.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use ts_rs::{Config, TS};

use crate::commands::{GuiCommandInput, GuiMarkdownInput, GuiQueryInput, GuiSubscribeInput};
use crate::errors::{GuiError, GuiErrorPayload};
use crate::markdown::{MarkdownBlockNode, MarkdownDocumentAst, MarkdownInlineNode};
use crate::state::GuiStreamNotification;

const PROTOCOL_TYPES: &[&str] = &[
    "CommandEnvelope",
    "CommandReceipt",
    "CommandReceiptInspection",
    "ErrorCode",
    "EventCausation",
    "EventCausationKind",
    "EventCursor",
    "EventEnvelope",
    "HelloRequest",
    "HelloResponse",
    "IpcError",
    "LineageStatus",
    "OperationKind",
    "OperationName",
    "ProtocolVersion",
    "QueryEnvelope",
    "QueryResponse",
    "ReasonCode",
    "ReceiptState",
    "RetryClass",
    "Sensitivity",
];

fn insert_gui_ts_type<T: TS + 'static>(
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
        .map(|dep| {
            if PROTOCOL_TYPES.contains(&dep.as_str()) {
                format!("import type {{ {dep} }} from \"../generated/{dep}\";")
            } else {
                format!("import type {{ {dep} }} from \"./{dep}\";")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let body = if imports.is_empty() {
        export_decl
    } else {
        format!("{imports}\n\n{export_decl}")
    };

    let content = format!(
        "// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).\n// Ne pas modifier manuellement.\n\n{body}\n"
    );
    files.insert(format!("{name}.ts"), content);
}

/// Génère l'ensemble des fichiers TypeScript de la façade `unleash-gui`.
#[must_use]
pub fn generate_gui_ts_files() -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let config = Config::default();

    insert_gui_ts_type::<MarkdownInlineNode>(&mut files, &config, "MarkdownInlineNode");
    insert_gui_ts_type::<MarkdownBlockNode>(&mut files, &config, "MarkdownBlockNode");
    insert_gui_ts_type::<MarkdownDocumentAst>(&mut files, &config, "MarkdownDocumentAst");
    insert_gui_ts_type::<GuiQueryInput>(&mut files, &config, "GuiQueryInput");
    insert_gui_ts_type::<GuiCommandInput>(&mut files, &config, "GuiCommandInput");
    insert_gui_ts_type::<GuiSubscribeInput>(&mut files, &config, "GuiSubscribeInput");
    insert_gui_ts_type::<GuiMarkdownInput>(&mut files, &config, "GuiMarkdownInput");
    insert_gui_ts_type::<GuiStreamNotification>(&mut files, &config, "GuiStreamNotification");
    insert_gui_ts_type::<GuiErrorPayload>(&mut files, &config, "GuiErrorPayload");

    let mut index_content = String::from(
        "// Ce fichier a été généré automatiquement par unleash-gui (ts-rs).\n// Ne pas modifier manuellement.\n\n",
    );
    for filename in files.keys() {
        let base = filename.trim_end_matches(".ts");
        let _ = writeln!(index_content, "export * from \"./{base}\";");
    }
    files.insert(String::from("index.ts"), index_content);

    files
}

/// Écrit tous les fichiers TypeScript de la façade GUI dans `dir`.
///
/// # Errors
/// Retourne [`GuiError::Tauri`] si une opération disque échoue.
pub fn write_gui_ts_bindings(dir: &Path) -> Result<(), GuiError> {
    fs::create_dir_all(dir).map_err(|e| GuiError::Tauri(e.to_string()))?;
    let files = generate_gui_ts_files();
    for (filename, content) in files {
        fs::write(dir.join(filename), content).map_err(|e| GuiError::Tauri(e.to_string()))?;
    }
    Ok(())
}
