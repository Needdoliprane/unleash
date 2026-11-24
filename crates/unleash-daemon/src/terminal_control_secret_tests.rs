//! REQ-TERM-003 : le secret de contrôle d'un hôte de processus ne dérive pas du générateur d'identifiants.
#![allow(clippy::unwrap_used)]

use crate::terminal_service::fresh_control_secret;
use unleash_testkit::DeterministicIdGenerator;

#[test]
fn control_secret_does_not_follow_the_identifier_generator() {
    // Deux générateurs de même graine produisent la même séquence d'identifiants : un secret
    // qui en dérive serait reproductible par quiconque connaît la graine ou l'heure de démarrage.
    let first = fresh_control_secret(&DeterministicIdGenerator::new(7))
        .unwrap()
        .expose_secret()
        .to_owned();
    let second = fresh_control_secret(&DeterministicIdGenerator::new(7))
        .unwrap()
        .expose_secret()
        .to_owned();
    assert_ne!(
        first, second,
        "le secret de contrôle ne doit pas dériver du générateur d'identifiants"
    );
}
