//! REQ-MODEL-015 : le secret de contrôle d'un service fournisseur managé ne dérive pas du générateur d'identifiants.
#![allow(clippy::unwrap_used)]

use crate::provider_service_evaluation::fresh_provider_service_secret;
use unleash_testkit::DeterministicIdGenerator;

#[test]
fn provider_service_secret_does_not_follow_the_identifier_generator() {
    // Deux générateurs de même graine produisent la même séquence d'identifiants : un secret
    // qui en dérive serait reproductible par quiconque connaît la graine ou l'heure de démarrage.
    let first = fresh_provider_service_secret(&DeterministicIdGenerator::new(11))
        .unwrap()
        .expose_secret()
        .to_owned();
    let second = fresh_provider_service_secret(&DeterministicIdGenerator::new(11))
        .unwrap()
        .expose_secret()
        .to_owned();
    assert_ne!(
        first, second,
        "le secret de contrôle du service fournisseur ne doit pas dériver du générateur d'identifiants"
    );
}
