# Matrice de parité GUI / TUI / CLI

Statut : **contrat de conception, aucune ligne n’est encore une preuve de produit exécuté**. Révision : 2026-11-16. Références : [dix espaces GUI](gui-screen-specifications.md), [architecture d’information](gui-information-architecture.md), [design terminal](cli-tui-design.md), [F014](../features/F014-gui/README.md) et [F015](../features/F015-cli-tui/README.md).

## Règle de parité

La GUI et la TUI offrent les mêmes fonctions métier, informations, décisions, scopes, permissions, validations, opérations, erreurs et preuves. Leur navigation et leur rendu s’adaptent au médium. Aucun écran ni aucune fonction n’est réservé à la GUI. La CLI non interactive expose les mêmes opérations par arguments et documents/flux structurés ; la reproduction d’un geste de mise en page n’est pas exigée.

Les dix espaces ci-dessous comprennent **58 groupes d’opérations** : les 50 groupes R2 sont conservés, avec PAR-AUTOMATION-01 à 06, PAR-ONBOARDING-01 et PAR-LOCAL-01. Chaque verbe d’un groupe produit un sous-cas de recette ; une réussite partielle du groupe ne vaut pas parité. « Oui » signifie obligatoire dans la V1, pas déjà implémenté. Les noms CLI sont une grammaire proposée, raccordée au [catalogue commun](../architecture/client-operation-catalog.md) et aux [interfaces F015](../features/F015-cli-tui/interfaces.md). Aucun espace ou moteur métier supplémentaire n’est ajouté.

Toutes les lignes attendent EV-F015-001 avec même fixture, identité et mandat, puis comparaison des commandes, reçus, effets, révisions, erreurs et preuves. La dernière colonne ajoute les vérifications de parcours ou de rendu nécessaires. Les références de preuves sont des artefacts attendus, non des liens vers des résultats inexistants.

## UI-OVERVIEW — Vue d’ensemble

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-OVERVIEW-01 | Créer/réviser un projet ; lister/sélectionner projets, goals et missions ; lire états, budgets, slots et prochain événement utile. | Oui | Tableau et synthèse filtrables, détails et labels d’état. | Oui : project list/show/create/revise, overview, goal/mission list/show. | EV-F015-009, EV-F015-015 |
| PAR-OVERVIEW-02 | Créer/réviser un goal, objectif, périmètre, priorités et conditions de fin ; enregistrer sa décision de clôture selon mandat. | Oui | Formulaire complet, validation et diff avant soumission. | Oui : goal create/revise/decide avec document/flags (RecordGoalDecision : verdict, bundle, révision examinée). | EV-F015-004, EV-F015-006 |
| PAR-OVERVIEW-03 | Inspecter/réviser/affecter une mission, dépendances, workflow et responsable ; enregistrer sa décision de clôture. | Oui | Fiche mission, sections navigables et sélecteurs typés. | Oui : mission show/revise/decide (RecordMissionDecision) et opérations d’affectation communes. | EV-F015-006, EV-F015-010 |
| PAR-OVERVIEW-04 | Demander pause/reprise/annulation ; distinguer demande, état atteint, résultat incertain et terminé. | Oui | Action à cible visible, reçu et suivi. | Oui : mission pause/resume/cancel, operation/receipt show. | EV-F015-003, EV-F015-005 |
| PAR-OVERVIEW-05 | Rechercher/filtrer, ouvrir blocage/décision/alerte et inspecter une opération en vol. | Oui | Recherche, palette et alertes hors filtre accessibles. | Oui : alert list, recherches filtrées, decision/operation/receipt show. | EV-F015-009, EV-F015-016 |
| PAR-ONBOARDING-01 | Retrouver les préparations sauvegardées, lister templates, créer/sauvegarder un GuidedSetupDraft, valider, lire rapport/aperçu et appliquer sous mandat ; goal/sources/paramètres/mode/provider si nécessaire/permissions ; préparer le travail sans effet avant StartRun. | Oui : parcours guidé dans Overview, détails avancés accessibles. | Même formulaire séquentiel, templates/version, aide, paramètres et aperçu ; aucun agent, dépôt ou jargon préalable. | Oui : setup template list, setup draft create/show/save/validate, setup validation show, setup preview/apply. | EV-F004-012, EV-F014-015, EV-F015-017 |

## UI-WORKSPACES — Espaces de travail

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-WORKSPACES-01 | Lister/inspecter features, lots, dépendances et responsables. | Oui | Arbre/tableau et liens vers mission/agents. | Oui : workspace list/show, dependency list/show. | EV-F015-009, EV-F015-015 |
| PAR-WORKSPACES-02 | Inspecter branches, worktrees, snapshots, disponibilité locale et provenance ; demander le retrait autorisé d’un worktree. | Oui | Liste structurée, état dirty/occupé/non intégré explicite ; retrait refusé motivé. | Oui : worktree list/show/retire (RetireWorktree), workspace show. | EV-F015-011 |
| PAR-WORKSPACES-03 | Lire/rechercher un diff intégral et comparer les snapshots applicables. | Oui | Diff unifié ou panneaux commutables, recherche et pagination. | Oui : workspace diff, export textuel/structuré. | EV-F015-010, EV-F015-015 |
| PAR-WORKSPACES-04 | Demander/préparer, inspecter et publier une intégration autorisée ; voir conflits, cible checkout bloquée (`blocked_target_checkout`) avec ses remèdes, et preuves. | Oui | Formulaire cible/base, progression, conflit, remèdes de cible checkout et reçus. | Oui : integration request/show/publish ; integration show rend `blocked_target_checkout`, la branche cible gérée proposée, les instructions exactes de libération du checkout et la réobservation ; aucun verbe ne transfère un checkout (Q-10). | EV-F015-003, EV-F015-004 |
| PAR-WORKSPACES-05 | Demander coordination/dépendance ; décider réutilisation, attente ou nouvelle création avec motif. | Oui | Candidats, état d’intégration et décision liée au receipt. | Oui : dependency request/show, discovery decide. | EV-F015-011, EV-F015-016 |

## UI-AGENTS — Agents et contexte

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-AGENTS-01 | Naviguer la hiérarchie N+2/N+1/N, rôle, skills et scopes ; créer un agent selon mandat. | Oui | Arbre repliable/recherchable, inspecteur et provenance ; formulaire de création. | Oui : agent list/show avec relations parent/enfant, agent create (CreateAgent). | EV-F015-009, EV-F015-015 |
| PAR-AGENTS-02 | Distinguer agent logique, session, invocation/tentative, dormance, file, attente outil et erreur ; affecter une tâche selon mandat. | Oui | Labels distincts, historique et filtres ; affectation à cible visible. | Oui : agent/invocation list/show, task assign (AssignTask). | EV-F015-016 |
| PAR-AGENTS-03 | Lire modèle effectif, contexte autorisé/sélectionné/transmis, appels, compactions et sources. | Oui | Manifest et sections navigables ; compare/export et limites. | Oui : context show/diff/export/compact selon mandat. | EV-F015-011 |
| PAR-AGENTS-04 | Envoyer un message, ouvrir le thread et inspecter sa visibilité N+2. | Oui | Conversation intégrée, reçus et visibilité distinguée de lecture LLM. | Oui : agent message, conversation show, message show. | EV-F015-003, EV-F015-006 |
| PAR-AGENTS-05 | Réaffecter selon mandat, demander pause/reprise ou interruption d’invocation ; inspecter processus associé. | Oui | Commandes séparées avec cible et conséquence exactes. | Oui : agent reassign/interrupt, mission pause/resume, process show. | EV-F015-004, EV-F015-007 |

## UI-CONVERSATIONS — Conversations

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-CONVERSATIONS-01 | Lister/lire/rechercher threads, participants et historique autorisé. | Oui | Threads paginés, texte intégral et recherche. | Oui : conversation list/show/search. | EV-F015-006, EV-F015-015 |
| PAR-CONVERSATIONS-02 | Rédiger/envoyer un message multiligne à N+1/N+2 et vérifier sa visibilité. | Oui | Composeur intégré, collage contrôlé et action d’envoi. | Oui : message send depuis fichier/stdin ; message show. | EV-F015-003, EV-F015-006 |
| PAR-CONVERSATIONS-03 | Citer document, snapshot, preuve, rapport ou décision ; ouvrir leur provenance. | Oui | Sélecteur de référence et inspecteur avec retour au thread. | Oui : message send avec refs, evidence/resource/report show. | EV-F015-011 |
| PAR-CONVERSATIONS-04 | Proposer/inspecter une révision de mission et sa décision, sans confondre message et mutation. | Oui | Proposition, diff, validation et reçu de révision distinct. | Oui : mission revise, decision list/show. | EV-F015-004, EV-F015-010 |
| PAR-CONVERSATIONS-05 | Conserver un brouillon, retrouver un message accepté après coupure et traiter un conflit. | Oui | Brouillon local marqué, réconciliation et reprise explicite. | Oui : fichiers de brouillon, receipt/message show et replay idempotent. | EV-F015-003, EV-F015-016 |

## UI-WORKFLOWS — Workflows

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-WORKFLOWS-01 | Rechercher/lire catalogue, versions, contenu intégral, usages et diffs. | Oui | Catalogue, source/AST, arbre et comparaison. | Oui : workflow list/show/diff, version usages. | EV-F015-010 |
| PAR-WORKFLOWS-02 | Créer un draft depuis intention ou version ; éditer texte, étapes, ordre, branches, boucles, bindings, contrats et done ; sauvegarder. Relire le texte et la filiation d’un brouillon dérivé. | Oui | Éditeur WORKFLOW intégré et formulaires ; déplacement clavier. | Oui : workflow draft/save depuis document structuré. | EV-F015-006, EV-F015-010 |
| PAR-WORKFLOWS-03 | Valider/normaliser, ouvrir diagnostic localisé et corriger un draft invalide. | Oui | Diagnostics par chemin AST et navigation vers champ/source. | Oui : workflow validate/normalize et diagnostics structurés. | EV-F015-010 |
| PAR-WORKFLOWS-04 | Simuler et inspecter trajectoire, événements fictifs, bornes et raisons de blocage. | Oui | Timeline/arbre de simulation marqué, détail des conditions. | Oui : workflow simulate et résultat structuré. | EV-F015-010, EV-F015-011 |
| PAR-WORKFLOWS-05 | Publier un workflow. Créer, démarrer, mettre en pause, reprendre ou annuler un run. Lister les runs du projet, ouvrir leur détail, inspecter et adopter une version éligible, lier un rôle autorisé, accepter une étape ou demander une correction. | Oui | Actions complètes, version engagée visible, frontières et refus explicites. | Oui : workflow publish, run list/show et commandes de cycle de vie, binding show/set, step accept/request-correction. | EV-F015-003, EV-F015-004, EV-F015-010 |
| PAR-AUTOMATION-01 | Rechercher/lire les procédures autorisées, modes, versions/digests, contrats, qualifications, dépendances, usages et diffs. | Oui : catalogue Procédures au sein de Workflows. | Catalogue filtrable, manifeste complet, source/plan, versions et diff. | Oui : automation list/search/show/diff, version usages ; SearchAutomations/GetAutomationVersion et lectures communes. | EV-F019-004, EV-F019-016, EV-F015-017 |
| PAR-AUTOMATION-02 | Créer/sauvegarder un draft, apprendre sous mandat, éditer paramètres/étapes/locators/contrats et conserver un draft invalide ; préparation distincte de l’exécution. | Oui : liste d’étapes guidée et éditeur expert. | Éditeurs et formulaires intégrés, provenance, limites et coûts de conception. | Oui : automation draft create/show/save, automation learn ; CreateAutomationDraft/SaveAutomationDraft/LearnAutomation. | EV-F019-001, EV-F019-003, EV-F014-015, EV-F015-017 |
| PAR-AUTOMATION-03 | Importer CSV/données, régler parsing/mapping/identités/bornes, inspecter la révision immuable et valider contre version/mode/paramètres. | Oui : tableau et diagnostics par ligne/colonne. | Même tableau navigable, mapping et formulaire intégré de correction vers une nouvelle révision ; aucune conversion LLM cachée. | Oui : automation data import/show/validate ; ImportAutomationData/GetAutomationDataRevision/ValidateAutomationData. | EV-F019-005, EV-F019-016, EV-F015-017 |
| PAR-AUTOMATION-04 | Valider/normaliser, évaluer avec cible fixture/test/live annoncée, lire preuves, publier/déprécier/révoquer et inspecter l’impact ; aucun juge qualité universel ajouté. | Oui : diagnostics, évaluation, diff, gates et reçus. | Même information, commandes et refus ; effets réels d’évaluation annoncés. | Oui : automation validate/evaluate/evaluation show/publish/deprecate/revoke ; commandes F019 exactes et GetAutomationEvaluation. | EV-F019-002, EV-F019-018, EV-F014-016, EV-F015-017 |

Pour la lecture d’un brouillon dérivé de PAR-WORKFLOWS-02, chaque client compose `GetWorkflowDraft` et `ReadArtifact` selon le [contrat de révision F006](../features/F006-workflow-compiler/interfaces.md#brouillon-dérivé-dune-version-publiée).

## UI-SKILLS — Skill Studio et rôles

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-SKILLS-01 | Rechercher/lire skills et rôles, namespaces, versions, provenance, contenu intégral et usages. | Oui | Deux catalogues, résolution explicite et inspecteur complet. | Oui : skill/role list/search/show, version usages. | EV-F015-009, EV-F015-011 |
| PAR-SKILLS-02 | Effectuer discovery, enregistrer la décision reuse/create et sa stratégie explicite (reuse : use_existing/coordinate/wait/adapt ; create : copy/adapt/new) ; créer/éditer/sauvegarder SKILL, instructions, schémas et outils. | Oui | Éditeur intégré, candidat et motif, brouillon conservé. | Oui : knowledge discover, discovery decide/attach (AttachDiscoveryReceipt), skill draft/save. | EV-F015-006, EV-F015-010 |
| PAR-SKILLS-03 | Créer/composer/éditer un rôle : skills versionnés, responsabilités, modèle, capacités, scope et contexte. | Oui | Formulaires et composition de références précises. | Oui : role draft/compose/save depuis document. | EV-F015-010 |
| PAR-SKILLS-04 | Valider contrats et permissions, lancer/inspecter évaluations applicables et comparer versions. | Oui | Diagnostics, résultats qualifiés et diff textuel/structuré. | Oui : skill validate/evaluate/diff, role validate/diff. | EV-F015-004, EV-F015-010 |
| PAR-SKILLS-05 | Publier/déprécier/révoquer selon mandat ; lire historique, export et impact sur usages. | Oui | Actions versionnées, revue de diff, reçus et usages. | Oui : skill/role publish/deprecate/revoke/export, version usages. | EV-F015-003, EV-F015-010 |

## UI-KNOWLEDGE — Connaissance et documents

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-KNOWLEDGE-01 | Rechercher/discover ressources, filtrer par type/projet/fraîcheur et lire couverture de recherche. | Oui | Résultats typés, recherche/filtre et état stale explicite. | Oui : knowledge search/discover. | EV-F015-009, EV-F015-015 |
| PAR-KNOWLEDGE-02 | Lire symboles, composants, tests, dépendances et travail non intégré ; ouvrir snapshot et provenance ; déclarer une intention de travail. | Oui | Arbre/tableau, contenu paginé, branche et disponibilité explicites. | Oui : knowledge search, intent declare (DeclareWorkIntent) et lectures de ressources/snapshots. | EV-F015-011 |
| PAR-KNOWLEDGE-03 | Explorer/rechercher documents connectés ; afficher métadonnées, preview, contenu autorisé et source/export. | Oui | Browser de ressources, texte/blocs/tableaux et limites de média. | Oui : resource browse/search/show/preview/export. | EV-F015-011, EV-F015-012 |
| PAR-KNOWLEDGE-04 | Pin/unpin, affecter document, définir Scope/Watch, rafraîchir, retirer affectation et inspecter permissions/fraîcheur. | Oui | Formulaires complets, racines/filtres/bornes et suivi. | Oui : connector pin/unpin/scope/watch/refresh, assignment list/remove. | EV-F015-004, EV-F015-012 |
| PAR-KNOWLEDGE-05 | Enregistrer la décision reuse/create avec stratégie explicite, lier et relire DiscoveryReceipt, demander dépendance et comparer contexte source. | Oui | Décision motivée, liens de preuve et inspecteur. | Oui : discovery show/decide/attach, dependency request, context show/diff. | EV-F015-011 |

## UI-ACTIVITY — Activité, processus et terminaux

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-ACTIVITY-01 | Lire/rechercher/filtrer timeline par projet, agent, mission, causalité et sévérité. | Oui | Tableau/timeline filtrable et inspecteur causal. | Oui : events search/watch avec filtres. | EV-F015-009, EV-F015-015 |
| PAR-ACTIVITY-02 | Suivre flux, reprendre par cursor, repérer trou/séquence manquante et ouvrir commande/reçu. | Oui | Follow/pause d’affichage, rattrapage et état de fraîcheur. | Oui : events watch, operation/receipt show. | EV-F015-002, EV-F015-016 |
| PAR-ACTIVITY-03 | Lister/inspecter processus, générations, propriétaires, worktrees, ports et logs ; rechercher/suivre/exporter logs. | Oui | Registre, logs paginés/recherchables et follow borné. | Oui : process discover (DiscoverTerminals)/list/show/log avec offset/follow, sortie exportable. | EV-F015-007, EV-F015-015 |
| PAR-ACTIVITY-04 | Lire PTY, acquérir contrôle/écrire/redimensionner avec lease, libérer/détacher et arrêter la génération autorisée. | Oui | Inspecteur puis attachement explicite, entrée isolée et escape visible. | Oui : process acquire (AcquireTerminal non interactif, nouvelle demande ou génération enregistrée ; rend lease_id/fence/lease_expires_at)/log/input/resize/release/stop/reconcile ; input/resize/release exigent lease_id et fence, aucune commande n’acquiert ni ne libère un lease implicitement ; attach interactif facultatif, payload d’entrée scriptable autorisé. | EV-F015-005, EV-F015-007, EV-F015-014 |
| PAR-ACTIVITY-05 | Ouvrir événement Git, audit, preuve/source ; inspecter masquage et incident de ressource. | Oui | Inspecteur lié, contenu/raison de manque et retour à timeline. | Oui : événements, integration show, evidence/resource show. | EV-F015-004, EV-F015-011 |
| PAR-AUTOMATION-05 | Créer un Run source automation_version ; démarrer/pause/reprise/annuler, inspecter données/mode/politique, checkpoints, supervision, lignes/effets, limites et réconciliation. | Oui : suivi commun du Run, blocs mécaniques et leader_decision seulement aux points publiés. | Même suivi et contrôles ; replay forbidden affiché sans agent/LLM ; rapport d’observation, assertions et logs distincts. | Oui : run create/start/show/pause/resume/cancel, recovery reconcile/resolve ; CreateRun et lifecycle commun, sans CreateAutomationRun. | EV-F019-006, EV-F019-007, EV-F019-011, EV-F019-012, EV-F015-017 |

## UI-REPORTS — Rapports et historique

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-REPORTS-01 | Lister/filtrer les rapports N, N+1, N+2 et ouvrir résultat lié au goal/mission. | Oui | Catalogue paginé, filtre et liens hiérarchiques. | Oui : report list/show. | EV-F015-009, EV-F015-015 |
| PAR-REPORTS-02 | Lire contenu intégral, OK/KO, périmètre, déclarations non confirmées et incidents. | Oui | Texte/tableaux navigables avec statut explicite de preuve. | Oui : report show en humain/JSON. | EV-F015-011 |
| PAR-REPORTS-03 | Comparer rapports, versions, snapshots et décisions dans l’historique. | Oui | Diff et panneaux commutables, liens de provenance. | Oui : report diff et lectures de snapshots/décisions. | EV-F015-010, EV-F015-011 |
| PAR-REPORTS-04 | Ouvrir preuve/source, contrôler applicabilité et constater contenu périmé, inaccessible ou supprimé. | Oui | Inspecteur avec hash/version/raison ; permissions conservées. | Oui : evidence show/export, resource show. | EV-F015-004, EV-F015-011 |
| PAR-REPORTS-05 | Demander rapport puis exporter Markdown/JSON et paquet de références autorisé. | Oui | Formulaire, progression, choix d’export et reçu. | Oui : report request/export. | EV-F015-002, EV-F015-003 |
| PAR-AUTOMATION-06 | Lire/exporter résultats par ligne, effets et appels modèles ; inspecter dérive, demander réparation séparée mandatée, comparer nouvelle version et préparer une succession après réconciliation. | Oui : résultat, preuves, incident et diff liés au run d’origine. | Même contenu et actions ; aucun résultat inconnu transformé en succès par les logs ou le leader. | Oui : run show, report request/show/export, evidence show, automation repair/diff ; RepairAutomation et opérations communes. | EV-F019-013, EV-F019-016, EV-F014-016, EV-F015-017 |

## UI-SETTINGS — Configuration et maintenance

| Repère | Opérations métier requises | GUI | TUI | CLI non interactive proposée | Preuve complémentaire |
| --- | --- | --- | --- | --- | --- |
| PAR-SETTINGS-01 | Gérer profil, fournisseurs/modèles, références de clés ; ajouter/remplacer/révoquer et tester connexion. | Oui | Formulaires avec secrets masqués, test et erreur explicite ; import d’un catalogue de modèles et confirmation de tarif par SetProviderProfile (Q-14). | Oui : provider list/show/set/test, dont `provider set --from-catalog <fichier>`, secret set/revoke (RevokeSecretReference) via canal protégé. | EV-F015-004, EV-F015-012 |
| PAR-SETTINGS-02 | Configurer connecteurs ; initier/suivre/annuler/reprendre consentement ou token qualifié, tester et révoquer. | Oui | Settings lance navigateur externe et suit le flux ; aucune GUI Unleash requise. | Oui : connector auth start/status/cancel/retry, test, connector revoke (DisconnectAccount, révocation locale, distincte de secret revoke) ; consentement externe humain si requis. | EV-F015-008, EV-F015-012 |
| PAR-SETTINGS-03 | Lire/configurer scopes, politiques, mandats, AI-Guard ; suspendre, reprendre ou révoquer un sujet ; approuver ou refuser une action en attente d’approbation AI-Guard ; enregistrer une décision de déclassification autorisée ; voir garanties présentes/manquantes et révocation. | Oui | Formulaires, état de santé, validation et diff selon mandat. | Oui : policy show/set, guard show/configure, subject suspend/resume/revoke (SuspendSubject/ResumeSubject/RevokeSubject), action approve/deny (ResolveActionApproval), declassification record (RecordDeclassificationDecision). | EV-F015-004, EV-F015-012 |
| PAR-SETTINGS-04 | Lire/ajuster limites, réserve de contrôle (mode, unités, rafale), budgets, quotas, routage/fallback et préférences de présentation autorisées ; lire les métriques d’attente et les capacités du client. | Oui | Champs validés et aide ; préférences terminal adaptées. | Oui : settings show/set, provider set, capabilities show (GetClientCapabilities) ; rendu par flags/préférences. | EV-F015-012, EV-F015-013 |
| PAR-SETTINGS-05 | Gérer la rétention (politique `unleash.retention.default.v1` affichée et modifiable, impact visible avant toute réduction, sauvegardes gérées affectées par une purge listées), sauvegarde/validation/restauration, rebuild index ; inspecter santé/récupération, entrées `dead_letter`, profils d’analyse et runtimes navigateur préparés ; importer explicitement l’environnement du shell et lancer une installation assistée d’analyseur sous confirmation ; lire l’état du daemon et demander un arrêt drain/stop autorisé ; demander réconciliation, décider une disposition autorisée et reprendre une invocation récupérable. | Oui | Actions communes avec portée, progression et effets annoncés. | Oui : backup request/show/validate/restore, recovery show/reconcile/resolve/resume, index rebuild, history purge, settings show/set pour la rétention, daemon status/stop (GetDaemonStatus/StopDaemon), doctor, doctor analyzers (installation assistée par AcquireTerminal catalogué). | EV-F015-003, EV-F015-008, EV-F015-016 |
| PAR-LOCAL-01 | Configurer LM Studio/Ollama/llama.cpp et profils compatibles, endpoint/dialecte/auth facultative, local_only, capacités/placement/santé et ressources ; test phasé explicite et fallback sous mêmes contraintes. | Oui : formulaire guidé ou expert, métadonnées et preuves de placement/fraîcheur. | Mêmes champs, SecretRef seulement si requise, diagnostics et phase de test ; aucun serveur chargé à l’ouverture. | Oui : provider list/show/set/test avec phase et bornes, settings/policy ; lectures de santé passives. | EV-F014-016, EV-F015-017 ; preuves locales F012 |

## Adaptations de rendu autorisées

| Fonction | GUI | Adaptation terminal conforme |
| --- | --- | --- |
| Graphe de workflow / hiérarchie | Nœuds et liens visuels | Arbre/liste structurée, identifiants et dépendances ; édition de chaque étape/condition/binding et mêmes diagnostics. |
| Diff large | Colonnes côte à côte | Diff unifié ou panneaux alternés ; texte intégral, champs, recherche et navigation conservés. |
| Contexte/document | Mise en page enrichie | Sections, tableaux, blocs de texte, métadonnées et provenance ; accès à source/export autorisé depuis TUI. |
| Média/binaire | Preview graphique | Métadonnées et texte disponible, limite de preview explicite ; source/export/outil externe déclenchable depuis TUI. L’image dans le terminal est facultative. |
| Drag-and-drop | Déplacement à la souris | Commande clavier/formulaire de déplacement avec position cible et même mutation. |
| Authentification fournisseur | Navigateur externe depuis GUI | Même service lancé depuis TUI ou CLI, état suivi au terminal. Consentement externe ne signifie pas dépendance à la GUI. |
| Grande densité d’information | Panneaux simultanés | Panneaux commutables en mode compact, sans disparition de fonction ou donnée. |
| Mise en page / préférences | Dimensions, zoom, thème | Largeur terminal, sans couleur, ASCII, focus et lecture linéaire ; mêmes opérations métier. |
| Démarrage guidé / procédure | Formulaire progressif, preview et tableau de données | Même succession, valeurs par défaut personnalisables, validation et aperçu complets ; détails toujours accessibles et aucune confirmation supplémentaire due au terminal. |
| Modèles locaux / supervision | Fiche de profil, blocs et checkpoints | Même localité, capacités, santé, coûts et décisions ; lire un écran ne lance ni probe ni manager. |
| Archives (Q-12) | Vues masquant par défaut missions et runs terminaux depuis plus de 30 jours, agents `retired` et agents dormants sans invocation depuis 90 jours | Même filtre par défaut, identique et réversible ; geste de présentation local qui ne change aucun état backend. L’archive hors ligne d’un projet reste un ExportBundle. |

Les différences de pixels, d’animations, de police ou de geste ne sont pas des écarts fonctionnels. Une information inaccessible, un éditeur absent, une configuration réalisable uniquement en GUI ou une autorité divergente sont des écarts bloquants. Ouvrir une source externe est un complément : cela ne peut remplacer l’inspection des manifests, métadonnées, versions, décisions et preuves que le produit doit rendre lui-même.

## Vérification et évolution

REQ-CLI-001 / AC-F015-001 / T-F015-001 / EV-F015-001 couvrent les 58 lignes ; REQ-UI-012 relie la même exigence côté GUI. Les critères F015-009 à 016 conservent finition, éditeurs, contexte, Settings, capacités, lifecycle, ressources et reprise ; F015-017 et F014-015/016 couvrent le guidage, les automatisations et les modèles locaux. F004-011/012 et F019 portent leurs contrats métier. Le protocole et les features propriétaires restent l’autorité sémantique ; aucune preuve n’est annoncée exécutée ici.

Pour chaque ligne PAR, consigner les sous-opérations, l’entrée, l’identité/scope, la révision initiale, la commande backend, le reçu, l’état final et les preuves. Vérifier un succès et les refus/conflits applicables. Les gestes GUI/TUI peuvent différer, les effets attendus ne changent pas. L’adaptation non interactive d’un geste de présentation est documentée comme telle ; elle ne dispense pas d’exposer sa lecture ou mutation métier.

La couverture est vérifiée par le test CI généré depuis [catalog/operations.json](../catalog/operations.json) ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)). Les mises à jour signées passent par l’updater de l’application et du paquet CLI, hors protocole daemon (Q-16), et la planification calendaire de Runs est exclue de la V1 (Q-21) : ni l’une ni l’autre ne forme d’opération ou de groupe PAR. Toute future fonction GUI ajoute sa couverture terminal dans cette matrice avant d’être déclarée livrée. Un service externe non qualifié reste un gate commun aux trois clients. Aucune ligne ne peut être retirée au motif que la TUI serait « uniquement pour superviser ».
