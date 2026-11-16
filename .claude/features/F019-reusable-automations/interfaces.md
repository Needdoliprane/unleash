# Interfaces F019

**Contrats internes proposés, pas API externe implémentée.** Toutes les mutations utilisent le `CommandEnvelope`, `CommandReceipt`, `Error`, l'identité authentifiée, la clé d'idempotence et `expected_revision` de [l'IPC commun](../../architecture/ipc-and-apis.md). Les lectures appliquent les mêmes ACL ; une lecture cachée retourne `NotFoundOrHidden`. Les noms complètent le [catalogue client](../../architecture/client-operation-catalog.md), sans façade réservée à la GUI.

## Commandes de conception et de registre

| Commande | Entrées et préconditions structurantes | Résultat référencé par le receipt |
|---|---|---|
| `CreateAutomationDraft` | Projet/scope, intention, référence source éventuelle, discovery receipt/décision et identité de création. Aucun appel modèle implicite. | Draft révisionné et liste des données de conception manquantes. |
| `SaveAutomationDraft` | Draft, révision attendue, contenu ou patch typé, références et provenance. | Révision nouvelle ou conflit ; jamais modification d'une version publiée. |
| `LearnAutomation` | Draft exact, cible autorisée, mandat de conception, politique modèle et budgets. | Job de conception avec session/provenance ; résultats proposés dans une nouvelle révision de draft. |
| `ValidateAutomationDraft` | Snapshot de draft, versions de compilation/catalogue, modes demandés et profil de plateforme. | Diagnostics de forme/sémantique/autorité, bornes et dépendances manquantes ; aucun modèle. |
| `EvaluateAutomationDraft` | Digest exact du candidat, révision de données, scénarios/oracles, cible fixture/test/live déclarée et autorisée, mode/politique et budget d'évaluation. | Évaluation, observations, appels éventuels de supervision déclarés, contradictions et limites ; aucune publication implicite. |
| `PublishAutomationVersion` | Draft exact, résultats de validation/évaluation requis, statut de gates et droit de publication. | `AutomationArtifact` version/digest immuables, références de workflow par mode supporté. |
| `ImportAutomationData` | Source autorisée discriminée : `resource_ref` (snapshot F013), `project_file` (chemin relatif sous une racine de projet validée) ou `client_upload` (octets lus par l'adaptateur client Rust de confiance, jamais par la WebView, transmis en segments bornés avec digest final, la limite d'import de 10 MiB excédant une frame IPC de 8 MiB) ; aucun chemin absolu fourni par le client n'est accepté. Profil de parsing, schéma/mapping, identité de lignes et limites. | `AutomationDataRevision` immuable et diagnostics d'import ; aucune exécution métier. |
| `ValidateAutomationData` | Révision exacte, version d'artefact, paramètres complémentaires déclarés et mode. | Rapport de validité complet, bornes effectives et erreurs par ligne/colonne ; aucun LLM. |
| `RepairAutomation` | Version source, incident/evidence bundle, mandat explicite utilisateur ou politique préautorisée versionnée, budget de conception. | Nouveau draft et job séparé ; provenance source conservée, run engagé inchangé. |
| `DeprecateAutomationVersion` | Version, révision de registre, motif et droit associé. | Éligibilité de sélection mise à jour ; historique et usages conservés. |
| `RevokeAutomationVersion` | Version, motif, portée et politique de révocation. | Blocages applicables aux futurs dispatchs/bindings ; aucun effacement des preuves historiques. |

Une évaluation en `replay` est sans modèle ; une évaluation `supervised` annonce les points et coûts de supervision. Les effets d'une cible réelle ne deviennent pas sûrs par le nom « évaluation » : le mandat et le broker restent applicables. Les opérations de parsing, compilation et normalisation ne demandent pas à un LLM de deviner une donnée manquante.

## Exécution : réutilisation des opérations existantes

| Opération existante | Extension ou usage pour F019 |
|---|---|
| `CreateRun` | `source` discriminée `workflow_version` ou `automation_version` ; pour cette dernière, référence exacte, `AutomationDataRevision`, `automation_mode`, paramètres et limites. Le daemon dérive et vérifie `model_policy` et le workflow du mode sélectionné. |
| `StartRun`, `PauseRun`, `ResumeRun`, `CancelRun` | Même révision et machine d'état que F005 ; reprise interdite si dépendance, autorité ou effet restent non résolus. Aucun changement implicite de mode/version. |
| `GetRun` | Projection `AutomationRun` : version, données, mode, états de lignes/effets, checkpoint, supervision, compteurs et actions permises. Pour un Run `prepared` créé directement par CreateRun, elle porte l'`EffectPreview`, son digest et les faits observés référencés ; StartRun et toute décision humaine portent ce digest ([aperçu](../../specs/reusable-automations.md#aperçu-autorisation-et-effets)). |
| `AcceptStep` | Décision humaine déclarée, révision/evidence bundle précis et droits ; ne remplace ni assertion machine obligatoire ni réconciliation d'un effet inconnu. Après une intervention humaine prévue (MFA, CAPTCHA, session), elle déclare seulement la fin d'intervention : elle n'accepte pas l'étape et autorise l'activation d'observation publiée qui peut lever le blocage ([design technique](technical-design.md#états-et-erreurs)). |
| `ReconcileAction`, `ResolveActionOutcome` | Protocole F018 ; preuve ou disposition autorisée reste distincte d'une réussite observée. Sous forbidden, aucun juge modèle caché. |
| `DiscoverResources`, `RecordDiscoveryDecision` | Type `automation`, candidats, fraîcheur, réutilisation/création et provenance. |
| `GetVersionDiff`, `ListVersionUsages` | Comparer versions d'automatisation et identifier runs/drafts qui les référencent. |
| `CreateReportSnapshot`, `GetReportSnapshot`, `ExportReport` | Template déterministe, watermark, masquage, origine des appels de supervision/réparation et limites de preuve. |
| `SubscribeEvents` | Flux commun, cursor/déduplication/resynchronisation ; aucune notification navigateur privée non durable. |
| `GetHealth` | Lecture F001 de l'inventaire des runtimes navigateur épinglés (version, SHA-256 de Chrome for Testing et de chromedriver, plateforme, état), préparés par l'étape explicite sous mandat de l'[architecture navigateur](../../architecture/browser-execution.md#3-contenu-de-la-version-et-du-manifest-de-run) ; aucun téléchargement ni probe implicite. |

Il n'existe pas de commande concurrente `CreateAutomationRun` ou `StartAutomationRun`. La commande de création épingle les références ; StartRun ne suit pas un alias plus récent. Une nouvelle révision de données ou un autre mode exige un nouveau run autorisé et les barrières de succession applicables.

## Lectures propres au registre

| Requête | Contenu |
|---|---|
| `SearchAutomations` | Candidats autorisés, modes, version, statut, paramètres, profil d'exécution, provenance et raisons de classement. |
| `GetAutomationDraft` | Révision éditable, contenu, diagnostics et références de conception accessibles. |
| `GetAutomationVersion` | Manifest exact, schémas, contrats, workflows par mode, dépendances, qualification et statut de registre actuel. |
| `GetAutomationDataRevision` | Digest/profil/schema/mapping, lignes paginées autorisées, erreurs et limites ; secrets exclus. |
| `GetAutomationEvaluation` | Cible et mode évalués, candidat exact, environnement, oracles, résultats, appels modèle et preuves manquantes. |

## Projection de run et contrats de driver

La projection proposée comprend `run_id`, source/version/digests, `automation_mode`, `model_policy`, référence de données, manifest de workflow et de driver, `row_results`, `effect_states`, checkpoint, `supervision_checkpoints`, réservations, `allowed_operations` et watermark. Les secrets apparaissent seulement comme références/statuts autorisés. Les preuves sont lues par référence et pagination, pas envoyées intégralement dans chaque événement.

Les compteurs distinguent `model_dispatch_count`, `model_denied_attempt_count`, invocations de supervision, usage retourné/inconnu, coût modèle et coût des effets/ressources. Zéro dispatch ne signifie pas qu'aucune tentative de contournement n'a été refusée ; un refus doit rester inspectable. En supervised, chaque dispatch est lié à une tâche/checkpoint déclaré. Un job de réparation distinct reste visible dans la mission et ne disparaît pas des totaux.

Le driver reçoit une activation/actions typées, paramètres épinglés, scope, contrat, deadline, identité/fence et `model_policy` dérivée. Il retourne des observations attribuables, l'état de l'effet et des références de preuves ; il ne décide ni de la politique, ni de la clôture du run. Les opérations navigateur et leurs limites exactes appartiennent au [BrowserPort](../../architecture/browser-execution.md) ; aucun nom de méthode Playwright n'est défini ici. Le backend de référence V1 est l'adaptateur WebDriver `unleash-browser` (`fantoccini`) sur chromedriver et Chrome for Testing épinglés ; chromedriver est lancé par F011, la préparation du runtime est une étape explicite sous mandat sans commande dédiée et la sortie réseau du navigateur passe par le proxy d'egress d'Unleash ([Q-18](../../brief/open-questions.md)).

La sortie de supervision contient une décision dans `continue/reconcile/correct/escalate`, cibles autorisées, référence d'evidence bundle/révision et justification. Un parseur ne rend pas cette décision vraie : F008 vérifie contrat, fraîcheur et observations contradictoires avant d'en faire une commande admissible. Une décision sans preuve requise retourne `ContractViolation`.

## Événements et erreurs

Événements proposés : `AutomationDraftCreated`, `AutomationDraftSaved`, `AutomationLearningStarted`, `AutomationValidationCompleted`, `AutomationEvaluationCompleted`, `AutomationVersionPublished`, `AutomationDataImported`, `AutomationDataValidated`, `AutomationRepairRequested`, `AutomationVersionDeprecated` et `AutomationVersionRevoked`. Le lifecycle utilise les événements Run/Action existants. Les observations de lignes, checkpoints et décisions sont reliées aux activations de ce run ; elles ne pilotent pas une seconde machine d'état.

Les classes et raisons de [technical-design.md](technical-design.md), y compris la correspondance des noms de domaine du profil navigateur, passent dans l'objet Error commun : `code`, `reason_code`, `message`, `retry_class`, `details_ref`, `current_revision` éventuelle et `command_id`. Un conflit de révision ne publie pas partiellement un draft ; un effet au résultat inconnu n'est pas classé « réessayer immédiatement ». Un client déconnecté retrouve le receipt de sa commande au lieu de lancer un autre run par supposition.

## Modes publiés et jobs de conception

Une version d’AutomationArtifact peut publier replay, supervised ou les deux. Chaque entrée porte sa référence exacte de workflow/policy/driver ; CreateRun sélectionne une entrée existante et épingle cette résolution. Choisir un mode absent est un refus, pas une compilation implicite. La projection SQL correspondante est automation_version_modes ; le Run possède une FK de mode et aucune substitution en vol.

learn et repair décrivent les jobs de conception lancés par leurs commandes sous mandat distinct. Ils produisent un draft ; ils ne sont pas des modes d’exécution d’une version d’automatisation publiée. Ils réutilisent les tâches/invocations et, lorsqu’un plan de conception est utilisé, un Run de source workflow_version avec leurs références de draft/provenance. Les évaluations utilisent des snapshots candidats immuables sous autorité de qualification, jamais l’autorisation générale de lancer un draft mutable. L’interface distingue conception, évaluation et exécution publiée.

## Corrections de données par nouvelle révision R6

ImportAutomationData peut porter parent_data_revision_ref et correction_provenance_ref en plus de la source autorisée et du parsing/mapping. Le backend vérifie parent lisible, scope, taille et cohérence du diff déclaré avec les données réimportées ; les déclarations de correction ne remplacent pas ce contrôle. Le résultat est une nouvelle révision immuable, validée entièrement par ValidateAutomationData. Aucune commande de mise à jour en place d’une cellule ni de retry automatique d’effets inconnus n’est introduite. La copie de travail GUI/TUI et ses limites sont décrites dans [l’expérience guidée](../../design/guided-automation-experience.md).
