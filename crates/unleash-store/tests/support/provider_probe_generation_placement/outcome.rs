//! T-F012-015 / AC-F012-015 / REQ-MODEL-015 : types publics d'une génération managée et d'un
//! lancement de service, dans `SQLite` réel. Aucun réseau, aucun processus fils.

use super::*;

// T-F012-015 / AC-F012-015 / REQ-MODEL-015 — tout type public porte `Debug`.
#[test]
fn t_f012_015_managed_launch_and_generation_types_implement_debug() {
    fn assert_debug<T: std::fmt::Debug>() {}
    assert_debug::<unleash_store::AttestedProviderServiceLaunch>();
    assert_debug::<unleash_store::ManagedGenerationDispatchPermit>();
    assert_debug::<unleash_store::ManagedOllamaTransport>();
    assert_debug::<unleash_store::ProviderServiceLaunchPermit>();
}
