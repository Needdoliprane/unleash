# Chantiers et responsabilité des équipes

La répartition ci-dessous permet un développement parallèle après accord sur les contrats des 19 features : F001–F018 conservées et F019 ajoutée. Une feature a un propriétaire d’intégration, même lorsque plusieurs équipes contribuent. Les experts de [agents/](../agents/README.md) sont des fonctions de revue ; ils ne remplacent pas les propriétaires de code. Les chantiers ne sont pas des microservices.

| Chantier | Features principales | Livrables de frontière | Relecteur contradicteur |
| --- | --- | --- | --- |
| Socle local | F001 F002 F018 | Identités, IPC, client partagé `unleash-client`, transactions, migrations, journal, recovery | Reliability + Security |
| Exécution et modèles | F003 F011 F012 | Admission mixte 5/50, processus, API et trois profils locaux, qualification de capacités/localité, OOM et annulation | Rust + API + QA |
| Procédures et équipes | F004 F005 F006 | AST v1/v1.1/v1.2, actions bornées, runs/joins, décisions de supervision publiées | Workflow + Multi-agent |
| Automatisations réutilisables | F019, avec ports F011/F017 | Artefact/données immuables, module automation du moteur, BrowserPort, profils mediated/project_script, preuve sans modèle et reprise | Workflow + Security + QA |
| Compétences | F007 F008 | Registries, binding, contrat, validation report, préparation idempotente des rôles nécessaires | Skill + Adversarial |
| Repository et connaissance | F009 F010 | Snapshot, provenance, DiscoveryReceipt, candidate integration | Git + Knowledge |
| Contexte et sécurité | F013 F017 | Document binding, OAuth, authorization port, audit | Security + API |
| Produit et expérience guidée | F014 F015 F016, parcours F019 | Read models, commandes, templates, aperçu d'effets, résultats/rapports, configuration locale et parité GUI/TUI | UX + Product |

## Propriété et coopération

Le socle fournit le port de transaction et les contraintes communes ; les domaines proposent leurs tables et transitions. Le writer n’absorbe pas leur logique métier. L’équipe workflow est propriétaire de la sémantique run/step et de son module automation, mais n’implémente pas les adaptateurs fournisseurs. L’équipe runtime ne décide pas du succès métier d’une mission. L’équipe GUI ne lit pas directement SQLite et ne réimplémente pas les permissions. L'équipe navigateur possède le mapping Rust vers un moteur réutilisé, sans ledger ou orchestrateur autonome ; l'équipe modèles qualifie LM Studio/Ollama/llama.cpp comme transports du même routeur.

Le parcours sans agents/Git n'est pas une réduction du chantier repository : c'est une composition du domaine où ces ressources ne sont pas nécessaires. Le mode supervised ne crée pas un manager universel ; les tâches de décision et leurs budgets sont publiés avant le run. La qualification d'un script externe précise ce qu'il peut contourner sans médiation réelle, au lieu de déplacer ce risque dans une autre équipe.

F016 est transversal : chaque producteur doit publier des faits causaux et des références de preuve exploitables. Le reporting ne reconstruit pas après coup des faits qui n’ont jamais été observés. F018 est également transversal : les failpoints et méthodes de réconciliation sont conçus avec chaque effet de bord dès son introduction.

## Contrats à figer avant parallélisme

Le premier paquet partagé contient IDs, revisions, CommandEnvelope unleash.ipc.v2 (command_id, authority_generation), erreurs, pagination, event cursor, actor context et politique de compatibilité. Le second contient InvocationBundle, WorkflowRunRef, RoleBinding, ToolAction, EvidenceRef et unités d'admission agent_invocation/replay_worker/provider_probe. Le troisième contient SnapshotRef/DiscoveryReceipt, IntegrationCandidate et DocumentRevisionRef. R3–R5 ajoutent au même paquet versionné AutomationArtifact/DataRevision, origine/politique modèle, BrowserPort, tuple de qualification locale et outputs de checkpoint `safe_to_continue/effect_state/assertion_refs`. R6 y ajoute ModelOrigin/ProviderProbeContext, DiscoveryReceipt multi-origines/TypedSourceRef, attentes de test scellées et frames for_each v1.2. Les décisions `continue/reconcile/correct/escalate` restent bornées par ces contrats.

Figer signifie publier une version discutée, des fixtures et des règles de changement ; cela ne signifie pas interdire toute évolution. Un changement incompatible doit annoncer ses consommateurs, migrations et tests. Les conventions de Rust restent dans [rust-workspace](../architecture/rust-workspace.md), le protocole dans [ipc-and-apis](../architecture/ipc-and-apis.md).

## Intégration des branches de développement

Une équipe travaille sur un chantier cohérent, pas un fichier arbitraire. Elle publie intentions, interfaces affectées et base Git. Les modifications de protocole ou migration partagée ont un intégrateur désigné pour éviter que deux branches créent des versions de schéma concurrentes incompatibles. Les worktrees de développement vivent uniquement sous `.claude/worktrees/` ([worktrees](../dev/worktrees.md)) ; ils ne sont ni inventoriés ni exportés avec le dossier.

Les critères de merge sont ceux de la [définition de terminé](../dev/definition-of-done.md) et du [protocole de goal](../dev/goal-protocol.md) acceptés en R7 pour le développement d’Unleash. Ce dossier recommande tests de domaine, compatibilité et revue pour ce projet, mais n’inscrit pas ces critères comme obligation universelle pour tous les workflows utilisateurs du futur produit.

## Mobilisation des experts

Au cadrage, architecture/product identifient exigences et risques. Avant implémentation d’une frontière, l’expert de domaine et son contradicteur comparent deux options. Pendant développement, les revues portent sur preuves et changements, pas sur réécriture systématique de tous les documents. Avant clôture, QA relie résultats aux critères, sécurité examine nouvelles autorités, UX vérifie parcours visibles, et le reviewer indépendant cherche les régressions transversales.

Les sorties attendues sont un constat reproductible, la propriété affectée, le fichier ou contrat, un scénario d’échec, une recommandation et une alternative. Un désaccord ne disparaît pas par vote : il est arbitré dans un ADR avec le risque résiduel.

## Interfaces après clarification R2

F014 et F015 partagent le catalogue et les contrats F001/F002 ; les équipes rendent séparément GUI et terminal, sans recopier le métier. Tauri 2 + React/TypeScript et Ratatui/Crossterm sont les piles acceptées ([ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) ; CLI, TUI et GUI accèdent au daemon par `unleash-client`. Le lot n’est terminé que lorsque les parcours avancés, réglages, éditeurs, erreurs et preuves satisfont la [matrice](../design/gui-tui-parity.md) dans les deux interfaces. L’équipe backend reçoit explicitement les opérations manquantes avant implémentation, au lieu de laisser naître une API privée d’écran.

## Raccordement R3–R5 et responsabilités de preuve

M0 qualifie la frontière navigateur et le pilote de référence de Q-18 ; M2 intègre les trois profils locaux et l'admission mixte ; M3 assemble artefact/données/profils v1.1/v1.2 et supervision ; M5 rend le parcours guidé et expert équivalents en GUI/TUI ; M7 consolide W/X/Y/Z et les campagnes locales/F019. H-17 à H-21 et INV-058 à INV-065 ont des responsables transversaux, sans créer une feature F020.

Le propriétaire F019 fournit les preuves de clic valide/résultat faux et de zéro modèle ; F012 fournit tuple, local_only, OOM et absence de cloud fallback ; F003 fournit le plafond mixte ; F014/F015 vérifient qu'un novice commence sans agents configurés et comprend les effets. QA relie ces preuves aux gates de livraison ; les comptes finaux d'exigences et d'exécutions appartiennent à la traçabilité commune, pas à une estimation dans ce plan.
