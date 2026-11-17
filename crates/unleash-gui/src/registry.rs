// SPDX-License-Identifier: Apache-2.0
//! Registre contrôlé des opérations exposées au renderer GUI (`ADR-03` amendement R7).
//!
//! Seules les 200 opérations du catalogue possédant au moins un groupe `PAR-…` dans
//! `unleash_protocol::OperationName` sont exposées. `Hello`, `RecordClaim`, `RecordObservation`
//! et `RecordJudgment` sont explicitement exclues.

use unleash_protocol::{OperationKind, OperationName};

use crate::errors::GuiError;

/// Opérations explicitement interdites au renderer GUI (réservées au handshake ou au worker/harness).
pub const FORBIDDEN_GUI_OPERATIONS: &[OperationName] = &[
    OperationName::Hello,
    OperationName::RecordClaim,
    OperationName::RecordJudgment,
    OperationName::RecordObservation,
];

/// Retourne l'ensemble ordonné des 200 opérations autorisées pour le renderer GUI.
#[must_use]
pub fn exposed_gui_operations() -> Vec<OperationName> {
    OperationName::all_operations()
        .iter()
        .copied()
        .filter(OperationName::is_exposed_to_gui)
        .collect()
}

/// Valide qu'un nom d'opération est connu et autorisé côté GUI (`is_exposed_to_gui()`).
///
/// # Errors
/// Retourne [`GuiError::ForbiddenOperation`] si l'opération est inconnue ou exclue de l'exposition GUI.
pub fn validate_exposed_operation(name: &str) -> Result<OperationName, GuiError> {
    let op: OperationName = name.parse().map_err(|_| GuiError::ForbiddenOperation {
        operation: name.to_owned(),
    })?;
    if !op.is_exposed_to_gui() || FORBIDDEN_GUI_OPERATIONS.contains(&op) {
        return Err(GuiError::ForbiddenOperation {
            operation: op.as_str().to_owned(),
        });
    }
    Ok(op)
}

/// Convertit un [`OperationName`] en identifiant canonique de commande Tauri (`op_<snake_case>`).
#[must_use]
pub fn tauri_command_name(op: OperationName) -> String {
    let raw = op.as_str();
    let mut out = String::with_capacity(raw.len() + 8);
    out.push_str("op_");
    let chars: Vec<char> = raw.chars().collect();
    for (idx, ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            let prev_is_lower_or_digit = idx
                .checked_sub(1)
                .and_then(|p| chars.get(p))
                .is_some_and(|c| c.is_lowercase() || c.is_ascii_digit());
            let next_is_lower = chars.get(idx + 1).is_some_and(|c| c.is_lowercase());
            let prev_is_upper = idx
                .checked_sub(1)
                .and_then(|p| chars.get(p))
                .is_some_and(|c| c.is_uppercase());
            if idx > 0 && (prev_is_lower_or_digit || (prev_is_upper && next_is_lower)) {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(*ch);
        }
    }
    out
}

/// Vérifie que le genre de l'opération correspond à la voie appelée (`Query` vs `Command`).
///
/// # Errors
/// Retourne [`GuiError::InvalidInput`] en cas de décalage entre le verbe et le type d'opération.
pub fn ensure_operation_kind(op: OperationName, expected: OperationKind) -> Result<(), GuiError> {
    if op.kind() != expected {
        return Err(GuiError::InvalidInput {
            detail: format!(
                "L'opération {} est de type {:?}, pas {:?}",
                op.as_str(),
                op.kind(),
                expected
            ),
        });
    }
    Ok(())
}
