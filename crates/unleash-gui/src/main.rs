// SPDX-License-Identifier: Apache-2.0
//! Point d'entrée binaire de l'hôte desktop OVPT Unleash (`unleash-gui`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;
use unleash_client::ClientConfig;
use unleash_platform::{SOCKET_FILENAME, canonical_rendezvous_dir};
use unleash_protocol::ClientKind;

fn default_client_config() -> Option<ClientConfig> {
    let dir: PathBuf = canonical_rendezvous_dir().ok()?;
    let sock = dir.join(SOCKET_FILENAME);
    Some(ClientConfig::new(
        sock,
        ClientKind::Gui,
        env!("CARGO_PKG_VERSION"),
    ))
}

fn main() -> Result<(), unleash_gui::GuiError> {
    unleash_gui::run(default_client_config())
}
