# F014 — interfaces

**Révision : 16 novembre 2026.** Contrats Unleash proposés ; les noms ci-dessous ne sont pas des API fournies par Tauri ou par un framework de rendu.

## Backend consommé

La GUI consomme les requêtes/commandes publiques du [catalogue IPC](../../architecture/ipc-and-apis.md), sans variante métier réservée au graphique. Exemples de lectures existantes dans ce catalogue : GetOverview, ListGoals, GetMission, ListAgents, GetConversation, GetWorkflowVersion, GetRun, SearchSkills, GetSkillVersion, SearchRoles, GetRoleVersion, GetVersionDiff, ListVersionUsages, SearchKnowledge, GetContextManifest, ReadEvidence, ListProcesses, ReadSettings, BrowseResources, SearchResources, PreviewResource et GetReportSnapshot. La liste est illustrative ; tout écran et parcours de la matrice doit avoir son contrat versionné, sans inventer un verbe privé qui contourne l’autorité.

GuiProjection conserve stream_id, last_sequence, snapshot_revision, connection_state et selection. PendingCommand conserve l’enveloppe d’origine complète (command_id, authority_generation, command_type, project_id, aggregate_id, expected_revision, payload canonique et digest), submitted_at et receipt_state ; aucun champ n’est recalculé lors d’un retry ([IPC](../../architecture/ipc-and-apis.md)). Les brouillons sont des DraftRef avec base_revision ; application/publication exige validation serveur. Les clients CLI/TUI obtiennent les mêmes erreurs, reçus, ressources et versions, pour une même identité et le même état initial. Les profils et variantes de source Run sont négociés, sans fallback implicite d’une automatisation vers un agent.

| Surface commune R3–R5 | Commandes consommées | Lectures et résultats |
| --- | --- | --- |
| Guidage dans Overview | CreateGuidedSetupDraft, SaveGuidedSetupDraft, ValidateGuidedSetup, ApplyGuidedSetup | ListGuidedTemplates, GetGuidedSetupDraft, GetGuidedSetupValidation, GetSetupPreview ; template/version, diff des defaults, sources/mode/provider conditionnel/policies et agrégats à préparer. |
| Drafts de procédure dans Workflows | CreateAutomationDraft, SaveAutomationDraft, LearnAutomation | SearchAutomations, GetAutomationDraft, GetAutomationVersion ; provenance, modes et sources exactes. |
| Données | ImportAutomationData, ValidateAutomationData | GetAutomationDataRevision ; lignes/mapping/erreurs et rapports référencés par résultats communs. |
| Qualification et registre | ValidateAutomationDraft, EvaluateAutomationDraft, PublishAutomationVersion, DeprecateAutomationVersion, RevokeAutomationVersion | GetAutomationEvaluation, GetVersionDiff, ListVersionUsages, ReadArtifact ; aucune évaluation modèle/live implicite. |
| Incident/réparation dans Reports | RepairAutomation | Même run source, preuves/incident, nouveau draft/job, diff et coûts séparés. |
| Exécution dans Activity | CreateRun, StartRun, PauseRun, ResumeRun, CancelRun, AcceptStep ; ReconcileAction/ResolveActionOutcome selon mandat | GetRun : source workflow_version ou automation_version, version/données/mode/politique, lignes, effets, checkpoints, budgets, appels et décisions. |
| Profils locaux dans Settings | SetProviderProfile, TestProviderProfile ; import/référence de secret uniquement si requis | ListProviderProfiles, GetProviderProfile, GetHealth, GetSecretReferenceStatus ; dialecte, santé/capacités/localité et fraîcheur. |

F019 conserve exactement **11 commandes et 5 lectures propres** ; diff, Run, rapports, reçus et flux restent communs. Le guidage ajoute **4 commandes et 4 lectures**, toutes reprises par F015. ApplyGuidedSetup prépare les agrégats locaux de façon idempotente, sans Learn/probe/StartRun implicite. Les mandats préconfigurés évitent les confirmations répétées ; la GUI ne peut élargir les permissions.

## Adaptateur Tauri (ADR-03)

| Frontière | Contrat et contrôle |
| --- | --- |
| WebView → hôte Rust | DTO de présentation stricts dont les types TypeScript sont générés par ts-rs, une commande Tauri générée par opération exposée, capability de fenêtre/WebView et scopes vérifiés ; aucune invocation libre de shell/SQL/chemin/socket |
| Hôte → daemon | CommandEnvelope et requêtes du client IPC commun `unleash-client`, authentification locale, command_id, authority_generation et expected_revision conservés ; actor_ref validé par le daemon |
| Daemon → hôte → WebView | Projection autorisée, Receipt/Error communs, flux borné avec stream_id/sequence et logs avec offset ; redaction avant exposition au renderer |
| Configurer un secret | Initier/suivre un parcours du broker/coffre depuis Settings ; saisir dans une fenêtre native masquée créée par l’hôte hors WebView (`unleash-platform`, ADR-03) ; exposer seulement SecretRef/statut, jamais le secret dans la WebView |
| Fermer une fenêtre ou l’hôte | Libérer abonnements et ressources du client ; aucune traduction implicite en CancelMission, StopProcess ou arrêt du daemon |

Le registre des commandes Tauri et ses permissions est généré depuis l’énumération Rust des opérations du catalogue et de la [matrice de parité](../../design/gui-tui-parity.md) ; les commandes sont accordées à la seule fenêtre `main`. Aucune commande générique de relais donnant accès à tout le protocole n’existe. Les métadonnées de capacité d’affichage n’accordent pas de privilège ; seul le backend autorise l’effet. Les plugins système ne sont pas exposés largement à la WebView pour contourner cet adaptateur.

## Contrats d’affichage et équivalents terminal

AgentRow distingue logical_state et invocation_state ; ModelBadge indique profil effectif, provenance, localité et capacités sans confondre absence de clé et erreur pour un profil sans token ; EvidenceBadge distingue observé/déclaré/jugé/manquant/périmé. ContextSection expose `selection` (accessible/included/excluded) et `availability` (retained/redacted/purged/opaque/missing) du manifeste, ainsi que la redaction ; l’état de transmission provient du ModelCallRecord (prepared/dispatch_attempted/completed/unknown/not_sent) et n’est jamais déduit du segment. DocumentRow indique service, identité, révision, fraîcheur et droits.

Les projections de setup indiquent draft/template/version/digest, résultat de validation exact, aperçu et ressources créées/réutilisées ; elles ne constituent pas une seconde machine d’état. La projection de run rend automation_mode distinct d’execution_mode. Replay/forbidden exclut tout modèle ; supervised/allowed_bounded présente ObservationReport, assertions/logs, tâche leader_decision et décision typée. Un champ safe_to_continue ou une justification LLM ne modifie pas un verdict du broker ou l’état inconnu d’un effet.

Les profils F012 rendent auth_mode et SecretRef facultative selon la qualification. TestProviderProfile porte sa phase et ses bornes ; GetProviderProfile/GetHealth restent des lectures d’observations et ne provoquent ni chargement ni génération. Les limites de local_only, fallback et placement sont celles de [local-models](../../specs/local-models.md).

La présentation d’un graphe est dérivée du même AST et des mêmes liens que la liste/arborescence TUI et les données CLI. Les ressources riches exposent contenu textuel/structure disponible, métadonnées, références et capacités de lecture/export. Les informations non disponibles sont signalées sans synthèse inventée ; les trois clients conservent l’accès aux actes sur la même ressource. Les coordonnées d’un canevas et les pixels ne sont pas un contrat de parité.

Le terminal s’appuie sur un flux contrôlé de F011, pas sur un shell lancé directement par un widget ou un plugin WebView. Les entrées deviennent actions autorisées avec propriétaire et fence ; le resize suit le même contrat. L’arrêt ne cible jamais seulement le numéro PID affiché. Les logs ordinaires restent texte inerte, distinct du flux d’émulation contrôlé.

## Événements locaux, reconnexion et compatibilité

Disconnected et ResyncRequired changent la présentation, sans créer d’événement métier fictif. Après fermeture/crash, négocier le protocole et rétablir snapshot/cursor, puis résoudre les commandes incertaines par leur command_id et leur authority_generation d’origine ; aucun nouvel ID ne masque une réponse perdue. Un refus StaleFence/STALE_AUTHORITY_GENERATION est présenté comme commande d’une lignée antérieure ; une nouvelle intention exige la décision de récupération applicable, un nouveau command_id et un lien causal explicite. Les reçus et erreurs de l’adaptateur restent distingués de l’acceptation ou de l’achèvement métier.

Une ancienne GUI face à protocole incompatible propose la mise à jour signée de l’application, appliquée sur commande de l’utilisateur hors protocole daemon (ADR-14, amendement R7), ou une lecture compatible ; elle ne transforme pas un champ inconnu en commande par défaut. L’API reste l’autorité des actions. Les capacités manquantes d’un fournisseur apparaissent dans tous les clients avec la même raison, pas comme succès d’une fonction inaccessible.

Le format d’enveloppe, les erreurs communes et la diffusion suivent [IPC](../../architecture/ipc-and-apis.md). Les identités et versions suivent le [modèle de stockage](../../architecture/storage-model.md). Les mécanismes Tauri v2 et leurs limites vérifiées le 16/11/2026 sont sourcés dans [la comparaison](../../research/technology-comparison.md) ; aucune signature de fonction Tauri n’est inventée par ce contrat.
