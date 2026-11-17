//! Registre local des JSON Schemas 2020-12 (sans résolution réseau).
//!
//! Charge les cinq schémas normatifs de `.claude/schemas/` sous leurs URN canoniques
//! (`urn:ovpt:unleash:*:1`) dans `referencing::Registry` et compile des validateurs `jsonschema`
//! avec vérification des formats activée (`should_validate_formats(true)`).

use std::fs;
use std::path::Path;

use jsonschema::Validator;
use serde::Serialize;
use serde_json::Value;

use crate::errors::SpikeError;

/// Identifiants des schémas locaux chargés par le spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemaKind {
    /// `discovery-receipt.schema.json` (`urn:ovpt:unleash:discovery-receipt:1`).
    DiscoveryReceipt,
    /// `expected-test-execution.schema.json` (`urn:ovpt:unleash:expected-test-execution:1`).
    ExpectedTestExecution,
    /// `expected-execution-binding.schema.json` (`urn:ovpt:unleash:expected-execution-binding:1`).
    ExpectedExecutionBinding,
    /// `test-execution-observation.schema.json` (`urn:ovpt:unleash:test-execution-observation:1`).
    TestExecutionObservation,
    /// `contract-claim.schema.json` (`urn:ovpt:unleash:contract-claim:1`).
    ContractClaim,
}

impl SchemaKind {
    /// Nom du fichier dans `.claude/schemas/`.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::DiscoveryReceipt => "discovery-receipt.schema.json",
            Self::ExpectedTestExecution => "expected-test-execution.schema.json",
            Self::ExpectedExecutionBinding => "expected-execution-binding.schema.json",
            Self::TestExecutionObservation => "test-execution-observation.schema.json",
            Self::ContractClaim => "contract-claim.schema.json",
        }
    }

    /// URN canonique déclarée par `$id` dans le schéma.
    #[must_use]
    pub const fn urn(self) -> &'static str {
        match self {
            Self::DiscoveryReceipt => "urn:ovpt:unleash:discovery-receipt:1",
            Self::ExpectedTestExecution => "urn:ovpt:unleash:expected-test-execution:1",
            Self::ExpectedExecutionBinding => "urn:ovpt:unleash:expected-execution-binding:1",
            Self::TestExecutionObservation => "urn:ovpt:unleash:test-execution-observation:1",
            Self::ContractClaim => "urn:ovpt:unleash:contract-claim:1",
        }
    }
}

/// Collection compilée des validateurs JSON Schema 2020-12 locaux.
pub struct SchemaRegistry {
    discovery_receipt: Validator,
    expected_test_execution: Validator,
    expected_execution_binding: Validator,
    test_execution_observation: Validator,
    contract_claim: Validator,
}

impl std::fmt::Debug for SchemaRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaRegistry")
            .field("schemas_loaded", &5usize)
            .finish_non_exhaustive()
    }
}

impl SchemaRegistry {
    /// Charge les schémas depuis `<repo_root>/.claude/schemas/` dans un registre local hors ligne.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::Io`], [`SpikeError::Json`] ou [`SpikeError::SchemaBuild`] si un
    /// schéma est introuvable, invalide ou impossible à compiler.
    pub fn load_from_repo_root(repo_root: &Path) -> Result<Self, SpikeError> {
        let schemas_dir = repo_root.join(".claude/schemas");
        let discovery_val = load_schema_value(&schemas_dir, SchemaKind::DiscoveryReceipt)?;
        let expected_val = load_schema_value(&schemas_dir, SchemaKind::ExpectedTestExecution)?;
        let binding_val = load_schema_value(&schemas_dir, SchemaKind::ExpectedExecutionBinding)?;
        let observation_val =
            load_schema_value(&schemas_dir, SchemaKind::TestExecutionObservation)?;
        let claim_val = load_schema_value(&schemas_dir, SchemaKind::ContractClaim)?;

        let registry = referencing::Registry::new()
            .add(SchemaKind::DiscoveryReceipt.urn(), discovery_val.clone())
            .map_err(|e| SpikeError::SchemaBuild {
                schema: SchemaKind::DiscoveryReceipt.urn().to_owned(),
                reason: e.to_string(),
            })?
            .add(
                SchemaKind::ExpectedTestExecution.urn(),
                expected_val.clone(),
            )
            .map_err(|e| SpikeError::SchemaBuild {
                schema: SchemaKind::ExpectedTestExecution.urn().to_owned(),
                reason: e.to_string(),
            })?
            .add(
                SchemaKind::ExpectedExecutionBinding.urn(),
                binding_val.clone(),
            )
            .map_err(|e| SpikeError::SchemaBuild {
                schema: SchemaKind::ExpectedExecutionBinding.urn().to_owned(),
                reason: e.to_string(),
            })?
            .add(
                SchemaKind::TestExecutionObservation.urn(),
                observation_val.clone(),
            )
            .map_err(|e| SpikeError::SchemaBuild {
                schema: SchemaKind::TestExecutionObservation.urn().to_owned(),
                reason: e.to_string(),
            })?
            .add(SchemaKind::ContractClaim.urn(), claim_val.clone())
            .map_err(|e| SpikeError::SchemaBuild {
                schema: SchemaKind::ContractClaim.urn().to_owned(),
                reason: e.to_string(),
            })?
            .prepare()
            .map_err(|e| SpikeError::SchemaBuild {
                schema: "local-urn-registry".to_owned(),
                reason: e.to_string(),
            })?;

        Ok(Self {
            discovery_receipt: compile_validator(
                SchemaKind::DiscoveryReceipt,
                &discovery_val,
                &registry,
            )?,
            expected_test_execution: compile_validator(
                SchemaKind::ExpectedTestExecution,
                &expected_val,
                &registry,
            )?,
            expected_execution_binding: compile_validator(
                SchemaKind::ExpectedExecutionBinding,
                &binding_val,
                &registry,
            )?,
            test_execution_observation: compile_validator(
                SchemaKind::TestExecutionObservation,
                &observation_val,
                &registry,
            )?,
            contract_claim: compile_validator(SchemaKind::ContractClaim, &claim_val, &registry)?,
        })
    }

    /// Charge le registre depuis la racine du dépôt déduite de `CARGO_MANIFEST_DIR` (`../..`).
    ///
    /// # Errors
    /// Voir [`Self::load_from_repo_root`].
    pub fn load_default() -> Result<Self, SpikeError> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self::load_from_repo_root(&root)
    }

    /// Valide une valeur JSON brute contre le schéma désigné.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::SchemaValidation`] si la valeur ne respecte pas le schéma.
    pub fn validate_value(&self, kind: SchemaKind, instance: &Value) -> Result<(), SpikeError> {
        let validator = match kind {
            SchemaKind::DiscoveryReceipt => &self.discovery_receipt,
            SchemaKind::ExpectedTestExecution => &self.expected_test_execution,
            SchemaKind::ExpectedExecutionBinding => &self.expected_execution_binding,
            SchemaKind::TestExecutionObservation => &self.test_execution_observation,
            SchemaKind::ContractClaim => &self.contract_claim,
        };

        let errors: Vec<String> = validator
            .iter_errors(instance)
            .map(|err| format!("{} ({})", err, err.instance_path()))
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(SpikeError::SchemaValidation {
                schema: kind.file_name().to_owned(),
                details: errors.join("; "),
            })
        }
    }

    /// Sérialise un DTO en `serde_json::Value` puis le valide contre le schéma désigné.
    ///
    /// # Errors
    /// Renvoie [`SpikeError::Json`] ou [`SpikeError::SchemaValidation`].
    pub fn validate_serializable<T: Serialize>(
        &self,
        kind: SchemaKind,
        item: &T,
    ) -> Result<Value, SpikeError> {
        let val = serde_json::to_value(item)?;
        self.validate_value(kind, &val)?;
        Ok(val)
    }
}

fn load_schema_value(schemas_dir: &Path, kind: SchemaKind) -> Result<Value, SpikeError> {
    let path = schemas_dir.join(kind.file_name());
    let raw = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn compile_validator(
    kind: SchemaKind,
    schema: &Value,
    registry: &referencing::Registry<'_>,
) -> Result<Validator, SpikeError> {
    jsonschema::options()
        .should_validate_formats(true)
        .with_registry(registry)
        .build(schema)
        .map_err(|e| SpikeError::SchemaBuild {
            schema: kind.file_name().to_owned(),
            reason: e.to_string(),
        })
}
