# F017 — interfaces

## Interfaces internes proposées

AuthorizeAction(ActionIntent, EffectiveAuthority) → AuthorizationDecision. ExecuteAuthorized(ActionId, DecisionRef, Fence) → ActionReceipt. GuardPort négocie capacité/santé, évalue une action et peut exposer révocation, suspension et audit selon protocole réel ([ai-guard](../../specs/ai-guard.md)). Le trait Rust `GuardPort` vit dans `unleash-action-broker` ; l’adaptateur réel est la crate séparée `unleash-guard-adapter`, identifiée par un `guard_mapping_profile` versionné ; le double déterministe vit dans `unleash-testkit`.

ActionIntent contient action_id, subject {kind invocation|replay|control_plane, principal_id, cause_ref}, project_id, agent_id?/mission_id?/role_version? selon kind, verb, resource_refs, payload_digest (= `actions.payload_hash`), effects_class, scope_revision, policy_revision et action_deadline. Les isolation_requirements proviennent d’EffectiveAuthority. effects_class appartient à la liste fermée versionnée `local_read`, `local_write`, `process_exec`, `terminal_input`, `git_local_write`, `git_remote`, `connector_read`, `connector_write`, `browser_action`, `model_dispatch`, `network_egress` ; une classe inconnue est refusée avant GuardPort (`InvalidArgument/EFFECTS_CLASS_UNKNOWN`). Decision comporte allow/deny/require_approval/unavailable/unsupported, raisons et contraintes ; l’expiry d’un Allow borne la décision et reste distincte d’action_deadline. Une approbation porte sur une action ou un mandat borné, jamais sur tout texte futur du modèle.

SecretStore retourne une valeur uniquement au composant transport autorisé et des erreurs Locked/Unavailable/Denied/NotFound. Les commandes GUI/CLI créent ou remplacent SecretRef ; elles ne lisent pas les valeurs en export général.

## Politique, guard et erreurs

La politique projet (SetPolicy, lue par ReadPolicySummary) porte `isolation_mode = standard | strict`, `standard` par défaut ([frontières de sécurité](../../architecture/security-boundaries.md), Q-05) ; passer un projet en `strict` ou en revenir est un changement de politique tracé avec acteur, motif et révision. ConfigureGuard porte `guard_mode = broker_only | guard_optional | guard_required`, l’endpoint, le transport, les délais et la `fallback_policy_ref` exigée par guard_optional ; `broker_only` est le défaut sans endpoint et `guard_required` le devient dès qu’un endpoint est enregistré (Q-04). ReadGuardHealth expose santé, version négociée, capacités et `guard_mapping_profile`. SuspendSubject et ResumeSubject suspendent puis rétablissent localement un sujet typé (portée, motif, révision), distinctement de RevokeSubject ; ResolveActionApproval enregistre la décision humaine sur une action `approval_required` (action_id, approval_scope, payload_digest, décision approve/deny, motif, révision attendue) et la mène à `authorized` ou `denied`. Les noms wire sont ceux du [catalogue commun](../../architecture/client-operation-catalog.md).

### Mandats de sonde fournisseur

Sous `SetPolicy`, le champ facultatif `provider_probe_mandate` engage une émission ou une révocation ciblée dans la transaction de politique, sous la même autorisation d’administration et le même contrôle de révision. Le [schéma de commande](../../schemas/ipc-command.schema.json) ferme le payload et la [fixture d’émission](../../examples/ipc-set-policy-probe-mandate.json) illustre la forme. Une commande sans ce champ conserve les mandats déjà engagés. Une émission n’ouvre aucune connexion et ne résout aucun secret.

`issue` désigne un principal exact, un UUID et une version de profil, une phase d’effet (`connect`, `inventory` ou `generation_probe`), la localité, les capacités autorisées, les plafonds et une expiration future. Le writer résout le profil dans la même portée, dérive le digest de sa configuration et sa destination normalisée, et engage ces valeurs avec la lignée courante. La politique globale couvre les profils `user/default`. Une politique projet couvre les profils de ce projet. Le `receipt_id` d’émission identifie ce mandat et devient `mandate_ref` pour le probe. Le serveur vérifie séparément le principal authentifié, ses droits, la portée et le gel.

`revoke` désigne le `mandate_ref` exact dans la même portée. La mutation préserve les autres mandats et leur reçu d’émission. Le rejeu d’une ancienne émission restitue le résultat de déduplication sans modifier l’état courant. Après restauration, un mandat de l’ancienne lignée reste historique. Le reçu seul ne réactive aucun droit. La projection `ReadPolicySummary` inclut, pour les lecteurs autorisés sur la portée, les références d’émission, paramètres publics, digests et états actif, expiré, révoqué ou historique. Elle conserve le watermark de la transaction de lecture.

Le tableau `result.provider_probe_mandates` expose `mandate_ref`, `scope_kind`, `scope_id`, `authority_generation`, `principal_id`, `profile_id`, `profile_version`, `configuration_digest`, `phase`, `locality`, `capability_set`, `limits`, `expires_at_ms`, `mandate_digest`, `status` et `revocation_receipt_id` (nul avant révocation). La projection applique les statuts dans l’ordre `historical` pour une autre lignée, puis `revoked`, `expired` et `active`.

Le digest de politique et le snapshot du broker couvrent les bindings des mandats émis et les révocations de leur portée. Une décision mise en cache ou une révision numérique maximale ne dispensent pas de comparer les digests effectifs. Juste avant dispatch, le writer recoupe sous sa génération et son fence courants l’état du mandat, son expiration, le profil exact, le principal, le scope et sa révision, les limites, la destination effective et la politique de l’origine. L’expiration du mandat borne la deadline d’action et reste distincte de l’expiration de la décision du garde. Un mandat révoqué, caché, expiré ou étranger à la demande est indisponible pour une nouvelle intention. Un dispatch déjà engagé suit la réconciliation des effets s’il ne dispose pas d’une observation fiable.

Projection wire des erreurs F017, enregistrée par le propriétaire dans le format commun d’[ipc-and-apis](../../architecture/ipc-and-apis.md) (nom de domaine, classe/reason_code, retry_class) :

| Nom de domaine | Wire (classe/reason_code) | retry_class |
| --- | --- | --- |
| SecretStore Locked | CapabilityDenied/SECRET_STORE_LOCKED | after_human_decision |
| SecretStore Unavailable | InternalFailure/SECRET_STORE_UNAVAILABLE | after_backoff |
| SecretStore Denied | CapabilityDenied/SECRET_ACCESS_DENIED | never |
| SecretStore NotFound | NotFoundOrHidden/SECRET_REF_NOT_FOUND | never |
| PolicyDenied | CapabilityDenied/POLICY_DENIED | never |
| Classe d’effet inconnue | InvalidArgument/EFFECTS_CLASS_UNKNOWN | never |
| Deny | CapabilityDenied/GUARD_DENIED | never (pour ce digest) |
| Allow à obligation insatisfaite | CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED | never |
| RequireApproval admise par le profil de mapping | CapabilityDenied/GUARD_APPROVAL_REQUIRED | after_human_decision |
| RequireApproval non prise en charge | CapabilityDenied/GUARD_APPROVAL_UNSUPPORTED | never |
| Unavailable, timeout ou réponse malformée | CapabilityDenied/GUARD_UNAVAILABLE | after_backoff |
| UnsupportedCapability ou version non négociée | ProtocolIncompatible/GUARD_PROTOCOL_UNSUPPORTED | never |
| Sujet suspendu | CapabilityDenied/SUBJECT_SUSPENDED | after_human_decision |
| Sujet révoqué | CapabilityDenied/SUBJECT_REVOKED | never |
| Backend d’isolation absent ou en échec | IsolationUnavailable/ISOLATION_BACKEND_MISSING | after_human_decision |
| Obligation d’isolation non imposable | IsolationUnavailable/ISOLATION_OBLIGATION_UNENFORCEABLE | after_human_decision |

## Événements et audit

ActionAuthorizationRequested, ActionApprovalRequired, ActionAuthorized, ActionDenied, ActionDispatchCommitted, ActionObserved, SubjectSuspended, SubjectResumed, SubjectRevoked, GuardHealthChanged et IncidentOpened. Le journal conserve décision, version politique, identité logique, contexte de demande et preuve d’exécution. Les paramètres sensibles sont redacted selon schéma ; les hashes ne contiennent pas de clés exploitables.

## Contrat externe manquant

Le dossier ne spécifie aucun endpoint AI-Guard prétendument existant. Il exige un mapping versionné (`guard_mapping_profile` de `unleash-guard-adapter`, gate EXT-GUARD) avec auth/TLS éventuel, codes d’erreurs, conditions d’idempotence, annulation, limites et références d’audit. Si une capacité n’existe pas, l’UI l’indique et le critère correspondant reste ouvert, sans simuler un succès.

Le format d’enveloppe, les erreurs communes et la diffusion des événements suivent [IPC](../../architecture/ipc-and-apis.md). Les identités et versions sont celles du [modèle de stockage](../../architecture/storage-model.md). Tous les noms de ports ci-dessus sont des contrats internes Unleash (ADR-10).

## Obligations de décision et sujets non agents R6

Le GuardPort interne normalise les sujets invocation/replay/control_plane ; agent_id et rôle ne sont pas obligatoires pour une action sans agent. Une réponse Allow est convertie en obligations internes versionnées selon [ai-guard](../../specs/ai-guard.md). Avant secret/dispatch, chaque obligation doit être appliquée et prouvée applicable ; inconnue/non imposable/contradictoire refuse GUARD_OBLIGATION_UNSATISFIED. Le mapping réel reste gate EXT-GUARD ; profil initial `obligations=none_accepted` : tout Allow assorti d’une contrainte additionnelle est refusé. Le reçu de décision n’est pas la preuve d’application du contrôle.
