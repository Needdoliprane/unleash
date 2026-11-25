//! REQ-TERM-003 : le secret de contrôle d'un hôte de processus ne dérive pas du générateur d'identifiants.
#![allow(clippy::unwrap_used)]

use crate::terminal_service::fresh_control_secret;

#[test]
fn control_secret_does_not_follow_the_identifier_generator() {
    // Le générateur d'identifiants du démon donne une séquence reproductible : un secret qui en
    // dérive serait calculable par quiconque connaît la graine ou l'heure de démarrage.
    let first = fresh_control_secret().unwrap().expose_secret().to_owned();
    let second = fresh_control_secret().unwrap().expose_secret().to_owned();
    assert_ne!(
        first, second,
        "deux secrets de contrôle successifs doivent différer"
    );
    assert_eq!(first.len(), 64, "256 bits encodés en hexadécimal");
    assert!(
        !first.starts_with("terminal-control:"),
        "le secret de contrôle ne doit pas dériver du générateur d'identifiants"
    );
    assert!(
        first.chars().all(|c| c.is_ascii_hexdigit()),
        "le secret de contrôle est la forme hexadécimale de 32 octets aléatoires"
    );
}
