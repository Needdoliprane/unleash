//! REQ-MODEL-015 : le secret de contrôle d'un service fournisseur managé ne dérive pas du générateur d'identifiants.
#![allow(clippy::unwrap_used)]

use crate::provider_service_evaluation::fresh_provider_service_secret;

#[test]
fn provider_service_secret_does_not_follow_the_identifier_generator() {
    // Le générateur d'identifiants du démon donne une séquence reproductible : un secret qui en
    // dérive serait calculable par quiconque connaît la graine ou l'heure de démarrage.
    let first = fresh_provider_service_secret().unwrap().expose_secret().to_owned();
    let second = fresh_provider_service_secret()
        .unwrap()
        .expose_secret()
        .to_owned();
    assert_ne!(
        first, second,
        "deux secrets de contrôle successifs doivent différer"
    );
    assert_eq!(first.len(), 64, "256 bits encodés en hexadécimal");
    assert!(
        !first.starts_with("provider-service:"),
        "le secret de contrôle ne doit pas dériver du générateur d'identifiants"
    );
}
