// SPDX-License-Identifier: Apache-2.0
//! # unleash-gui
//!
//! Responsabilité exclusive : Hôte desktop Tauri 2 d'OVPT Unleash (`ADR-03`, `F014`).
//!
//! ## Frontières architecturales strictes
//! - `#![forbid(unsafe_code)]`.
//! - Consomme exclusivement `unleash_client::UnleashClient` (zéro `rusqlite`, zéro règle métier,
//!   zéro machine d'état parallèle).
//! - Expose exactement les 200 opérations du catalogue rattachées à au moins un groupe `PAR-…`,
//!   en excluant explicitement `Hello`, `RecordClaim`, `RecordObservation` et `RecordJudgment`.
//! - Restreint les capacités Tauri à la seule fenêtre `main`, avec CSP de production stricte et
//!   sans plugin `shell`, `fs` ou `http`.
//! - Parse tout document Markdown côté Rust en AST borné avant exposition au renderer React.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod commands;
pub mod errors;
pub mod markdown;
pub mod registry;
pub mod state;

pub use commands::{
    GuiCommandInput, GuiMarkdownInput, GuiQueryInput, GuiSubscribeInput, MAX_GUI_PAYLOAD_BYTES,
    validate_webview_payload,
};
pub use errors::{GuiError, GuiErrorPayload};
pub use markdown::{
    MAX_AST_DEPTH, MAX_AST_NODES, MAX_MARKDOWN_INPUT_BYTES, MarkdownBlockNode, MarkdownDocumentAst,
    MarkdownInlineNode, parse_markdown_ast, sanitize_external_url,
};
pub use registry::{
    FORBIDDEN_GUI_OPERATIONS, ensure_operation_kind, exposed_gui_operations, tauri_command_name,
    validate_exposed_operation,
};
pub use state::{GuiAppState, GuiStreamNotification};

use unleash_client::ClientConfig;

/// Attache l'état [`GuiAppState`] et le routeur des commandes autorisées à un [`tauri::Builder`].
///
/// Cette fonction est générique sur [`tauri::Runtime`] afin d'être réutilisée à l'identique par
/// le binaire de production (`Wry`) et par la suite de tests headless (`MockRuntime`).
#[must_use]
pub fn configure_builder<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    state: GuiAppState,
) -> tauri::Builder<R> {
    let builder = builder.manage(state);
    commands::install_invoke_handler(builder)
}

/// Démarre l'application desktop Tauri 2 sur le runtime natif (`Wry`).
///
/// La fermeture de la fenêtre ou de l'hôte libère uniquement l'état client local et n'émet aucune
/// commande d'arrêt vers le daemon ni les jobs en cours (`REQ-UI-013`).
///
/// # Errors
/// Retourne [`GuiError::Tauri`] si la construction ou la boucle événementielle Tauri échoue.
pub fn run(initial_client_config: Option<ClientConfig>) -> Result<(), GuiError> {
    let state = GuiAppState::new(initial_client_config);
    let app = configure_builder(tauri::Builder::default(), state)
        .build(tauri::generate_context!())
        .map_err(|e| GuiError::Tauri(e.to_string()))?;
    app.run(|_app_handle, _event| {});
    Ok(())
}
