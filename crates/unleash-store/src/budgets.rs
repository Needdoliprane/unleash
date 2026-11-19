//! Réservations atomiques de capacité (`capacity_pools` 5/50) et de budgets (`budget_accounts`/`budget_reservations`).

use serde::{Deserialize, Serialize};
use unleash_domain::{
    AccountId, ActionId, AttemptId, AuthorityGeneration, DaemonEpoch, IdGeneratorPort,
    InvocationId, MicrosAmount, PoolId, ProviderProbeId, ReplayWorkerId, ReservationId,
    Sha256Digest, TimestampMs,
};

/// Mode de réserve pour la classe `control` sur le pool `execution` (Q-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlReserveMode {
    /// Réserve souple : prêtée au travail tant qu'aucun contrôle admissible n'attend.
    Soft,
    /// Réserve dure : unités strictement gardées pour la classe `control` (exige `C >= 2`).
    Hard,
}

impl ControlReserveMode {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Hard => "hard",
        }
    }
}

/// Configuration du pool global d'exécution (`execution`, 1..=50, défaut 5, Q-15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPoolParams {
    /// Identifiant du pool.
    pub pool_id: PoolId,
    /// Limite demandée (1..=50, défaut 5).
    pub requested_limit: i64,
    /// Mode de réserve contrôle (`soft`/`hard`).
    pub control_reserve_mode: ControlReserveMode,
    /// Unités réservées pour le contrôle (`1..=max(1, requested_limit / 2)`).
    pub control_reserved_units: i64,
    /// Plafond de rafale contrôle (>= 1, défaut 2).
    pub control_burst_limit: i64,
    /// Délai d'escalade checkpoint en secondes (10..=600, défaut 60).
    pub checkpoint_escalation_s: i64,
}

impl Default for ExecutionPoolParams {
    fn default() -> Self {
        Self {
            pool_id: PoolId::from_u128(1),
            requested_limit: 5,
            control_reserve_mode: ControlReserveMode::Soft,
            control_reserved_units: 1,
            control_burst_limit: 2,
            checkpoint_escalation_s: 60,
        }
    }
}

/// Propriétaire exclusif typé d'une réservation d'exécution (`execution_reservation_owners`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOwner {
    /// Invocation d'agent logique et tentative associée.
    AgentInvocation {
        /// Invocation propriétaire.
        invocation_id: InvocationId,
        /// Tentative active éventuelle.
        attempt_id: Option<AttemptId>,
    },
    /// Worker de replay mécanique.
    ReplayWorker {
        /// Identifiant du worker.
        replay_worker_id: ReplayWorkerId,
    },
    /// Sonde fournisseur.
    ProviderProbe {
        /// Identifiant de la sonde.
        provider_probe_id: ProviderProbeId,
    },
}

/// Demande de réservation atomique de capacité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityReservationRequest {
    /// Identifiant attribué ou fourni de réservation.
    pub reservation_id: ReservationId,
    /// Pool cible.
    pub pool_id: PoolId,
    /// Propriétaire typé (si pool `execution`).
    pub execution_owner: Option<ExecutionOwner>,
    /// Type de propriétaire SQL (`agent_invocation`, `replay_worker`, `provider_probe`, etc.).
    pub owner_type: String,
    /// Identifiant texte du propriétaire.
    pub owner_id: String,
    /// Classe d'admission (`control` / `work`, obligatoire pour `execution`).
    pub admission_class: Option<unleash_domain::AdmissionClass>,
    /// Nombre d'unités (> 0).
    pub amount: i64,
}

/// État observé d'un pool de capacité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityPoolState {
    /// Identifiant du pool.
    pub pool_id: PoolId,
    /// Limite demandée.
    pub requested_limit: i64,
    /// Limite effective actuellement en vigueur.
    pub effective_limit: i64,
    /// Plafond dur.
    pub hard_limit: i64,
    /// Admissions gelées (`true` pendant une baisse en cours de drainage).
    pub admissions_frozen: bool,
    /// Condition dérivée `draining_to_limit` (`requested_limit < effective_limit && admissions_frozen`).
    pub draining_to_limit: bool,
    /// Occupation totale courante (`held` + `reconciling`).
    pub current_occupation: i64,
    /// Révision du pool.
    pub revision: i64,
}

/// Portée d'un compte budgétaire (`global`, `project`, `provider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScopeType {
    /// Plafond global utilisateur.
    Global,
    /// Plafond par projet.
    Project,
    /// Plafond par fournisseur.
    Provider,
}

impl BudgetScopeType {
    /// Représentation SQL canonique.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Provider => "provider",
        }
    }
}

/// Statut de tarification observé avant réservation budgétaire (Q-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingCheckStatus {
    /// Tarif valide et non périmé.
    Valid,
    /// Tarif périmé mais autorisé uniquement si `pricing_stale_policy == Warn`.
    Stale {
        /// Politique applicable (`block` ou `warn`).
        stale_policy_warn: bool,
    },
    /// Aucun tarif disponible pour le modèle distant.
    Missing,
}

/// Demande de réservation sur un compte budgétaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAccountCharge {
    /// Compte cible (`global`, `project` ou `provider`).
    pub account_id: AccountId,
    /// Montant maximal estimé à réserver en micro-unités.
    pub max_micros: MicrosAmount,
}

/// Demande atomique multi-comptes pour une action (`REQ-PERSIST-008`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReservationRequest {
    /// Identifiant de l'action financée.
    pub action_id: ActionId,
    /// Modèle ou cible tarifée.
    pub model_id: String,
    /// Version de catalogue tarifaire épinglée.
    pub pricing_version: String,
    /// Statut du tarif (Q-14).
    pub pricing_status: PricingCheckStatus,
    /// Hash canonique de la requête bornée.
    pub max_request_hash: Sha256Digest,
    /// Liste de comptes devant tous réussir ensemble (ou aucun).
    pub charges: Vec<BudgetAccountCharge>,
}

/// État d'un compte budgétaire lu en base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAccountSnapshot {
    /// Identifiant du compte.
    pub account_id: AccountId,
    /// Portée (`global`/`project`/`provider`).
    pub scope_type: BudgetScopeType,
    /// Clé de portée.
    pub scope_id: String,
    /// Devise explicite.
    pub currency: String,
    /// Plafond en micro-unités.
    pub limit_micros: MicrosAmount,
    /// Montant dépensé confirmé.
    pub spent_micros: MicrosAmount,
    /// Montant actuellement réservé (inclut `held` et `unknown`).
    pub reserved_micros: MicrosAmount,
}

/// Enregistrement de réservation budgétaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReservationRecord {
    /// Identifiant de réservation.
    pub reservation_id: ReservationId,
    /// Compte associé.
    pub account_id: AccountId,
    /// Action associée.
    pub action_id: ActionId,
    /// Montant réservé.
    pub reserved_micros: MicrosAmount,
    /// Montant réglé effectif si `settled`.
    pub settled_micros: Option<MicrosAmount>,
    /// État (`held`, `unknown`, `settled`, `released`).
    pub state: String,
    /// Lignée et horodatage éventuel.
    pub authority_generation: Option<AuthorityGeneration>,
    /// Horodatage de lecture.
    pub observed_at_ms: Option<TimestampMs>,
}

use rusqlite::{Connection, OptionalExtension, Transaction};
use unleash_domain::AdmissionClass;

use crate::error::StoreError;

pub(crate) fn tx_global_execution_pool_id(
    connection: &Connection,
) -> Result<Option<PoolId>, StoreError> {
    let id: Option<String> = connection
        .query_row(
            "SELECT pool_id FROM capacity_pools
             WHERE pool_kind='execution' AND scope_key='global' AND unit='slot'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|value| value.parse().map_err(StoreError::from))
        .transpose()
}

/// Initialise le pool global dans la transaction qui rend le daemon prêt.
///
/// Un pool déjà présent conserve son identifiant, sa configuration et sa révision.
///
/// # Errors
/// Retourne une erreur si le pool ne peut pas être lu ou créé.
pub fn ensure_global_execution_pool_tx(
    tx: &Transaction<'_>,
    id_generator: &dyn IdGeneratorPort,
) -> Result<(), StoreError> {
    if tx_global_execution_pool_id(tx)?.is_some() {
        return Ok(());
    }
    let mut params = ExecutionPoolParams::default();
    let id_in_use: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM capacity_pools WHERE pool_id=?1)",
        [params.pool_id.to_string()],
        |row| row.get(0),
    )?;
    if id_in_use {
        params.pool_id = PoolId::from_uuid(id_generator.new_uuid());
    }
    tx_configure_execution_pool(tx, &params)?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "Validation stricte de cohérence du pool d'exécution (soft/hard) et calcul de drainage atomique"
)]
pub(crate) fn tx_configure_execution_pool(
    tx: &Transaction<'_>,
    params: &ExecutionPoolParams,
) -> Result<CapacityPoolState, StoreError> {
    if !(1..=50).contains(&params.requested_limit) {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "requested_limit {} hors [1..=50]",
            params.requested_limit
        )));
    }
    if params.control_burst_limit < 1 {
        return Err(StoreError::ControlReserveIncoherent(
            "control_burst_limit doit être >= 1".to_owned(),
        ));
    }
    if !(10..=600).contains(&params.checkpoint_escalation_s) {
        return Err(StoreError::ControlReserveIncoherent(
            "checkpoint_escalation_s hors [10..=600]".to_owned(),
        ));
    }
    let max_reserve = (params.requested_limit / 2).max(1);
    if params.control_reserved_units < 1 || params.control_reserved_units > max_reserve {
        return Err(StoreError::ControlReserveIncoherent(format!(
            "control_reserved_units={} hors [1..={max_reserve}] pour C={}",
            params.control_reserved_units, params.requested_limit
        )));
    }
    if params.control_reserve_mode == ControlReserveMode::Hard && params.requested_limit < 2 {
        return Err(StoreError::ControlReserveIncoherent(
            "control_reserve_mode=hard exige requested_limit >= 2".to_owned(),
        ));
    }

    let pool_str = params.pool_id.to_string();
    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT effective_limit, revision FROM capacity_pools WHERE pool_id = ?1
             AND pool_kind='execution' AND scope_key='global' AND unit='slot'",
            [&pool_str],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let occupation: i64 = tx
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM capacity_reservations
             WHERE pool_id = ?1 AND state IN ('held', 'reconciling')",
            [&pool_str],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    let (effective_limit, admissions_frozen, new_rev) = match existing {
        None => (params.requested_limit, 0_i64, 1_i64),
        Some((cur_eff, rev)) => {
            if params.requested_limit < occupation {
                // Baisse sous occupation : garder l'ancienne effective_limit et geler les admissions
                (cur_eff, 1_i64, rev.saturating_add(1))
            } else {
                (params.requested_limit, 0_i64, rev.saturating_add(1))
            }
        }
    };

    if existing.is_none() {
        tx.execute(
            "INSERT INTO capacity_pools
             (pool_id, pool_kind, scope_key, unit, requested_limit, effective_limit, admissions_frozen, hard_limit,
              control_reserve_mode, control_reserved_units, control_burst_limit, checkpoint_escalation_s, revision)
             VALUES (?1, 'execution', 'global', 'slot', ?2, ?3, ?4, 50, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                pool_str,
                params.requested_limit,
                effective_limit,
                admissions_frozen,
                params.control_reserve_mode.as_str(),
                params.control_reserved_units,
                params.control_burst_limit,
                params.checkpoint_escalation_s,
                new_rev
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    } else {
        tx.execute(
            "UPDATE capacity_pools
             SET requested_limit = ?1, effective_limit = ?2, admissions_frozen = ?3,
                 control_reserve_mode = ?4, control_reserved_units = ?5,
                 control_burst_limit = ?6, checkpoint_escalation_s = ?7, revision = ?8
             WHERE pool_id = ?9",
            rusqlite::params![
                params.requested_limit,
                effective_limit,
                admissions_frozen,
                params.control_reserve_mode.as_str(),
                params.control_reserved_units,
                params.control_burst_limit,
                params.checkpoint_escalation_s,
                new_rev,
                pool_str
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    Ok(CapacityPoolState {
        pool_id: params.pool_id,
        requested_limit: params.requested_limit,
        effective_limit,
        hard_limit: 50,
        admissions_frozen: admissions_frozen != 0,
        draining_to_limit: params.requested_limit < effective_limit && admissions_frozen != 0,
        current_occupation: occupation,
        revision: new_rev,
    })
}

pub(crate) fn tx_read_execution_pool(
    tx: &Connection,
    pool_id: PoolId,
) -> Result<CapacityPoolState, StoreError> {
    let pool_str = pool_id.to_string();
    let row: Option<(i64, i64, i64, i64, i64)> = tx
        .query_row(
            "SELECT requested_limit, effective_limit, hard_limit, admissions_frozen, revision
             FROM capacity_pools WHERE pool_id = ?1",
            [&pool_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((req_lim, eff_lim, hard_lim, frozen, rev)) = row else {
        return Err(StoreError::NotFound(format!(
            "pool introuvable : {pool_id}"
        )));
    };
    let occ: i64 = tx
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM capacity_reservations
             WHERE pool_id = ?1 AND state IN ('held', 'reconciling')",
            [&pool_str],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;
    Ok(CapacityPoolState {
        pool_id,
        requested_limit: req_lim,
        effective_limit: eff_lim,
        hard_limit: hard_lim,
        admissions_frozen: frozen != 0,
        draining_to_limit: req_lim < eff_lim && frozen != 0,
        current_occupation: occ,
        revision: rev,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "Réservation atomique capacité + contrôle réserve/anti-famine + propriétaire XOR"
)]
pub(crate) fn tx_reserve_capacity(
    tx: &Transaction<'_>,
    authority: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    now_ms: TimestampMs,
    req: &CapacityReservationRequest,
) -> Result<(), StoreError> {
    if req.amount <= 0 {
        return Err(StoreError::InvalidArgument {
            reason: None,
            message: "capacity amount doit être > 0".to_owned(),
        });
    }
    let pool_str = req.pool_id.to_string();
    let pool_row: Option<(String, i64, i64, i64, Option<String>, Option<i64>)> = tx
        .query_row(
            "SELECT pool_kind, effective_limit, hard_limit, admissions_frozen, control_reserve_mode, control_reserved_units
             FROM capacity_pools WHERE pool_id = ?1",
            [&pool_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    let Some((pool_kind, eff_lim, hard_lim, frozen, ctrl_mode, ctrl_units)) = pool_row else {
        return Err(StoreError::NotFound(format!(
            "pool de capacité introuvable : {}",
            req.pool_id
        )));
    };

    if frozen != 0 {
        return Err(StoreError::CapacityBlocked(
            "admissions gelées (draining_to_limit)".to_owned(),
        ));
    }

    let occ: i64 = tx
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM capacity_reservations
             WHERE pool_id = ?1 AND state IN ('held', 'reconciling')",
            [&pool_str],
            |r| r.get(0),
        )
        .map_err(StoreError::from_rusqlite)?;

    if occ.saturating_add(req.amount) > eff_lim || eff_lim > hard_lim {
        return Err(StoreError::CapacityBlocked(format!(
            "pool saturé ({occ} + {} > {eff_lim})",
            req.amount
        )));
    }

    if pool_kind == "execution" {
        let Some(adm_class) = req.admission_class else {
            return Err(StoreError::InvariantViolation(
                "admission_class obligatoire pour le pool execution".to_owned(),
            ));
        };
        if ctrl_mode.as_deref() == Some("hard") && adm_class == AdmissionClass::Work {
            let work_occ: i64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(amount), 0) FROM capacity_reservations
                     WHERE pool_id = ?1 AND state IN ('held', 'reconciling') AND admission_class = 'work'",
                    [&pool_str],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)?;
            let reserved = ctrl_units.unwrap_or(1);
            if work_occ.saturating_add(req.amount) > eff_lim.saturating_sub(reserved) {
                return Err(StoreError::CapacityBlocked(
                    "réserve dure contrôle protégée (mode hard)".to_owned(),
                ));
            }
        }
    }

    let adm_str = req.admission_class.map(|c| c.to_string());
    let epoch_i64 = i64::try_from(daemon_epoch.as_u64()).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO capacity_reservations
         (reservation_id, pool_id, owner_type, owner_id, admission_class, amount, state,
          authority_generation, daemon_epoch, created_at_ms, released_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'held', ?7, ?8, ?9, NULL)",
        rusqlite::params![
            req.reservation_id.to_string(),
            pool_str,
            req.owner_type,
            req.owner_id,
            adm_str,
            req.amount,
            authority.to_string(),
            epoch_i64,
            now_ms.as_i64()
        ],
    )
    .map_err(StoreError::from_rusqlite)?;

    if let Some(ref owner) = req.execution_owner {
        let (inv_id, att_id, rw_id, probe_id) = match owner {
            ExecutionOwner::AgentInvocation {
                invocation_id,
                attempt_id,
            } => (
                Some(invocation_id.to_string()),
                attempt_id.map(|a| a.to_string()),
                None,
                None,
            ),
            ExecutionOwner::ReplayWorker { replay_worker_id } => {
                (None, None, Some(replay_worker_id.to_string()), None)
            }
            ExecutionOwner::ProviderProbe { provider_probe_id } => {
                (None, None, None, Some(provider_probe_id.to_string()))
            }
        };
        tx.execute(
            "INSERT INTO execution_reservation_owners
             (reservation_id, invocation_id, attempt_id, replay_worker_id, provider_probe_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                req.reservation_id.to_string(),
                inv_id,
                att_id,
                rw_id,
                probe_id
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    Ok(())
}

pub(crate) fn tx_release_capacity_reservation(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    reservation_id: ReservationId,
) -> Result<(), StoreError> {
    let execution: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM capacity_reservations c JOIN capacity_pools p ON p.pool_id=c.pool_id WHERE c.reservation_id=?1 AND p.pool_kind='execution')", [reservation_id.to_string()], |row| row.get(0))?;
    if execution {
        // Historical queued reservations with no physical attempt/effect can be
        // released. An absent or uncertain owner is never evidence of a stop.
        let never_started: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_reservation_owners o JOIN invocations i ON i.invocation_id=o.invocation_id JOIN capacity_reservations c ON c.reservation_id=o.reservation_id WHERE o.reservation_id=?1 AND o.attempt_id IS NULL AND i.state='queued' AND c.owner_type='agent_invocation' AND c.owner_id=i.invocation_id AND NOT EXISTS(SELECT 1 FROM invocation_attempts t WHERE t.invocation_id=i.invocation_id) AND NOT EXISTS(SELECT 1 FROM actions a WHERE a.invocation_id=i.invocation_id))", [reservation_id.to_string()], |row| row.get(0))?;
        if !never_started {
            return Err(StoreError::InvalidArgument {
                reason: None,
                message:
                    "une exécution active exige une observation de checkpoint et neutralisation"
                        .to_owned(),
            });
        }
    }
    tx_release_capacity_reservation_verified(tx, now_ms, reservation_id)
}

// Only the checkpoint transaction or the no-start path above may call this.
pub(crate) fn tx_release_capacity_reservation_verified(
    tx: &Transaction<'_>,
    now_ms: TimestampMs,
    reservation_id: ReservationId,
) -> Result<(), StoreError> {
    let pool_id: Option<String> = tx
        .query_row(
            "SELECT pool_id FROM capacity_reservations
             WHERE reservation_id = ?1 AND state IN ('held', 'reconciling')",
            [reservation_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some(pool_str) = pool_id else {
        return Err(StoreError::NotFound(format!(
            "réservation active introuvable : {reservation_id}"
        )));
    };

    tx.execute(
        "UPDATE capacity_reservations SET state = 'released', released_at_ms = ?1
         WHERE reservation_id = ?2",
        rusqlite::params![now_ms.as_i64(), reservation_id.to_string()],
    )
    .map_err(StoreError::from_rusqlite)?;

    let (req_lim, eff_lim, frozen, rev): (i64, i64, i64, i64) = tx
        .query_row(
            "SELECT requested_limit, effective_limit, admissions_frozen, revision
             FROM capacity_pools WHERE pool_id = ?1",
            [&pool_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(StoreError::from_rusqlite)?;

    if frozen != 0 || req_lim < eff_lim {
        let occ: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM capacity_reservations
                 WHERE pool_id = ?1 AND state IN ('held', 'reconciling')",
                [&pool_str],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)?;
        if occ <= req_lim {
            tx.execute(
                "UPDATE capacity_pools
                 SET effective_limit = ?1, admissions_frozen = 0, revision = ?2
                 WHERE pool_id = ?3",
                rusqlite::params![req_lim, rev.saturating_add(1), pool_str],
            )
            .map_err(StoreError::from_rusqlite)?;
        }
    }

    Ok(())
}

pub(crate) fn tx_upsert_budget_account(
    tx: &Transaction<'_>,
    account_id: AccountId,
    scope_type: BudgetScopeType,
    scope_id: &str,
    currency: &str,
    limit_micros: MicrosAmount,
) -> Result<BudgetAccountSnapshot, StoreError> {
    let acc_str = account_id.to_string();
    let lim_i64 = i64::try_from(limit_micros.as_u64()).unwrap_or(i64::MAX);
    let existing: Option<(i64, i64, i64)> = tx
        .query_row(
            "SELECT spent_micros, reserved_micros, revision FROM budget_accounts WHERE account_id = ?1",
            [&acc_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;

    match existing {
        None => {
            tx.execute(
                "INSERT INTO budget_accounts
                 (account_id, scope_type, scope_id, currency, limit_micros, spent_micros, reserved_micros, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0)",
                rusqlite::params![acc_str, scope_type.as_str(), scope_id, currency, lim_i64],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(BudgetAccountSnapshot {
                account_id,
                scope_type,
                scope_id: scope_id.to_owned(),
                currency: currency.to_owned(),
                limit_micros,
                spent_micros: MicrosAmount::ZERO,
                reserved_micros: MicrosAmount::ZERO,
            })
        }
        Some((spent, reserved, rev)) => {
            tx.execute(
                "UPDATE budget_accounts SET limit_micros = ?1, revision = ?2 WHERE account_id = ?3",
                rusqlite::params![lim_i64, rev.saturating_add(1), acc_str],
            )
            .map_err(StoreError::from_rusqlite)?;
            Ok(BudgetAccountSnapshot {
                account_id,
                scope_type,
                scope_id: scope_id.to_owned(),
                currency: currency.to_owned(),
                limit_micros,
                spent_micros: MicrosAmount::new(u64::try_from(spent).unwrap_or(0)),
                reserved_micros: MicrosAmount::new(u64::try_from(reserved).unwrap_or(0)),
            })
        }
    }
}

pub(crate) fn tx_read_budget_account(
    tx: &Connection,
    account_id: AccountId,
) -> Result<BudgetAccountSnapshot, StoreError> {
    let row: Option<(String, String, String, i64, i64, i64)> = tx
        .query_row(
            "SELECT scope_type, scope_id, currency, limit_micros, spent_micros, reserved_micros
             FROM budget_accounts WHERE account_id = ?1",
            [account_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(StoreError::from_rusqlite)?;
    let Some((st, sid, cur, lim, spent, res)) = row else {
        return Err(StoreError::NotFound(format!(
            "compte budget introuvable : {account_id}"
        )));
    };
    let scope_type = match st.as_str() {
        "global" => BudgetScopeType::Global,
        "project" => BudgetScopeType::Project,
        _ => BudgetScopeType::Provider,
    };
    Ok(BudgetAccountSnapshot {
        account_id,
        scope_type,
        scope_id: sid,
        currency: cur,
        limit_micros: MicrosAmount::new(u64::try_from(lim).unwrap_or(0)),
        spent_micros: MicrosAmount::new(u64::try_from(spent).unwrap_or(0)),
        reserved_micros: MicrosAmount::new(u64::try_from(res).unwrap_or(0)),
    })
}

pub(crate) fn tx_reserve_budgets(
    tx: &Transaction<'_>,
    ids: &dyn unleash_domain::IdGeneratorPort,
    req: &BudgetReservationRequest,
) -> Result<(), StoreError> {
    match req.pricing_status {
        PricingCheckStatus::Valid => {}
        PricingCheckStatus::Missing => {
            return Err(StoreError::PricingUnknown {
                model_id: req.model_id.clone(),
            });
        }
        PricingCheckStatus::Stale { stale_policy_warn } => {
            if !stale_policy_warn {
                return Err(StoreError::PricingStale {
                    pricing_version: req.pricing_version.clone(),
                });
            }
        }
    }

    // Phase 1 : vérifier TOUS les comptes avant toute écriture (tout ou rien)
    let mut validated: Vec<(String, i64, i64, i64)> = Vec::with_capacity(req.charges.len());
    for charge in &req.charges {
        let acc_str = charge.account_id.to_string();
        let row: Option<(i64, i64, i64, i64)> = tx
            .query_row(
                "SELECT limit_micros, spent_micros, reserved_micros, revision
                 FROM budget_accounts WHERE account_id = ?1",
                [&acc_str],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(StoreError::from_rusqlite)?;
        let Some((limit, spent, reserved, rev)) = row else {
            return Err(StoreError::NotFound(format!(
                "compte budget introuvable : {acc_str}"
            )));
        };
        let req_i64 = i64::try_from(charge.max_micros.as_u64()).unwrap_or(i64::MAX);
        let projected = spent.saturating_add(reserved).saturating_add(req_i64);
        if projected > limit {
            return Err(StoreError::BudgetExhausted {
                account_id: acc_str,
            });
        }
        validated.push((acc_str, reserved, req_i64, rev));
    }

    // Phase 2 : appliquer toutes les réservations atomiquement
    for (acc_str, cur_reserved, req_i64, rev) in validated {
        let new_reserved = cur_reserved.saturating_add(req_i64);
        tx.execute(
            "UPDATE budget_accounts SET reserved_micros = ?1, revision = ?2 WHERE account_id = ?3",
            rusqlite::params![new_reserved, rev.saturating_add(1), acc_str],
        )
        .map_err(StoreError::from_rusqlite)?;

        let res_id = ReservationId::from_uuid(ids.new_uuid());
        tx.execute(
            "INSERT INTO budget_reservations
             (reservation_id, account_id, action_id, reserved_micros, settled_micros, state, pricing_version, max_request_hash)
             VALUES (?1, ?2, ?3, ?4, NULL, 'held', ?5, ?6)",
            rusqlite::params![
                res_id.to_string(),
                acc_str,
                req.action_id.to_string(),
                req_i64,
                req.pricing_version,
                req.max_request_hash.as_prefixed_str()
            ],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    Ok(())
}

pub(crate) fn tx_mark_action_budget_unknown(
    tx: &Transaction<'_>,
    action_id: ActionId,
) -> Result<(), StoreError> {
    let updated = tx
        .execute(
            "UPDATE budget_reservations SET state = 'unknown'
             WHERE action_id = ?1 AND state = 'held'",
            [action_id.to_string()],
        )
        .map_err(StoreError::from_rusqlite)?;
    if updated == 0 {
        return Err(StoreError::NotFound(format!(
            "aucune réservation held pour action {action_id}"
        )));
    }
    Ok(())
}

pub(crate) fn tx_settle_action_budget(
    tx: &Transaction<'_>,
    action_id: ActionId,
    actual_micros: MicrosAmount,
) -> Result<(), StoreError> {
    let act_str = action_id.to_string();
    let rows: Vec<(String, String, i64)> = {
        let mut stmt = tx
            .prepare(
                "SELECT reservation_id, account_id, reserved_micros
                 FROM budget_reservations
                 WHERE action_id = ?1 AND state IN ('held', 'unknown')",
            )
            .map_err(StoreError::from_rusqlite)?;
        let mapped = stmt
            .query_map([&act_str], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(StoreError::from_rusqlite)?;
        let mut v = Vec::new();
        for item in mapped {
            v.push(item.map_err(StoreError::from_rusqlite)?);
        }
        v
    };

    if rows.is_empty() {
        return Err(StoreError::NotFound(format!(
            "aucune réservation ouverte à régler pour action {action_id}"
        )));
    }

    let actual_i64 = i64::try_from(actual_micros.as_u64()).unwrap_or(i64::MAX);
    for (res_id, acc_id, reserved) in rows {
        tx.execute(
            "UPDATE budget_reservations SET state = 'settled', settled_micros = ?1 WHERE reservation_id = ?2",
            rusqlite::params![actual_i64, res_id],
        )
        .map_err(StoreError::from_rusqlite)?;

        tx.execute(
            "UPDATE budget_accounts
             SET spent_micros = spent_micros + ?1,
                 reserved_micros = MAX(0, reserved_micros - ?2),
                 revision = revision + 1
             WHERE account_id = ?3",
            rusqlite::params![actual_i64, reserved, acc_id],
        )
        .map_err(StoreError::from_rusqlite)?;
    }

    Ok(())
}

pub(crate) fn tx_release_action_budget(
    tx: &Transaction<'_>,
    action_id: ActionId,
) -> Result<(), StoreError> {
    let act_str = action_id.to_string();
    let rows: Vec<(String, String, i64)> = {
        let mut stmt = tx
            .prepare(
                "SELECT reservation_id, account_id, reserved_micros
                 FROM budget_reservations
                 WHERE action_id = ?1 AND state = 'held'",
            )
            .map_err(StoreError::from_rusqlite)?;
        let mapped = stmt
            .query_map([&act_str], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(StoreError::from_rusqlite)?;
        let mut v = Vec::new();
        for item in mapped {
            v.push(item.map_err(StoreError::from_rusqlite)?);
        }
        v
    };
    if rows.is_empty() {
        return Err(StoreError::NotFound(format!(
            "aucune réservation held à libérer pour action {action_id}"
        )));
    }
    for (res_id, acc_id, reserved) in rows {
        tx.execute(
            "UPDATE budget_reservations SET state = 'released' WHERE reservation_id = ?1",
            [res_id],
        )
        .map_err(StoreError::from_rusqlite)?;
        tx.execute(
            "UPDATE budget_accounts
             SET reserved_micros = MAX(0, reserved_micros - ?1),
                 revision = revision + 1
             WHERE account_id = ?2",
            rusqlite::params![reserved, acc_id],
        )
        .map_err(StoreError::from_rusqlite)?;
    }
    Ok(())
}
