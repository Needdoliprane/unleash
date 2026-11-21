# F004 — Interfaces proposées

Les libellés d’erreur métier de cette feature sont des `reason_code` dans l’enveloppe canonique [IPC](../../architecture/ipc-and-apis.md), sous une classe commune telle que InvalidArgument, ContractViolation, CapabilityDenied, RevisionConflict ou OutcomeUnknown ; ils ne définissent pas un transport concurrent.
Le transport utilise l’enveloppe commune de [l’IPC](../../architecture/ipc-and-apis.md) et les noms du [catalogue client](../../architecture/client-operation-catalog.md). L’identité d’acteur est résolue côté serveur. Une mission peut avoir un OwnerRef user ou agent, sans repository ni agent actif ; sa clôture mechanical/human/owner_agent est explicite. Le mode forbidden exclut tout réveil modèle. Cet exemple illustre une commande interne, pas une API de fournisseur :

```json
{
  "protocol_version": "unleash.ipc.v2",
  "request_id": "00000000-0000-4000-8000-000000000017",
  "command_id": "00000000-0000-4000-8000-000000000018",
  "authority_generation": "00000000-0000-4000-8000-000000000100",
  "actor_ref": "agent-n2",
  "project_id": "00000000-0000-4000-8000-000000000019",
  "aggregate_id": "00000000-0000-4000-8000-000000000020",
  "expected_revision": 0,
  "command_type": "CreateAgent",
  "payload": {
    "level": "N",
    "organization_parent_id": "agent-n2",
    "mission_id": "mission-integration-1",
    "owner_agent_id": "agent-n2",
    "scope_ref": "scope-integration-1",
    "role_binding_ref": "binding-integrator-1",
    "objective": "Intégrer les deux commits identifiés et rapporter les conflits observés",
    "evidence_bundle_id": "evidence-integration-1"
  }
}
```

`CreateAgent` retourne le CommandReceipt commun, dont la référence de résultat expose agent/session/assignment et événements autorisés. `AssignTask` référence activation et état de file, sans affirmer qu’un worker a déjà démarré. `RequestAgentReassignment` référence nouvel owner, epoch et actions en réconciliation ; une révision de mission reste ReviseMission. ListAgents/GetAgent exposent les relations permettant de construire l’arbre ; ListInvocations/GetInvocation distinguent les tentatives. Aucun GetAgentTree ou ReassignMission parallèle n’est ajouté au wire.


`AssignMission` accepte le champ facultatif `scopes` dans le payload d’une mission à propriétaire humain. L’objet associe les noms de portée utilisés par le workflow à des sélecteurs `{"kind":"project","revision":1}`, avec la révision exacte du projet actif désigné par l’enveloppe. Le daemon dérive l’identité de ce projet et refuse une révision périmée avec `RevisionConflict`. La révision initiale de la mission, son manifeste immuable, le reçu et l’événement `MissionAssigned` sont liés par la même transaction. Le `result_ref` du reçu vaut `mission-scope:<digest>` et le champ `scope_ref` de l’événement contient le même digest SHA-256. Le client transmet ce digest à [`CreateRun`](../F005-workflow-engine/interfaces.md). L’appel sans `scopes` suit le parcours existant. Les contrôles de droits restent applicables à chaque opération.

`GetAgent` reçoit le payload fermé `{agent_id}`. Le `project_id` de l’enveloppe doit correspondre au projet persistant de l’agent. La valeur `null` cible un agent global et requiert une session autorisée sans restriction de projet. Un agent absent de la portée exacte renvoie `NotFoundOrHidden`. La réponse [agent_projection_response](../../schemas/ipc-query-response.schema.json) donne l’identité, le niveau, l’état, la révision et la session primaire ouverte. Le parent organisationnel et l’agent créateur valent chacun `null` si le lien est absent ou masqué par le périmètre autorisé. Une identité historique dépourvue de session ouverte a `session_id=null`. La vue et son watermark proviennent de la même transaction lectrice. L’affectation et les tentatives relèvent de leurs propres lectures.

`PauseMission`, `ResumeMission` et `CancelMission` ont pour portée la mission entière désignée par `aggregate_id`, y compris ses runs non terminaux, qui suivent les transitions pause, reprise et annulation de la [machine de Run](../../architecture/state-machines.md#run). Leur payload ne porte que `reason`, justification partageable ; aucun champ de portée partielle n’existe en V1. Une intervention plus étroite emploie son propre verbe (`PauseRun`, `RequestInterruption`), jamais un paramètre de ces commandes. Le reçu distingue l’arrêt demandé de l’état atteint.

Une décision AcceptStep ou RequestCorrection porte sujet, evidence bundle, révision examinée, raison et paramètres applicables ; son identifiant et ses effets sont référencés par le receipt. Une escalade conserve la décision et emploie les opérations de message/révision autorisées, sans verbe EscalateMission concurrent. `reason` est une justification partageable, pas une demande de raisonnement privé. Le contrat distingue commande appliquée, doublon identique, conflit et refus d’autorisation.

`SendHumanMessage` accepte destinataire autorisé, corps, pièces référencées et audience. Son receipt référence message, événement et visibilité N+2 applicable. GetConversation/GetMessage et les projections autorisées distinguent visibilité, besoin de traitement, invocation programmée et décision associée. L’absence de réveil n’est pas un message perdu ; un run forbidden n’en provoque pas.

## Préparation guidée commune — R3–R5

Le point d’entrée suit [l’expérience guidée](../../design/guided-automation-experience.md). Un utilisateur décrit un résultat ou choisit une procédure recommandée, sans créer préalablement des agents. Un template exact définit des valeurs par défaut personnalisables ; il ne rend ni dépôt, ni modèle, ni évaluateur qualité LLM universel obligatoire.

| Opération | Contrat proposé |
| --- | --- |
| CreateGuidedSetupDraft | Projet/scope ou contexte de projet à créer, template ID/version/digest, goal/critères de fin, sources autorisées, paramètres, mode, budgets, permissions/policies et provider seulement si nécessaire. Création à expected_revision=0 ; aucune génération implicite. |
| SaveGuidedSetupDraft | Draft et révision, document ou patch typé ; conserver provenance, valeurs par défaut et modifications, y compris une révision invalide. Aucun effet métier. |
| ValidateGuidedSetup | Draft exact et versions de catalogue/templates ; contrôler références, données, source de Run, besoins de rôles/agents, autorité, mode/modèles et localité. Rapport déterministe lié au digest ; préconditions non observées explicites. |
| ApplyGuidedSetup | Draft/révision/digest, validation/aperçu applicables et mandat ; préparer ou réutiliser goal/mission, versions de rôles/bindings et agents nécessaires. Préparer éventuellement le Run via le service commun CreateRun si sa source publiée existe ; sinon rendre les étapes restantes. Transaction locale atomique et idempotence commune, jamais StartRun implicite. |
| ListGuidedTemplates | Liste paginée autorisée, versions/digests, modes, entrées et prérequis ; recommandation déterministe sans appel modèle selon les six templates intégrés de Q-20 ([parcours guidé](../../design/guided-automation-experience.md)) : `template.reconcile-csv` en tête sur profil vierge ; sans fournisseur actif, les templates `forbidden` d’abord et les autres avec leur prérequis manquant ; sans dépôt enregistré, les templates de développement restent accessibles mais ne sont pas recommandés. |
| GetGuidedSetupDraft | Révision exacte, contenu, provenance, références, modifications et diagnostics conservés. |
| GetGuidedSetupValidation | Rapport de l’entrée exacte avec code/path/severity, dépendances, autorité et préconditions restantes. |
| GetSetupPreview | Projection déterministe, input refs, preview_digest et watermark : créations/réutilisations prévues, goal, sources réellement choisies, rôles/agents éventuels, source/mode, permissions, localité, budget et effets futurs. Aucune consultation distante, création de job ou probe pour compléter une inconnue. |

ApplyGuidedSetup conserve les identités résultantes dans son reçu : rejouer la même commande ne crée pas une deuxième mission ni une deuxième équipe. Un changement de draft ou de politique pertinent invalide l’aperçu et la validation applicables. Les créations locales ne donnent aucun grant supplémentaire ; un rôle peut uniquement demander le sous-ensemble autorisé. Les erreurs de révision ou d’autorité laissent le résultat non appliqué plutôt qu’un staffing partiel déclaré complet.

Le digest de preview est recalculé à l’application ; GetSetupPreview n’a pas à persister une mutation pour être fiable. Rôles et skills sont des versions publiées réutilisées ou des préparations conformes aux gates F007/F008 ; un draft non validé ne devient pas exécutable via Apply. Si la source ou le rôle nécessaire n’est pas prêt, le résultat indique la préparation restante sans lancer de modèle.

Un replay mécanique peut rester possédé par l’utilisateur et ne créer aucun agent. Le staffing d’un mode supervisé ou d’une préparation modèle ne matérialise que les rôles/agents réellement déclarés et autorisés. Apply ne lance ni apprentissage, ni génération, ni test de fournisseur, ni chargement de modèle, ni effet navigateur. Les commandes de préparation explicites restent distinctes ; l’exécution prête attend StartRun. Une autorisation préconfigurée peut couvrir ces opérations sans nouvelle confirmation à chaque étape.

## Décisions de supervision

Sous supervised/allowed_bounded, un bloc mécanique fournit ObservationReport (`safe_to_continue`, `effect_state`, `reason`, `assertion_refs`) et des références de logs. Un choice publié peut activer une tâche leader_decision aux checkpoints ou exceptions métier prévues. Ses sorties typées continue/reconcile/correct/escalate sont appliquées par le harness, sous contrat, scope, budget et journal d’effets. `correct` choisit une branche publiée ou demande un job de réparation séparé déjà mandaté ; aucun hotpatch.

Des logs rassurants ou un avis favorable ne remplacent pas les assertions obligatoires. Une tâche de collecte peut avoir produit un rapport valide alors que l’effet métier reste inconnu ; le broker interdit la continuation conflictuelle et la clôture requérant sa résolution. Les pannes sans branche prévue restent failed/blocked. Les erreurs passent dans Error commun ; aucune nouvelle state machine de supervision n’est créée.

Les erreurs suivent Error commun : code/reason_code, message, retry_class, details_ref, current_revision éventuelle et command_id. Agrégat/règle et détails expurgés sont accessibles par la référence autorisée. Une raison StaleEpoch sous StaleFence ne suggère pas de renvoyer le même effet avec une identité différente ; le client doit réconcilier avec l’owner actuel. Aucun client ne peut imposer directement status=succeeded à un agent ou à une mission.

## Décision de clôture de mission et de goal — R6

`RecordMissionDecision` et `RecordGoalDecision` sont des commandes discriminées par sujet, pas des setters de statut. Le payload contient `subject_id`, `criteria_revision`, `owner_ref`, `owner_epoch`, `verdict` (`accept`, `rework`, `escalate`, `fail`), `evidence_bundle_id`, `reviewed_revision` et `reason` partageable. L’enveloppe porte expected_revision et l’acteur authentifié. Tous les champs doivent correspondre à l’état courant ; le serveur dérive l’autorité, vérifie la politique de clôture et refuse une ancienne affectation ou un bundle inapplicable avant toute mutation.

Sous `human`, le décideur est l’utilisateur ou son délégataire humain explicitement autorisé ; sous `owner_agent`, c’est l’owner agent courant avec un mandat de décision. Un override humain doit être autorisé et journalisé comme tel. Sous `mechanical`, le reducer applique les critères observables et enregistre la décision sous principal harness, sans appel modèle ni commande cliente obligatoire. Une mission forbidden ne peut configurer une clôture qui nécessite un owner_agent : validation refusée jusqu’à choix mechanical/human. Une décision humaine ne déclare pas vrais des critères mécaniques faux/inconnus ; un critère de jugement est satisfait par l’avis attribué du décideur prévu, sans prétendre prouver sa qualité.

`accept` mène mission à succeeded ou goal à achieved seulement si critères requis satisfaits, travaux pertinents quiescents et effets nécessaires connus. `rework` conserve une mission active (ou blocked si une barrière persiste), et un goal active/blocked ; un nouveau run reste une commande distincte. `escalate` bloque avec destinataire/raison visibles ; aucune invocation récursive automatique. `fail` clôt une mission failed après quiescence, ou laisse une intention de clôture bloquée ; un goal devient blocked avec décision négative, car sa machine ne possède pas d’état failed. Aucun run terminal n’est rouvert. Pause/cancel restent leurs propres opérations.

Transaction unique : décision, références exactes, nouvelle révision/état, MissionDecisionRecorded ou GoalDecisionRecorded, outbox de notifications et CommandReceipt. Redelivery identique restitue le même reçu. Événement de clôture émis uniquement si la transition a réellement eu lieu. Erreurs : RevisionConflict, StaleFence/OWNER_EPOCH_CHANGED (owner ou epoch d’affectation de mission/goal périmé, distinct de l’epoch d’exécution STALE_EPOCH), ContractViolation/DONE_NOT_MET, OutcomeUnknown/UNRESOLVED_EFFECT, CapabilityDenied/DECISION_AUTHORITY_REQUIRED. Les deux clients présentent les mêmes critères et décisions ; aucun écran ne peut contourner cette commande.

## Retrouver et appliquer une préparation — R6

GetOverview comprend une collection `guided_setup_drafts` paginée sous identité utilisateur, y compris drafts dont le projet n’est pas encore créé : draft_id, nom, saved_revision, saved_at, template_ref, état draft/applied, application_ref éventuelle. Filtre/tri/cursor et ACL suivent le catalogue commun ; un client neuf peut retrouver la préparation sans ID saisi ni cache local. GetGuidedSetupDraft restitue le contenu et les diagnostics à la révision exacte.

Un draft appliqué est figé. `ApplyGuidedSetup` possède en plus de l’idempotence transport une unicité métier `draft_id → application_id` persistée dans la transaction de matérialisation. Deux commandes différentes visant le même draft/révision/digest applicable rendent le même application_id et les mêmes goal/mission/run préparés ; chacune a son reçu, le second lie le résultat existant sans recréation. Une tentative sur un digest incompatible retourne `ContractViolation/APPLIED_DRAFT_CONFLICT` (nom de domaine AppliedDraftConflict, retry_class=never) avec référence de l’application. SaveGuidedSetupDraft sur draft appliqué est refusé `ContractViolation/APPLIED_DRAFT_IMMUTABLE` (nom de domaine AppliedDraftImmutable, retry_class=never). Pour un nouveau travail, CreateGuidedSetupDraft peut copier une source exacte en un nouveau draft clairement distinct.

GetGuidedSetupDraft/GetOverview affichent applied et les étapes restantes. Le bouton devient « Ouvrir le travail préparé ». La commande Apply ne lance toujours ni StartRun, ni modèle, ni effet métier. Une relance de procédure déjà publiée peut créer un nouveau Run explicite sans dupliquer la préparation initiale.

L’enveloppe mutante v2 conserve authority_generation de sa création durable dans toute retransmission. Une reconnexion ne réécrit jamais cette valeur ; après restauration, une ancienne commande est refusée avant idempotence. L’UI rend le conflit de lignée visible et demande une réconciliation distincte, pas un changement automatique de command_id. Le reçu courant conserve son schéma distinct sans champ protocol_version supplémentaire. Un reçu d’une ancienne lignée restitué par GetCommandReceipt ou une inspection historique est signalé comme historique selon [l’IPC](../../architecture/ipc-and-apis.md) ; il ne vaut ni acceptation, ni résolution de timeout dans la lignée active, ni autorisation de retry.
