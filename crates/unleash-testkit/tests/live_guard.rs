//! Tests d'intégration de [`LiveTestGuard`] (isolation `.env`, allowlist fermée, budget et masquage).
// T-F001-008 / REQ-CORE-008

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use secrecy::ExposeSecret;
use unleash_testkit::{
    LIVE_MODEL_ALLOWLIST, LiveGuardError, LiveModel, LiveTestGuard, local_test_dir,
};

fn env_fn<'a>(vars: &'a BTreeMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
    move |key| vars.get(key).map(|v| (*v).to_owned())
}

fn write_temp_dotenv(contents: &str) -> Result<(tempfile::TempDir, PathBuf), Box<dyn Error>> {
    let dir = local_test_dir("live-guard")?;
    let path = dir.path().join(".env");
    fs::write(&path, contents)?;
    Ok((dir, path))
}

#[test]
fn test_live_guard_accepts_haiku_and_luna_under_budget_and_redacts_debug()
-> Result<(), Box<dyn Error>> {
    let anthropic_dummy = "sk-ant-synthetic-secret-must-never-appear-in-debug";
    let openai_dummy = "sk-openai-synthetic-secret-must-never-appear-in-debug";
    let dotenv_body = format!(
        "UNLEASH_DEV_ANTHROPIC_API_KEY={anthropic_dummy}\n\
         UNLEASH_DEV_OPENAI_API_KEY={openai_dummy}\n\
         UNLEASH_LIVE_TEST_MODEL_ALLOWLIST=claude-haiku-5-5,gpt-6-luna\n\
         UNLEASH_LIVE_TEST_ANTHROPIC_MODEL=claude-haiku-5-5\n\
         UNLEASH_LIVE_TEST_OPENAI_MODEL=gpt-6-luna\n\
         UNLEASH_LIVE_TEST_BUDGET_USD=1.50\n"
    );
    let (_dir, dotenv_path) = write_temp_dotenv(&dotenv_body)?;

    let mut proc_env = BTreeMap::new();
    proc_env.insert("UNLEASH_LIVE_TESTS", "1");

    let guard = LiveTestGuard::from_env_and_dotenv_with(env_fn(&proc_env), Some(&dotenv_path))?;
    assert_eq!(LIVE_MODEL_ALLOWLIST, ["claude-haiku-5-5", "gpt-6-luna"]);
    assert_eq!(guard.budget_limit_micros(), 1_500_000);
    assert_eq!(guard.spent_micros(), 0);
    assert_eq!(guard.remaining_micros(), 1_500_000);
    assert!(guard.has_anthropic_key());
    assert!(guard.has_openai_key());

    let p1 = guard.authorize_call_usd("claude-haiku-5-5", 0.50)?;
    assert_eq!(p1.model(), LiveModel::ClaudeHaiku55);
    assert_eq!(p1.reserved_micros(), 500_000);
    assert_eq!(p1.api_key().expose_secret(), anthropic_dummy);

    let p2 = guard.authorize_call_usd("gpt-6-luna", 0.75)?;
    assert_eq!(p2.model(), LiveModel::Gpt6Luna);
    assert_eq!(p2.reserved_micros(), 750_000);
    assert_eq!(p2.api_key().expose_secret(), openai_dummy);

    assert_eq!(guard.spent_micros(), 1_250_000);
    assert_eq!(guard.remaining_micros(), 250_000);

    // Vérification stricte du masquage Debug (aucun secret exposé).
    let dbg_compact = format!("{guard:?}");
    let dbg_pretty = format!("{guard:#?}");
    let permit_dbg = format!("{p1:?}");
    for rendered in [&dbg_compact, &dbg_pretty, &permit_dbg] {
        assert!(!rendered.contains(anthropic_dummy));
        assert!(!rendered.contains(openai_dummy));
        assert!(rendered.contains("REDACTED"));
    }

    // Dépassement de budget refusé sans modifier le solde restant.
    let err = guard.authorize_call_usd("claude-haiku-5-5", 0.30);
    assert_eq!(
        err.err(),
        Some(LiveGuardError::BudgetExceeded {
            requested_micros: 300_000,
            remaining_micros: 250_000,
            limit_micros: 1_500_000,
        })
    );
    assert_eq!(guard.spent_micros(), 1_250_000);
    Ok(())
}

#[test]
fn test_live_guard_rejects_disallowed_intelligent_models() -> Result<(), Box<dyn Error>> {
    let (_dir, dotenv_path) = write_temp_dotenv(
        "UNLEASH_DEV_ANTHROPIC_API_KEY=sk-ant-dummy\n\
         UNLEASH_DEV_OPENAI_API_KEY=sk-openai-dummy\n\
         UNLEASH_LIVE_TEST_BUDGET_USD=5\n",
    )?;
    let mut proc_env = BTreeMap::new();
    proc_env.insert("UNLEASH_LIVE_TESTS", "1");

    let guard = LiveTestGuard::from_env_and_dotenv_with(env_fn(&proc_env), Some(&dotenv_path))?;

    for forbidden in [
        "gpt-6.1-sol",
        "claude-opus-5-5",
        "claude-fable-5-2",
        "claude-sonnet-5-5",
        "gpt-6-astra",
        "claude-haiku-5-5 ",
        "",
    ] {
        let res = guard.authorize_call_usd(forbidden, 0.01);
        assert!(
            matches!(res, Err(LiveGuardError::ModelNotAllowed { .. })),
            "le modèle '{forbidden}' devait être rejeté"
        );
    }

    assert_eq!(guard.spent_micros(), 0);
    Ok(())
}

#[test]
fn test_live_guard_rejects_disallowed_model_in_allowlist_override() -> Result<(), Box<dyn Error>> {
    let (_dir, dotenv_path) = write_temp_dotenv(
        "UNLEASH_DEV_OPENAI_API_KEY=sk-openai-dummy\n\
         UNLEASH_LIVE_TEST_MODEL_ALLOWLIST=claude-haiku-5-5,gpt-6.1-sol\n\
         UNLEASH_LIVE_TEST_BUDGET_USD=5\n",
    )?;
    let mut proc_env = BTreeMap::new();
    proc_env.insert("UNLEASH_LIVE_TESTS", "1");

    let res = LiveTestGuard::from_env_and_dotenv_with(env_fn(&proc_env), Some(&dotenv_path));
    assert_eq!(
        res.unwrap_err(),
        LiveGuardError::ModelNotAllowed {
            model: String::from("gpt-6.1-sol"),
        }
    );
    Ok(())
}

#[test]
fn test_live_guard_does_not_read_dotenv_when_live_tests_disabled() -> Result<(), Box<dyn Error>> {
    // Fichier .env volontairement invalide : si dotenvy l'ouvrait, il retournerait LiveGuardError::Dotenv.
    let (_dir, broken_dotenv) = write_temp_dotenv("THIS IS INVALID DOTENV SYNTAX === \"\"\"\n")?;

    let empty_env: BTreeMap<&str, &str> = BTreeMap::new();
    let res_missing =
        LiveTestGuard::from_env_and_dotenv_with(env_fn(&empty_env), Some(&broken_dotenv));
    assert_eq!(res_missing.unwrap_err(), LiveGuardError::LiveTestsDisabled);

    let mut zero_env = BTreeMap::new();
    zero_env.insert("UNLEASH_LIVE_TESTS", "0");
    let res_zero = LiveTestGuard::from_env_and_dotenv_with(env_fn(&zero_env), Some(&broken_dotenv));
    assert_eq!(res_zero.unwrap_err(), LiveGuardError::LiveTestsDisabled);
    Ok(())
}

#[test]
fn test_live_guard_rejects_invalid_budget_and_invalid_cost() -> Result<(), Box<dyn Error>> {
    let (_dir, dotenv_bad_budget) = write_temp_dotenv(
        "UNLEASH_DEV_ANTHROPIC_API_KEY=sk-ant-dummy\n\
         UNLEASH_LIVE_TEST_BUDGET_USD=-2.0\n",
    )?;
    let mut proc_env = BTreeMap::new();
    proc_env.insert("UNLEASH_LIVE_TESTS", "1");

    let bad = LiveTestGuard::from_env_and_dotenv_with(env_fn(&proc_env), Some(&dotenv_bad_budget));
    assert!(matches!(bad, Err(LiveGuardError::InvalidBudget { .. })));

    let (_dir2, dotenv_ok) = write_temp_dotenv(
        "UNLEASH_DEV_ANTHROPIC_API_KEY=sk-ant-dummy\n\
         UNLEASH_LIVE_TEST_BUDGET_USD=1\n",
    )?;
    let guard = LiveTestGuard::from_env_and_dotenv_with(env_fn(&proc_env), Some(&dotenv_ok))?;
    assert!(matches!(
        guard.authorize_call_usd("claude-haiku-5-5", f64::NAN),
        Err(LiveGuardError::InvalidCost { .. })
    ));
    assert!(matches!(
        guard.authorize_call_usd("claude-haiku-5-5", -0.01),
        Err(LiveGuardError::InvalidCost { .. })
    ));
    Ok(())
}
