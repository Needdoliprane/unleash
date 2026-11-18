//! Politique d'ordonnancement F003 (`REQ-AGENT-005`) sur un snapshot d'admission.
//!
//! Le crate dépend du domaine pour les identifiants et ne réalise aucune I/O.
//! L'appelant rattache chaque proposition à la révision observée. Le writer revalide
//! les préconditions et reste propriétaire des réservations et du journal.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use unleash_domain::{
    AdmissionClass, InvocationId, ProjectId, ProviderProbeId, ReplayWorkerId, ReservationId,
    TimestampMs,
};

/// Unité d'exécution du pool unique. Seule une invocation agente peut être `control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExecutionUnit {
    /// Invocation agente de classe contrôle.
    Control(InvocationId),
    /// Invocation agente de classe travail.
    AgentWork(InvocationId),
    /// Worker de replay mécanique.
    ReplayWorker(ReplayWorkerId),
    /// Sonde fournisseur.
    ProviderProbe(ProviderProbeId),
}

impl ExecutionUnit {
    /// Classe à transmettre au writer pour la réservation proposée.
    #[must_use]
    pub const fn admission_class(self) -> AdmissionClass {
        match self {
            Self::Control(_) => AdmissionClass::Control,
            Self::AgentWork(_) | Self::ReplayWorker(_) | Self::ProviderProbe(_) => {
                AdmissionClass::Work
            }
        }
    }
}

/// Prêt d'unités réservées à la classe contrôle (Q-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlReserveMode {
    /// Les travaux peuvent emprunter les unités réservées.
    Soft,
    /// Au plus `C - control_reserved_units` unités de travail peuvent être actives.
    Hard,
}

/// Paramètres effectifs du pool et de la politique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulingSettings {
    /// Limite globale effective, de 1 à 50.
    pub effective_limit: u8,
    /// Mode de réserve de contrôle.
    pub control_reserve_mode: ControlReserveMode,
    /// Unités réservées, entre 1 et `max(1, C / 2)`.
    pub control_reserved_units: u8,
    /// Admissions de contrôle consécutives permises avant de prioriser un travail.
    pub control_burst_limit: u64,
    /// Délai avant d'étendre la demande à une autre cible, de 10 à 600 secondes.
    pub checkpoint_escalation_s: u16,
}

impl Default for SchedulingSettings {
    fn default() -> Self {
        Self {
            effective_limit: 5,
            control_reserve_mode: ControlReserveMode::Soft,
            control_reserved_units: 1,
            control_burst_limit: 2,
            checkpoint_escalation_s: 60,
        }
    }
}

/// État d'un exécuteur observé par le propriétaire de la continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointEligibility {
    /// Aucune demande de checkpoint autorisée dans ce fragment.
    Unavailable,
    /// L'exécuteur peut recevoir une demande de checkpoint coopératif.
    Cooperative,
    /// L'exécuteur se trouve déjà à une frontière sûre.
    SafeBoundary,
}

/// Réservation d'une unité globale toujours comptée par le writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveExecution {
    /// Identifiant de la réservation en cours, renouvelé à chaque réadmission.
    pub reservation_id: ReservationId,
    /// Unité propriétaire de la réservation.
    pub unit: ExecutionUnit,
    /// Projet propriétaire, absent pour une sonde de portée utilisateur.
    pub project_id: Option<ProjectId>,
    /// Horodatage d'admission utilisé pour départager les cibles.
    pub admitted_at: TimestampMs,
    /// Frontière sûre ou capacité de coopérer observée.
    pub checkpoint: CheckpointEligibility,
}

/// Demande déjà éligible hors contraintes du pool global.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedExecution {
    /// Unité candidate à l'admission.
    pub unit: ExecutionUnit,
    /// Projet propriétaire, absent pour une sonde de portée utilisateur.
    pub project_id: Option<ProjectId>,
    /// Date d'entrée en file utilisée pour le vieillissement.
    pub queued_at: TimestampMs,
    /// Permet à un contrôle éligible de solliciter un checkpoint à pleine capacité.
    pub urgent: bool,
}

/// Poids et crédit d'équité issus de la configuration et des admissions commises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectFairness {
    /// Poids compris entre 1 et 10.
    pub weight: u8,
    /// Crédit accumulé par le projet.
    pub deficit: i64,
}

impl Default for ProjectFairness {
    fn default() -> Self {
        Self {
            weight: 1,
            deficit: 0,
        }
    }
}

/// Demande coopérative proposée au writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointRequest {
    /// Invocation de contrôle qui attend la capacité.
    pub waiting_control: InvocationId,
    /// Réservation de travail ciblée.
    pub target: ReservationId,
}

/// Observation d'une demande de checkpoint enregistrée par l'autorité.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedCheckpoint {
    /// Demande observée.
    pub request: CheckpointRequest,
    /// Durée monotone écoulée depuis la demande, si elle est établie.
    pub elapsed: Option<Duration>,
    /// Indique qu'une demande est toujours en cours pour cette cible.
    pub pending: bool,
}

/// Vue cohérente fournie par l'autorité d'admission.
#[derive(Debug, Clone, Copy)]
pub struct AdmissionSnapshot<'a> {
    /// Paramètres effectifs observés.
    pub settings: SchedulingSettings,
    /// Gel d'admission signalé par l'autorité.
    pub admissions_frozen: bool,
    /// Toutes les unités détenues ou en réconciliation, une entrée par unité réservée.
    /// Une cible sollicitée reste ici jusqu'à la libération observée par le writer.
    pub active: &'a [ActiveExecution],
    /// File éligible après validation des dépendances, droits, budgets et sous-limites.
    pub eligible: &'a [QueuedExecution],
    /// Poids et crédits par projet. Une entrée absente vaut poids 1, crédit 0.
    pub projects: &'a BTreeMap<Option<ProjectId>, ProjectFairness>,
    /// Longueur de la rafale de contrôles admis avec du travail éligible en file.
    pub consecutive_controls_with_work: u64,
    /// Historique de l'attente courante des contrôles en file et demandes encore en cours,
    /// y compris celles émises pour un autre contrôle.
    pub checkpoints: &'a [ObservedCheckpoint],
}

/// Cause d'attente restante après les vérifications d'éligibilité.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityWait {
    /// L'autorité a gelé les admissions.
    AdmissionsFrozen,
    /// Toutes les unités globales sont encore détenues.
    GlobalCapacity,
    /// Les unités disponibles sont réservées au contrôle par la réserve dure.
    HardControlReserve,
}

/// Proposition d'admission d'une unité avec sa comptabilité d'équité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPlan {
    /// Unité candidate à faire valider et réserver par le writer.
    pub unit: ExecutionUnit,
    /// Variations de crédit à appliquer seulement sur admission commise.
    pub deficit_deltas: BTreeMap<Option<ProjectId>, i64>,
    /// Nouvelle longueur de rafale à appliquer sur le même commit.
    pub consecutive_controls_with_work: u64,
}

/// Décision consultative calculée à partir d'un snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Aucune unité éligible.
    Idle,
    /// Une unité éligible attend de la capacité.
    Wait {
        /// Prochain candidat selon la politique d'admission.
        unit: ExecutionUnit,
        /// Cause d'attente de capacité.
        reason: CapacityWait,
        /// Demande coopérative dont le contrôle demandeur est dans `waiting_control`.
        checkpoint: Option<CheckpointRequest>,
    },
    /// Proposition de prochaine admission.
    Admit(AdmissionPlan),
}

/// Snapshot refusé par la politique.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchedulingError {
    /// Plafond, réserve, rafale ou délai d'escalade incohérent.
    #[error("paramètres de capacité ou de réserve de contrôle incohérents")]
    ControlReserveIncoherent,
    /// Poids hors des bornes de la politique.
    #[error("poids {weight} hors de 1..=10 pour le projet {project_id:?}")]
    InvalidProjectWeight {
        /// Projet ou portée utilisateur.
        project_id: Option<ProjectId>,
        /// Valeur hors des bornes.
        weight: u8,
    },
    /// Contradiction entre l'occupation observée et la limite effective.
    #[error("occupation observée {occupied} au-delà de la limite effective {limit}")]
    OverCapacity {
        /// Unités actives observées.
        occupied: usize,
        /// Limite effective du pool.
        limit: u8,
    },
}

/// Propose une unique décision d'admission ou d'attente.
///
/// Le contrôle prime jusqu'au seuil anti-famine si du travail éligible attend.
/// Une fois le seuil atteint, le travail le plus ancien passe dès qu'une unité lui
/// est accessible. La réserve dure reste protégée et le mode souple permet ce tour
/// de travail même avec une seule unité globale.
///
/// À pleine capacité, le contrôle urgent qui sollicite un checkpoint est choisi
/// indépendamment du candidat à la prochaine admission.
///
/// # Errors
/// Renvoie [`SchedulingError`] quand la configuration ou l'occupation est incohérente.
pub fn decide(snapshot: &AdmissionSnapshot<'_>) -> Result<Decision, SchedulingError> {
    let settings = snapshot.settings;
    validate(snapshot)?;
    let oldest_work = snapshot
        .eligible
        .iter()
        .filter(|entry| entry.unit.admission_class() == AdmissionClass::Work)
        .min_by_key(|entry| entry.age_key());

    let work_allowed_by_reserve = settings.control_reserve_mode == ControlReserveMode::Soft
        || snapshot
            .active
            .iter()
            .filter(|active| active.unit.admission_class() == AdmissionClass::Work)
            .count()
            < usize::from(settings.effective_limit - settings.control_reserved_units);
    let forced_work = oldest_work.filter(|_| {
        work_allowed_by_reserve
            && snapshot.consecutive_controls_with_work >= settings.control_burst_limit
    });
    let Some(next) = forced_work.or_else(|| {
        snapshot.eligible.iter().max_by_key(|entry| {
            (
                entry.unit.admission_class() == AdmissionClass::Control,
                snapshot.weighted_queue_key(entry),
            )
        })
    }) else {
        return Ok(Decision::Idle);
    };
    let class = next.unit.admission_class();

    let wait_reason = if snapshot.admissions_frozen {
        Some(CapacityWait::AdmissionsFrozen)
    } else if snapshot.active.len() == usize::from(settings.effective_limit) {
        Some(CapacityWait::GlobalCapacity)
    } else if class == AdmissionClass::Work && !work_allowed_by_reserve {
        Some(CapacityWait::HardControlReserve)
    } else {
        None
    };
    if let Some(reason) = wait_reason {
        let checkpoint = if reason == CapacityWait::GlobalCapacity {
            checkpoint_at_capacity(snapshot)
        } else {
            None
        };
        return Ok(Decision::Wait {
            unit: next.unit,
            reason,
            checkpoint,
        });
    }

    // Chaque projet en lice reçoit son poids une fois. Le projet servi paie la
    // somme des poids. Seul le commit de cette admission modifie les crédits.
    let mut deficit_deltas: BTreeMap<_, _> = snapshot
        .eligible
        .iter()
        .filter(|entry| entry.unit.admission_class() == class)
        .map(|entry| {
            (
                entry.project_id,
                i64::from(snapshot.project(entry.project_id).weight),
            )
        })
        .collect();
    let weight_sum: i64 = deficit_deltas.values().sum();
    *deficit_deltas.entry(next.project_id).or_default() -= weight_sum;

    let consecutive_controls_with_work =
        if class == AdmissionClass::Control && oldest_work.is_some() {
            snapshot.consecutive_controls_with_work.saturating_add(1)
        } else {
            0
        };
    Ok(Decision::Admit(AdmissionPlan {
        unit: next.unit,
        deficit_deltas,
        consecutive_controls_with_work,
    }))
}

impl QueuedExecution {
    fn age_key(&self) -> (TimestampMs, ExecutionUnit) {
        (self.queued_at, self.unit)
    }
}

impl AdmissionSnapshot<'_> {
    fn project(&self, id: Option<ProjectId>) -> ProjectFairness {
        self.projects.get(&id).copied().unwrap_or_default()
    }

    fn weighted_queue_key(
        &self,
        entry: &QueuedExecution,
    ) -> (i128, Reverse<(TimestampMs, ExecutionUnit)>) {
        let project = self.project(entry.project_id);
        (
            i128::from(project.deficit) + i128::from(project.weight),
            Reverse(entry.age_key()),
        )
    }
}

fn validate(snapshot: &AdmissionSnapshot<'_>) -> Result<(), SchedulingError> {
    let settings = snapshot.settings;
    let limit = settings.effective_limit;
    if !(1..=50).contains(&limit)
        || !(1..=(limit / 2).max(1)).contains(&settings.control_reserved_units)
        || (settings.control_reserve_mode == ControlReserveMode::Hard && limit < 2)
        || settings.control_burst_limit == 0
        || !(10..=600).contains(&settings.checkpoint_escalation_s)
    {
        return Err(SchedulingError::ControlReserveIncoherent);
    }
    if snapshot.active.len() > usize::from(limit) {
        return Err(SchedulingError::OverCapacity {
            occupied: snapshot.active.len(),
            limit,
        });
    }
    for (&project_id, project) in snapshot.projects {
        if !(1..=10).contains(&project.weight) {
            return Err(SchedulingError::InvalidProjectWeight {
                project_id,
                weight: project.weight,
            });
        }
    }
    Ok(())
}

fn checkpoint_at_capacity(snapshot: &AdmissionSnapshot<'_>) -> Option<CheckpointRequest> {
    let (_, control) = snapshot
        .eligible
        .iter()
        .filter_map(|entry| match (entry.unit, entry.urgent) {
            (ExecutionUnit::Control(control), true) => Some((entry, control)),
            _ => None,
        })
        .max_by_key(|(entry, _)| snapshot.weighted_queue_key(entry))?;

    let escalation_delay =
        Duration::from_secs(u64::from(snapshot.settings.checkpoint_escalation_s));
    if snapshot.checkpoints.iter().any(|entry| {
        entry.request.waiting_control == control
            && entry
                .elapsed
                .is_none_or(|elapsed| elapsed < escalation_delay)
    }) {
        return None;
    }
    let excluded: BTreeSet<_> = snapshot
        .checkpoints
        .iter()
        .filter(|entry| entry.pending || entry.request.waiting_control == control)
        .map(|entry| entry.request.target)
        .collect();

    // La part pondérée concerne les projets actifs ou éligibles de ce snapshot.
    // Après multiplication par la somme des poids, l'écart se compare sans arrondi.
    let mut occupied_by_project: BTreeMap<_, i64> = snapshot
        .eligible
        .iter()
        .map(|entry| (entry.project_id, 0))
        .collect();
    for active in snapshot.active {
        *occupied_by_project.entry(active.project_id).or_default() += 1;
    }
    let total_weight: i128 = occupied_by_project
        .keys()
        .map(|&id| i128::from(snapshot.project(id).weight))
        .sum();
    let limit = i128::from(snapshot.settings.effective_limit);
    let target = snapshot
        .active
        .iter()
        .filter(|entry| {
            entry.unit.admission_class() == AdmissionClass::Work
                && entry.checkpoint != CheckpointEligibility::Unavailable
                && !excluded.contains(&entry.reservation_id)
        })
        .max_by_key(|entry| {
            let occupied = occupied_by_project
                .get(&entry.project_id)
                .copied()
                .unwrap_or_default();
            let weight = i128::from(snapshot.project(entry.project_id).weight);
            let excess = i128::from(occupied) * total_weight - limit * weight;
            (
                entry.checkpoint == CheckpointEligibility::SafeBoundary,
                excess,
                entry.admitted_at,
                Reverse(entry.reservation_id),
            )
        })?;
    Some(CheckpointRequest {
        waiting_control: control,
        target: target.reservation_id,
    })
}
