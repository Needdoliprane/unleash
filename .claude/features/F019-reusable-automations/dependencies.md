# Dépendances F019

| Dépendance | Contrat réutilisé |
|---|---|
| [F001 — Daemon](../F001-core-daemon/README.md) | Autorité locale unique, identité du client, commandes/révisions/receipts, lifecycle et capacités négociées. |
| [F002 — Persistance](../F002-persistence-events/README.md) | Version immuable, données adressées par contenu, état/journal/outbox atomiques, ledger et checkpoints. |
| [F003 — Runtime agent](../F003-agent-runtime/README.md) | Invocations de conception `learn`/`repair` et tâches de supervision déclarées sous budgets/admission, sessions de provenance ; aucune invocation agente requise pour le replay mécanique. |
| [F004 — Hiérarchie](../F004-hierarchical-orchestration/README.md) | Rôles et profils métier réutilisés, décisions de supervision typées ; notifications déterministes aux managers sans réveil LLM automatique d'un run interdit. |
| [F005 — Moteur](../F005-workflow-engine/README.md) | Un seul `WorkflowReducer`, activation, séquence/choix/boucles/jointures, done, timers et annulation quiescente. |
| [F006 — Compilateur](../F006-workflow-compiler/README.md) | Profils v1.1/v1.2 d'actions fermés (v1.2 requis pour for_each), typage, références, analyse des bornes et compilation reproductible du même candidat. |
| [F007 — Registres](../F007-skill-studio-registry/README.md) | Discovery, drafts, publication/diffs/usages, namespaces et révocation, appliqués au type AutomationArtifact. |
| [F008 — Contrats](../F008-contract-engine/README.md) | Profils exacts, schémas, validateurs de catalogue sans modèle, distinctions observation/claim/judgment et traitement de l'inconnu. |
| [F009 — Connaissances](../F009-knowledge-engine/README.md) | Recherche autorisée d'artefacts et preuves ; aucune génération d'embedding déclenchée par le replay. Un index dérivé ne remplace pas l'artefact publié. |
| [F011 — Processus](../F011-terminal-pool/README.md) | Drivers et scripts externes supervisés, dont chromedriver du pilote de référence sur port loopback éphémère et l'exécution suivie qui prépare le runtime navigateur ; identité OS, descendants, bornes, logs, leases et arrêt vérifié. |
| [F012 — Routeur modèle](../F012-model-api-router/README.md) | Refus `forbidden` avant dispatch, attribution causale, compteur d'essais refusés séparé des appels réellement engagés et des coûts inconnus. |
| [F013 — Connecteurs](../F013-document-connectors/README.md) | Snapshots CSV/documents autorisés, provenance et ACL ; modification d'une source ne modifie pas une donnée épinglée. |
| [F014 — GUI](../F014-gui/README.md) / [F015 — Terminal](../F015-cli-tui/README.md) | Même catalogue de commandes et projections en GUI/TUI/CLI ; formulaire de paramètres, résultats par ligne et réparation accessibles au terminal. |
| [F016 — Preuves/rapports](../F016-reporting-observability/README.md) | Rapports par template, snapshots/watermarks, données redacted, causalité et preuves inspectables sans modèle. |
| [F017 — Sécurité](../F017-ai-guard-integration/README.md) | Intersection d'autorités, secrets référencés, broker, egress par le proxy Unleash et confinement réels ; `IsolationUnavailable` si garantie demandée absente. |
| [F018 — Reprise](../F018-reliability-recovery/README.md) | Cutpoints, identité d'effet, fencing, réconciliation guidée et interdiction de répétition aveugle. |

F010 reste une dépendance optionnelle des automatisations qui travaillent sur des repositories ; un comptable ou un particulier ne doit pas créer un dépôt Git pour utiliser un CSV ou une procédure navigateur. Le produit n'impose pas non plus de compte fournisseur de modèle à chaque replay d'une version déjà publiée. L'apprentissage assisté peut nécessiter une configuration modèle autorisée ; son absence ne transforme pas un replay en demande de clés.

## Adaptateurs et moteurs externes

Le [BrowserPort](../../architecture/browser-execution.md) définit la frontière navigateur et les profils médié/script. Le backend de référence V1 du profil `mediated` est l'adaptateur Rust `unleash-browser` en WebDriver sur `fantoccini`, avec chromedriver et Chrome for Testing du même build épinglés par version et SHA-256 ([Q-18](../../brief/open-questions.md)). Leur préparation est une étape explicite sous mandat, exécutée sous F011, sans commande dédiée ; toute sortie réseau du navigateur passe par le proxy d'egress d'Unleash. Le driver doit déclarer protocole, versions, capacités d'authentification, observation, annulation, isolation et réconciliation réellement disponibles. Réutiliser un moteur officiel ou une bibliothèque ne prouve pas automatiquement ces propriétés. Les [sources primaires](../../research/browser-automation-sources.md) sont la référence de compatibilité externe ; un contrat Unleash proposé n'est pas une méthode d'API Playwright inventée.

Le profil `project_script` peut utiliser un runtime et un paquet de projet épinglés ; Playwright Test (TypeScript) avec lockfile en est la référence. L'absence de contrôle instruction par instruction limite la prévention des effets et la réconciliation ; elle est indiquée avant sélection et dans les preuves. Le cœur Rust reste propriétaire du run, de la file, des permissions et du ledger. Aucun backend Python/Node concurrent ne décide des missions ou des budgets. Si l'absence de modèle ne peut être imposée aux appels réseau/processus du script, celui-ci n'est pas qualifié pour REQ-AUTO-006.

## Ordre d'intégration

Le socle critique est persistance/autorité → compilation/action profile/contrats → admission et modèle interdit → adaptateur/effets → reprise et preuves → parcours clients. Cette relation décrit les dépendances de F019 ; elle ne remplace pas le plan global. L'apprentissage avec LLM et le confort d'édition viennent après la capacité à vérifier et rejouer un candidat manuel sans modèle. Les trois fixtures métier partagent le même socle.

Les blocages de qualification doivent nommer la pièce manquante : protocole d'adaptateur non verrouillé, isolement OS non démontré, absence d'oracle métier ou effet distant non réconciliable. Aucun de ces blocages n'est résolu par l'ajout automatique d'un LLM de secours.
