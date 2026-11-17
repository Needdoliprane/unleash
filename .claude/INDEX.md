# Index intégral du dossier OVPT Unleash

Révision consolidée R7 — 16 novembre 2026. Le manifest inventorie 389 fichiers, plus lui-même ; state/, worktrees/ et settings.local.json sont exclus. Les tests produit et preuves EV sont spécifiés, non exécutés. Résultats documentaires : [document-validation](testing/document-validation.md).

## Parcours conseillés

- Démarrer la réalisation avec un agent : `AGENTS.md` à la racine, puis [protocole de goal](dev/goal-protocol.md), [démarrage M0](dev/m0-kickoff.md) et [définition de terminé](dev/definition-of-done.md).
- Lire les décisions en vigueur : [invariants](brief/invariants.md), [révision R7](brief/revision-2026-11-16-r7.md), [questions résolues](brief/open-questions.md), [périmètre V1](brief/product-scope-v1.md).
- Examiner les objections et corrections historiques : [14 expertises R6](audits/r6/README.md), [verdict](audits/consolidated-verdict.md), [contrelecture](audits/r6/release-readthrough.md).
- Brancher une application documentaire : [catalogue](catalog/README.md), [ingestion](specs/document-package-ingestion.md), [opérations](catalog/operations.json), [manifest](catalog/package-manifest.json).
- Concevoir une procédure accessible : [usages](brief/general-purpose-scope.md), [guidage](design/guided-automation-experience.md), [itération de données](architecture/workflow-data-iteration.md), [modèles locaux](specs/local-models.md).
- Réaliser et vérifier : [architecture](architecture/system-overview.md), [IPC](architecture/ipc-and-apis.md), [ADR](architecture/adr/README.md), [règles](rules/design-dossier.md), [traçabilité](testing/traceability.md), [plan complet](plans/v1-implementation-plan.md).

## Accueil

- [INDEX.md](INDEX.md) — Index intégral du dossier OVPT Unleash
- [README.md](README.md) — OVPT Unleash — Dossier d'ingénierie V1
- [settings.json](settings.json)

## agents

- [agents/README.md](agents/README.md) — Comité d'experts : protocoles et revue réellement menée
- [agents/adversarial-reviewer.md](agents/adversarial-reviewer.md) — Regard 13 — Independent Adversarial Reviewer
- [agents/chief-architect.md](agents/chief-architect.md) — Regard 01 — Chief Software Architect
- [agents/dev/architect.md](agents/dev/architect.md)
- [agents/dev/doc-keeper.md](agents/dev/doc-keeper.md)
- [agents/dev/implementer.md](agents/dev/implementer.md)
- [agents/dev/integrator.md](agents/dev/integrator.md)
- [agents/dev/scout.md](agents/dev/scout.md)
- [agents/dev/systems-implementer.md](agents/dev/systems-implementer.md)
- [agents/dev/test-engineer.md](agents/dev/test-engineer.md)
- [agents/dev/ui-implementer.md](agents/dev/ui-implementer.md)
- [agents/dev/verifier.md](agents/dev/verifier.md)
- [agents/git-expert.md](agents/git-expert.md) — Regard 06 — Expert Git et intégration
- [agents/knowledge-expert.md](agents/knowledge-expert.md) — Regard 07 — Expert Knowledge et intelligence de code
- [agents/llm-api-expert.md](agents/llm-api-expert.md) — Regard 05 — LLM API & Context Engineer
- [agents/multi-agent-expert.md](agents/multi-agent-expert.md) — Regard 04 — Coordination multi-agent
- [agents/product-architect.md](agents/product-architect.md) — Regard 14 — Technical Product Architect
- [agents/qa-expert.md](agents/qa-expert.md) — Regard 12 — QA and Verification
- [agents/reliability-expert.md](agents/reliability-expert.md) — Regard 09 — Reliability and Persistence
- [agents/rust-systems-expert.md](agents/rust-systems-expert.md) — Regard 02 — Rust Systems Architect
- [agents/security-expert.md](agents/security-expert.md) — Regard 10 — Security Architect / AI-Guard Reviewer
- [agents/skill-contract-expert.md](agents/skill-contract-expert.md) — Regard 08 — Skills, rôles et contrats
- [agents/ux-expert.md](agents/ux-expert.md) — Regard 11 — UX / GUI Product Designer
- [agents/workflow-expert.md](agents/workflow-expert.md) — Regard 03 — Workflow et exécution déterministe

## architecture

- [architecture/adr/ADR-01-sqlite-persistence.md](architecture/adr/ADR-01-sqlite-persistence.md) — ADR-01 — SQLite et modèle d’état
- [architecture/adr/ADR-02-event-architecture.md](architecture/adr/ADR-02-event-architecture.md) — ADR-02 — Événements et dispatch durable
- [architecture/adr/ADR-03-rust-gui.md](architecture/adr/ADR-03-rust-gui.md) — ADR-03 — GUI Tauri 2 + React/TypeScript et parité des interfaces
- [architecture/adr/ADR-04-workflow-semantics.md](architecture/adr/ADR-04-workflow-semantics.md) — ADR-04 — Moteur de workflows et frontière de migration
- [architecture/adr/ADR-05-durable-sessions.md](architecture/adr/ADR-05-durable-sessions.md) — ADR-05 — Sessions durables et incarnations éphémères
- [architecture/adr/ADR-06-skill-role-registry.md](architecture/adr/ADR-06-skill-role-registry.md) — ADR-06 — Skills et rôles persistants
- [architecture/adr/ADR-07-contract-enforcement.md](architecture/adr/ADR-07-contract-enforcement.md) — ADR-07 — Contrats de sortie et preuves
- [architecture/adr/ADR-08-knowledge-index.md](architecture/adr/ADR-08-knowledge-index.md) — ADR-08 — Connaissances partagées et discovery
- [architecture/adr/ADR-09-git-worktrees.md](architecture/adr/ADR-09-git-worktrees.md) — ADR-09 — Git, worktrees et intégration
- [architecture/adr/ADR-10-ai-guard-boundary.md](architecture/adr/ADR-10-ai-guard-boundary.md) — ADR-10 — Autorisation, isolation et AI-Guard
- [architecture/adr/ADR-11-model-api-abstraction.md](architecture/adr/ADR-11-model-api-abstraction.md) — ADR-11 — Abstraction des modèles API
- [architecture/adr/ADR-12-hierarchy.md](architecture/adr/ADR-12-hierarchy.md) — ADR-12 — Frontières N+2 / N+1 / N
- [architecture/adr/ADR-13-scheduler-budget.md](architecture/adr/ADR-13-scheduler-budget.md) — ADR-13 — Concurrence, fairness et budgets
- [architecture/adr/ADR-14-local-data-oss.md](architecture/adr/ADR-14-local-data-oss.md) — ADR-14 — Données locales, open source, plateformes et distribution
- [architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md](architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) — ADR-15 — Rust pragmatique et parité fonctionnelle GUI/TUI
- [architecture/adr/ADR-16-reusable-automation-and-supervision.md](architecture/adr/ADR-16-reusable-automation-and-supervision.md) — ADR-16 — Automatisations réutilisables et supervision par blocs
- [architecture/adr/ADR-17-local-model-providers.md](architecture/adr/ADR-17-local-model-providers.md) — ADR-17 — Fournisseurs de modèles locaux dans le Model Router
- [architecture/adr/ADR-18-bounded-data-iteration.md](architecture/adr/ADR-18-bounded-data-iteration.md) — ADR-18 — Itération bornée sur données et diagnostic bloqué
- [architecture/adr/ADR-19-design-package-ingestion.md](architecture/adr/ADR-19-design-package-ingestion.md) — ADR-19 — Paquet de conception contrôlable par une application
- [architecture/adr/README.md](architecture/adr/README.md) — Architecture Decision Records
- [architecture/automation-artifact-model.md](architecture/automation-artifact-model.md) — Automatisations réutilisables — artefacts, données et exécutions
- [architecture/automation-execution-profile.md](architecture/automation-execution-profile.md) — Profil d’exécution des automatismes — workflow v1.1
- [architecture/browser-execution.md](architecture/browser-execution.md) — Exécution navigateur — architecture R3
- [architecture/client-operation-catalog.md](architecture/client-operation-catalog.md) — Catalogue commun des opérations client — consolidation R3–R5
- [architecture/component-model.md](architecture/component-model.md) — Modèle de composants et propriété des décisions
- [architecture/dependency-map.md](architecture/dependency-map.md) — Carte des dépendances d’implémentation
- [architecture/event-model.md](architecture/event-model.md) — Événements, dispatch et réveils
- [architecture/failure-recovery.md](architecture/failure-recovery.md) — Pannes, cutpoints et protocole de reprise
- [architecture/git-coordination.md](architecture/git-coordination.md) — Architecture — coordination Git et journal d'intégration
- [architecture/ipc-and-apis.md](architecture/ipc-and-apis.md) — API locale, IPC et erreurs communes
- [architecture/knowledge-indexing.md](architecture/knowledge-indexing.md) — Architecture — captures et index de connaissance
- [architecture/process-supervision.md](architecture/process-supervision.md) — Supervision des processus et process-host
- [architecture/runtime-lifecycle.md](architecture/runtime-lifecycle.md) — Cycle de vie du runtime utilisateur
- [architecture/rust-workspace.md](architecture/rust-workspace.md) — Workspace Rust et frontières natives
- [architecture/security-boundaries.md](architecture/security-boundaries.md) — Frontières de sécurité et garanties minimales
- [architecture/session-model.md](architecture/session-model.md) — Sessions durables, invocations et tentatives
- [architecture/skills-contract-model.md](architecture/skills-contract-model.md) — Modèle des skills, rôles, contrats et preuves
- [architecture/sqlite-design.md](architecture/sqlite-design.md) — SQLite : transactions, contraintes et schéma système
- [architecture/state-machines.md](architecture/state-machines.md) — Machines à états et frontières d'exécution
- [architecture/storage-model.md](architecture/storage-model.md) — Modèle de stockage et identités canoniques
- [architecture/system-overview.md](architecture/system-overview.md) — Architecture consolidée proposée — OVPT Unleash
- [architecture/workflow-data-iteration.md](architecture/workflow-data-iteration.md) — Workflow v1.2 — itération bornée sur données
- [architecture/workflow-dsl.md](architecture/workflow-dsl.md) — DSL de workflow — contrat accepté `unleash.workflow.v1` (ADR-04)

## audits

- [audits/adversarial-findings.md](audits/adversarial-findings.md) — Red team architecturale — scénarios qui peuvent casser Unleash
- [audits/api-compatibility.md](audits/api-compatibility.md) — Audit indépendant — Expert 5, API LLM et contexte
- [audits/architecture-audit.md](audits/architecture-audit.md) — Audit 01 — Chief Software Architect
- [audits/browser-automation-audit.md](audits/browser-automation-audit.md) — Audit contradictoire — automatisation navigateur R3
- [audits/consolidated-verdict.md](audits/consolidated-verdict.md) — Verdict consolidé — audit architectural OVPT Unleash
- [audits/final-independent-review.md](audits/final-independent-review.md) — Seconde revue contradictoire indépendante — Expert 13
- [audits/frontend-terminal-revision-review.md](audits/frontend-terminal-revision-review.md) — Revue contradictoire ciblée R2 — Frontend et terminal
- [audits/general-purpose-product-review.md](audits/general-purpose-product-review.md) — Revue produit contradictoire R3 — usages polyvalents et automatisations
- [audits/generalist-local-consistency-review.md](audits/generalist-local-consistency-review.md) — Revue contradictoire indépendante — généralisation, automatisations et modèles locaux
- [audits/git-concurrency.md](audits/git-concurrency.md) — Audit 06 — Git, concurrence et intégration
- [audits/gui-ux-audit.md](audits/gui-ux-audit.md) — Audit indépendant — Expert 11, supervision et UX
- [audits/knowledge-audit.md](audits/knowledge-audit.md) — Audit 07 — Knowledge et intelligence de code
- [audits/multi-agent-coordination.md](audits/multi-agent-coordination.md) — Audit Expert 4 — Systèmes multi-agents et hiérarchie
- [audits/persistence-and-recovery.md](audits/persistence-and-recovery.md) — Regard 9 — Reliability and Persistence
- [audits/qa-verification.md](audits/qa-verification.md) — Regard 12 — QA and Verification
- [audits/r6/README.md](audits/r6/README.md) — R6 — Quatorze expertises déléguées et corrections
- [audits/r6/expert-01-chief.md](audits/r6/expert-01-chief.md) — R6 — Expert 01 — Chief Software Architect
- [audits/r6/expert-02-rust.md](audits/r6/expert-02-rust.md) — R6 — Expert 02 Rust Systems Architect
- [audits/r6/expert-03-workflow.md](audits/r6/expert-03-workflow.md) — R6 — Expert 03 : workflow et exécution déterministe
- [audits/r6/expert-04-multi-agent.md](audits/r6/expert-04-multi-agent.md) — R6 — Expert 04 — Coordination multi-agent
- [audits/r6/expert-05-llm.md](audits/r6/expert-05-llm.md) — R6 — Expert 05 — LLM API & Context Engineer
- [audits/r6/expert-06-git.md](audits/r6/expert-06-git.md) — R6 — Expert 06 — Git et intégration
- [audits/r6/expert-07-knowledge.md](audits/r6/expert-07-knowledge.md) — R6 — Expert 07 — Knowledge & Code Intelligence
- [audits/r6/expert-08-skills.md](audits/r6/expert-08-skills.md) — R6 — Expert 8 : skills, rôles et contrats
- [audits/r6/expert-09-reliability.md](audits/r6/expert-09-reliability.md) — R6 — Expert 09 Reliability and Persistence
- [audits/r6/expert-10-security.md](audits/r6/expert-10-security.md) — R6 — Expert 10 : sécurité et AI-Guard
- [audits/r6/expert-11-ux.md](audits/r6/expert-11-ux.md) — R6 — Expert 11 : UX / GUI / TUI
- [audits/r6/expert-12-qa.md](audits/r6/expert-12-qa.md) — R6 — Expert 12 : QA et vérification
- [audits/r6/expert-13-adversarial.md](audits/r6/expert-13-adversarial.md) — R6 — Expert 13 : revue contradictoire indépendante
- [audits/r6/expert-14-product.md](audits/r6/expert-14-product.md) — R6 — Expert 14 : Technical Product Architect
- [audits/r6/findings.json](audits/r6/findings.json)
- [audits/r6/release-readthrough.md](audits/r6/release-readthrough.md) — R6 — Contrelecture de remise
- [audits/rust-feasibility.md](audits/rust-feasibility.md) — Regard 2 — Rust Systems Architect
- [audits/security-threat-model.md](audits/security-threat-model.md) — Audit indépendant — Expert 10, sécurité et AI-Guard
- [audits/skills-and-contracts.md](audits/skills-and-contracts.md) — Audit Expert 8 — Skills, contrats et vérification
- [audits/workflow-determinism.md](audits/workflow-determinism.md) — Audit Expert 3 — Workflow et exécution déterministe

## brief

- [brief/assumptions.md](brief/assumptions.md) — Hypothèses proposées et falsifiables
- [brief/clarifications.md](brief/clarifications.md) — Clarifications utilisateur — R2 à R5 et R7, état consolidé R7
- [brief/general-purpose-scope.md](brief/general-purpose-scope.md) — Unleash polyvalent — périmètre produit R3
- [brief/glossary.md](brief/glossary.md) — Glossaire normatif
- [brief/implementation-policy.md](brief/implementation-policy.md) — Politique de langage, dépendances et interfaces — révision 2
- [brief/invariants.md](brief/invariants.md) — Inventaire des invariants et ambiguïtés
- [brief/open-questions.md](brief/open-questions.md) — Registre des questions et décisions
- [brief/product-scope-v1.md](brief/product-scope-v1.md) — Périmètre fonctionnel complet de la V1
- [brief/revision-2026-11-16-r2.md](brief/revision-2026-11-16-r2.md) — Révision 2 — Rust pragmatique, Kivy/Tauri et parité terminal
- [brief/revision-2026-11-16-r3-r5.md](brief/revision-2026-11-16-r3-r5.md) — Révisions R3–R5 — automatisations, modèles locaux et démarrage guidé
- [brief/revision-2026-11-16-r6.md](brief/revision-2026-11-16-r6.md) — Révision R6 — Revue multi-experts et exploitation du dossier
- [brief/revision-2026-11-16-r7.md](brief/revision-2026-11-16-r7.md) — Révision R7 — Arbitrages rendus, corrections d'audit et ouverture de la réalisation
- [brief/vision.md](brief/vision.md) — Vision — Unleash, produit OVPT

## catalog

- [catalog/README.md](catalog/README.md) — Catalogue exploitable du dossier de conception
- [catalog/design-catalog.json](catalog/design-catalog.json)
- [catalog/operations.json](catalog/operations.json)
- [catalog/package-manifest.json](catalog/package-manifest.json)

## design

- [design/activity-timeline.md](design/activity-timeline.md) — Timeline d’activité
- [design/agent-tree-and-status.md](design/agent-tree-and-status.md) — Arbre d’agents et états
- [design/cli-tui-design.md](design/cli-tui-design.md) — Design de la CLI et de la TUI
- [design/context-explorer.md](design/context-explorer.md) — Explorateur du contexte
- [design/conversation-ux.md](design/conversation-ux.md) — Conversations et interventions humaines
- [design/design-principles.md](design/design-principles.md) — Principes d’interface
- [design/gui-information-architecture.md](design/gui-information-architecture.md) — Architecture de l’information
- [design/gui-screen-specifications.md](design/gui-screen-specifications.md) — Spécifications des dix espaces GUI
- [design/gui-tui-parity.md](design/gui-tui-parity.md) — Matrice de parité GUI / TUI / CLI
- [design/guided-automation-experience.md](design/guided-automation-experience.md) — Expérience guidée d’automatisation — proposition R3
- [design/interaction-flows.md](design/interaction-flows.md) — Parcours d’interaction et points de vérité
- [design/reports-and-history.md](design/reports-and-history.md) — Rapports et historique
- [design/skill-studio.md](design/skill-studio.md) — Skill Studio et Role Registry
- [design/terminal-inspector.md](design/terminal-inspector.md) — Inspecteur de terminaux et processus
- [design/visual-design-system.md](design/visual-design-system.md) — Design system proposé — OVPT Unleash
- [design/workflow-editor.md](design/workflow-editor.md) — Éditeur de workflows

## dev

- [dev/definition-of-done.md](dev/definition-of-done.md) — Définition de terminé
- [dev/goal-journal-template.md](dev/goal-journal-template.md) — Modèles de fichiers d'un goal
- [dev/goal-protocol.md](dev/goal-protocol.md) — Protocole `/goal` : développement autonome d'Unleash
- [dev/m0-kickoff.md](dev/m0-kickoff.md) — Démarrage de la réalisation — jalon M0
- [dev/model-routing.md](dev/model-routing.md) — Routage des modèles pour les agents de développement
- [dev/worktrees.md](dev/worktrees.md) — Politique des worktrees de développement

## examples

- [examples/automation-observation.json](examples/automation-observation.json)
- [examples/automation-reconciliation-workflow.json](examples/automation-reconciliation-workflow.json)
- [examples/automation-rows-workflow.json](examples/automation-rows-workflow.json)
- [examples/automation-supervised-workflow.json](examples/automation-supervised-workflow.json)
- [examples/automation-supervisor-role.json](examples/automation-supervisor-role.json)
- [examples/automation-supervisor-skill.json](examples/automation-supervisor-skill.json)
- [examples/context-manifest.json](examples/context-manifest.json)
- [examples/contract-claim.json](examples/contract-claim.json)
- [examples/contract-profile.json](examples/contract-profile.json)
- [examples/discovery-completion.json](examples/discovery-completion.json)
- [examples/discovery-human.json](examples/discovery-human.json)
- [examples/discovery-receipt.json](examples/discovery-receipt.json)
- [examples/discovery-replay.json](examples/discovery-replay.json)
- [examples/expected-execution-binding.json](examples/expected-execution-binding.json)
- [examples/expected-test-execution.json](examples/expected-test-execution.json)
- [examples/feature-workflow.json](examples/feature-workflow.json)
- [examples/integration-role.json](examples/integration-role.json)
- [examples/integration-skill.json](examples/integration-skill.json)
- [examples/ipc-command.json](examples/ipc-command.json)
- [examples/leader-role.json](examples/leader-role.json)
- [examples/leader-skill.json](examples/leader-skill.json)
- [examples/merge-only-workflow.json](examples/merge-only-workflow.json)
- [examples/role.json](examples/role.json)
- [examples/skill.json](examples/skill.json)
- [examples/test-execution-observation.json](examples/test-execution-observation.json)
- [examples/test-verification-contract-profile.json](examples/test-verification-contract-profile.json)
- [examples/test-verification-skill.json](examples/test-verification-skill.json)
- [examples/unleash-config.toml](examples/unleash-config.toml)
- [examples/workflow-fixtures.md](examples/workflow-fixtures.md) — Fixtures de workflows, skills et rôles

## features

- [features/F001-core-daemon/README.md](features/F001-core-daemon/README.md) — F001 — Noyau et daemon utilisateur
- [features/F001-core-daemon/acceptance.md](features/F001-core-daemon/acceptance.md) — Acceptation F001
- [features/F001-core-daemon/dependencies.md](features/F001-core-daemon/dependencies.md) — Dépendances F001
- [features/F001-core-daemon/interfaces.md](features/F001-core-daemon/interfaces.md) — Interfaces F001
- [features/F001-core-daemon/requirements.md](features/F001-core-daemon/requirements.md) — Exigences F001
- [features/F001-core-daemon/technical-design.md](features/F001-core-daemon/technical-design.md) — Conception technique F001
- [features/F001-core-daemon/tests.md](features/F001-core-daemon/tests.md) — Tests et preuves F001
- [features/F002-persistence-events/README.md](features/F002-persistence-events/README.md) — F002 — Persistance, journal et événements
- [features/F002-persistence-events/acceptance.md](features/F002-persistence-events/acceptance.md) — Acceptation F002
- [features/F002-persistence-events/dependencies.md](features/F002-persistence-events/dependencies.md) — Dépendances F002
- [features/F002-persistence-events/interfaces.md](features/F002-persistence-events/interfaces.md) — Interfaces F002
- [features/F002-persistence-events/requirements.md](features/F002-persistence-events/requirements.md) — Exigences F002
- [features/F002-persistence-events/technical-design.md](features/F002-persistence-events/technical-design.md) — Conception technique F002
- [features/F002-persistence-events/tests.md](features/F002-persistence-events/tests.md) — Tests F002
- [features/F003-agent-runtime/README.md](features/F003-agent-runtime/README.md) — F003 — Runtime des agents et admission globale
- [features/F003-agent-runtime/acceptance.md](features/F003-agent-runtime/acceptance.md) — Acceptation F003
- [features/F003-agent-runtime/dependencies.md](features/F003-agent-runtime/dependencies.md) — Dépendances F003
- [features/F003-agent-runtime/interfaces.md](features/F003-agent-runtime/interfaces.md) — Interfaces F003
- [features/F003-agent-runtime/requirements.md](features/F003-agent-runtime/requirements.md) — Exigences F003
- [features/F003-agent-runtime/technical-design.md](features/F003-agent-runtime/technical-design.md) — Conception technique F003
- [features/F003-agent-runtime/tests.md](features/F003-agent-runtime/tests.md) — Tests F003
- [features/F004-hierarchical-orchestration/README.md](features/F004-hierarchical-orchestration/README.md) — F004 — Orchestration hiérarchique
- [features/F004-hierarchical-orchestration/acceptance.md](features/F004-hierarchical-orchestration/acceptance.md) — F004 — Critères d'acceptation
- [features/F004-hierarchical-orchestration/dependencies.md](features/F004-hierarchical-orchestration/dependencies.md) — F004 — Dépendances et frontières d'équipe
- [features/F004-hierarchical-orchestration/interfaces.md](features/F004-hierarchical-orchestration/interfaces.md) — F004 — Interfaces proposées
- [features/F004-hierarchical-orchestration/requirements.md](features/F004-hierarchical-orchestration/requirements.md) — F004 — Exigences atomiques
- [features/F004-hierarchical-orchestration/technical-design.md](features/F004-hierarchical-orchestration/technical-design.md) — F004 — Design technique proposé
- [features/F004-hierarchical-orchestration/tests.md](features/F004-hierarchical-orchestration/tests.md) — F004 — Plan de tests et preuves
- [features/F005-workflow-engine/README.md](features/F005-workflow-engine/README.md) — F005 — Moteur de workflow
- [features/F005-workflow-engine/acceptance.md](features/F005-workflow-engine/acceptance.md) — F005 — Acceptation
- [features/F005-workflow-engine/dependencies.md](features/F005-workflow-engine/dependencies.md) — F005 — Dépendances
- [features/F005-workflow-engine/interfaces.md](features/F005-workflow-engine/interfaces.md) — F005 — Interfaces proposées
- [features/F005-workflow-engine/requirements.md](features/F005-workflow-engine/requirements.md) — F005 — Exigences atomiques
- [features/F005-workflow-engine/technical-design.md](features/F005-workflow-engine/technical-design.md) — F005 — Design technique proposé
- [features/F005-workflow-engine/tests.md](features/F005-workflow-engine/tests.md) — F005 — Tests et preuves attendues
- [features/F006-workflow-compiler/README.md](features/F006-workflow-compiler/README.md) — F006 — Compilateur de workflows
- [features/F006-workflow-compiler/acceptance.md](features/F006-workflow-compiler/acceptance.md) — F006 — Acceptation
- [features/F006-workflow-compiler/dependencies.md](features/F006-workflow-compiler/dependencies.md) — F006 — Dépendances
- [features/F006-workflow-compiler/interfaces.md](features/F006-workflow-compiler/interfaces.md) — F006 — Interfaces proposées
- [features/F006-workflow-compiler/requirements.md](features/F006-workflow-compiler/requirements.md) — F006 — Exigences atomiques
- [features/F006-workflow-compiler/technical-design.md](features/F006-workflow-compiler/technical-design.md) — F006 — Design technique proposé
- [features/F006-workflow-compiler/tests.md](features/F006-workflow-compiler/tests.md) — F006 — Tests prévus
- [features/F007-skill-studio-registry/README.md](features/F007-skill-studio-registry/README.md) — F007 — Skills Studio, Skill Registry et Role Registry
- [features/F007-skill-studio-registry/acceptance.md](features/F007-skill-studio-registry/acceptance.md) — F007 — Acceptation
- [features/F007-skill-studio-registry/dependencies.md](features/F007-skill-studio-registry/dependencies.md) — F007 — Dépendances
- [features/F007-skill-studio-registry/interfaces.md](features/F007-skill-studio-registry/interfaces.md) — F007 — Interfaces proposées
- [features/F007-skill-studio-registry/requirements.md](features/F007-skill-studio-registry/requirements.md) — F007 — Exigences atomiques des deux registres
- [features/F007-skill-studio-registry/technical-design.md](features/F007-skill-studio-registry/technical-design.md) — F007 — Design technique proposé
- [features/F007-skill-studio-registry/tests.md](features/F007-skill-studio-registry/tests.md) — F007 — Plan de tests
- [features/F008-contract-engine/README.md](features/F008-contract-engine/README.md) — F008 — Moteur de contrats
- [features/F008-contract-engine/acceptance.md](features/F008-contract-engine/acceptance.md) — F008 — Acceptation
- [features/F008-contract-engine/dependencies.md](features/F008-contract-engine/dependencies.md) — F008 — Dépendances
- [features/F008-contract-engine/interfaces.md](features/F008-contract-engine/interfaces.md) — F008 — Interfaces proposées
- [features/F008-contract-engine/requirements.md](features/F008-contract-engine/requirements.md) — F008 — Exigences atomiques
- [features/F008-contract-engine/technical-design.md](features/F008-contract-engine/technical-design.md) — F008 — Design technique proposé
- [features/F008-contract-engine/tests.md](features/F008-contract-engine/tests.md) — F008 — Tests et preuves attendues
- [features/F009-knowledge-engine/README.md](features/F009-knowledge-engine/README.md) — F009 — Knowledge Engine et découverte d'OVPT Unleash
- [features/F009-knowledge-engine/acceptance.md](features/F009-knowledge-engine/acceptance.md) — F009 — Critères d'acceptation
- [features/F009-knowledge-engine/dependencies.md](features/F009-knowledge-engine/dependencies.md) — F009 — Dépendances et frontières d'équipe
- [features/F009-knowledge-engine/interfaces.md](features/F009-knowledge-engine/interfaces.md) — F009 — Interfaces proposées
- [features/F009-knowledge-engine/requirements.md](features/F009-knowledge-engine/requirements.md) — F009 — Exigences
- [features/F009-knowledge-engine/technical-design.md](features/F009-knowledge-engine/technical-design.md) — F009 — Conception technique
- [features/F009-knowledge-engine/tests.md](features/F009-knowledge-engine/tests.md) — F009 — Plan de tests et preuves attendues
- [features/F010-git-coordination/README.md](features/F010-git-coordination/README.md) — F010 — Coordination Git d'OVPT Unleash
- [features/F010-git-coordination/acceptance.md](features/F010-git-coordination/acceptance.md) — F010 — Critères d'acceptation
- [features/F010-git-coordination/dependencies.md](features/F010-git-coordination/dependencies.md) — F010 — Dépendances et intégration
- [features/F010-git-coordination/interfaces.md](features/F010-git-coordination/interfaces.md) — F010 — Interfaces proposées
- [features/F010-git-coordination/requirements.md](features/F010-git-coordination/requirements.md) — F010 — Exigences
- [features/F010-git-coordination/technical-design.md](features/F010-git-coordination/technical-design.md) — F010 — Conception technique
- [features/F010-git-coordination/tests.md](features/F010-git-coordination/tests.md) — F010 — Plan de tests Git
- [features/F011-terminal-pool/README.md](features/F011-terminal-pool/README.md) — F011 — Pool de terminaux, processus et ports
- [features/F011-terminal-pool/acceptance.md](features/F011-terminal-pool/acceptance.md) — Acceptation F011
- [features/F011-terminal-pool/dependencies.md](features/F011-terminal-pool/dependencies.md) — Dépendances F011
- [features/F011-terminal-pool/interfaces.md](features/F011-terminal-pool/interfaces.md) — Interfaces F011
- [features/F011-terminal-pool/requirements.md](features/F011-terminal-pool/requirements.md) — Exigences F011
- [features/F011-terminal-pool/technical-design.md](features/F011-terminal-pool/technical-design.md) — Conception technique F011
- [features/F011-terminal-pool/tests.md](features/F011-terminal-pool/tests.md) — Tests F011
- [features/F012-model-api-router/README.md](features/F012-model-api-router/README.md) — F012 — Routage API des modèles cloud et locaux
- [features/F012-model-api-router/acceptance.md](features/F012-model-api-router/acceptance.md) — F012 — critères d’acceptation
- [features/F012-model-api-router/dependencies.md](features/F012-model-api-router/dependencies.md) — F012 — dépendances et intégration
- [features/F012-model-api-router/interfaces.md](features/F012-model-api-router/interfaces.md) — F012 — interfaces et contrats
- [features/F012-model-api-router/requirements.md](features/F012-model-api-router/requirements.md) — F012 — exigences
- [features/F012-model-api-router/technical-design.md](features/F012-model-api-router/technical-design.md) — F012 — conception technique Rust
- [features/F012-model-api-router/tests.md](features/F012-model-api-router/tests.md) — F012 — stratégie de validation
- [features/F013-document-connectors/README.md](features/F013-document-connectors/README.md) — F013 — Connecteurs documentaires
- [features/F013-document-connectors/acceptance.md](features/F013-document-connectors/acceptance.md) — F013 — critères d’acceptation
- [features/F013-document-connectors/dependencies.md](features/F013-document-connectors/dependencies.md) — F013 — dépendances et intégration
- [features/F013-document-connectors/interfaces.md](features/F013-document-connectors/interfaces.md) — F013 — interfaces et contrats
- [features/F013-document-connectors/requirements.md](features/F013-document-connectors/requirements.md) — F013 — exigences
- [features/F013-document-connectors/technical-design.md](features/F013-document-connectors/technical-design.md) — F013 — conception technique Rust
- [features/F013-document-connectors/tests.md](features/F013-document-connectors/tests.md) — F013 — stratégie de validation
- [features/F014-gui/README.md](features/F014-gui/README.md) — F014 — GUI de supervision OVPT Unleash
- [features/F014-gui/acceptance.md](features/F014-gui/acceptance.md) — F014 — acceptation
- [features/F014-gui/dependencies.md](features/F014-gui/dependencies.md) — F014 — dépendances
- [features/F014-gui/interfaces.md](features/F014-gui/interfaces.md) — F014 — interfaces
- [features/F014-gui/requirements.md](features/F014-gui/requirements.md) — F014 — exigences
- [features/F014-gui/technical-design.md](features/F014-gui/technical-design.md) — F014 — conception technique
- [features/F014-gui/tests.md](features/F014-gui/tests.md) — F014 — tests et erreurs
- [features/F015-cli-tui/README.md](features/F015-cli-tui/README.md) — F015 — CLI et TUI Rust à parité avec la GUI
- [features/F015-cli-tui/acceptance.md](features/F015-cli-tui/acceptance.md) — F015 — acceptation
- [features/F015-cli-tui/dependencies.md](features/F015-cli-tui/dependencies.md) — F015 — dépendances
- [features/F015-cli-tui/interfaces.md](features/F015-cli-tui/interfaces.md) — F015 — interfaces
- [features/F015-cli-tui/requirements.md](features/F015-cli-tui/requirements.md) — F015 — exigences
- [features/F015-cli-tui/technical-design.md](features/F015-cli-tui/technical-design.md) — F015 — conception technique
- [features/F015-cli-tui/tests.md](features/F015-cli-tui/tests.md) — F015 — tests et erreurs
- [features/F016-reporting-observability/README.md](features/F016-reporting-observability/README.md) — F016 — Rapports, preuves et observabilité d'OVPT Unleash
- [features/F016-reporting-observability/acceptance.md](features/F016-reporting-observability/acceptance.md) — F016 — Critères d'acceptation
- [features/F016-reporting-observability/dependencies.md](features/F016-reporting-observability/dependencies.md) — F016 — Dépendances et responsabilités
- [features/F016-reporting-observability/interfaces.md](features/F016-reporting-observability/interfaces.md) — F016 — Interfaces proposées
- [features/F016-reporting-observability/requirements.md](features/F016-reporting-observability/requirements.md) — F016 — Exigences
- [features/F016-reporting-observability/technical-design.md](features/F016-reporting-observability/technical-design.md) — F016 — Conception technique
- [features/F016-reporting-observability/tests.md](features/F016-reporting-observability/tests.md) — F016 — Plan de tests de traçabilité
- [features/F017-ai-guard-integration/README.md](features/F017-ai-guard-integration/README.md) — F017 — Sécurité locale et intégration AI-Guard
- [features/F017-ai-guard-integration/acceptance.md](features/F017-ai-guard-integration/acceptance.md) — F017 — acceptation
- [features/F017-ai-guard-integration/dependencies.md](features/F017-ai-guard-integration/dependencies.md) — F017 — dépendances
- [features/F017-ai-guard-integration/interfaces.md](features/F017-ai-guard-integration/interfaces.md) — F017 — interfaces
- [features/F017-ai-guard-integration/requirements.md](features/F017-ai-guard-integration/requirements.md) — F017 — exigences
- [features/F017-ai-guard-integration/technical-design.md](features/F017-ai-guard-integration/technical-design.md) — F017 — conception technique
- [features/F017-ai-guard-integration/tests.md](features/F017-ai-guard-integration/tests.md) — F017 — tests et erreurs
- [features/F018-reliability-recovery/README.md](features/F018-reliability-recovery/README.md) — F018 — Fiabilité, reprise et réconciliation
- [features/F018-reliability-recovery/acceptance.md](features/F018-reliability-recovery/acceptance.md) — Acceptation F018
- [features/F018-reliability-recovery/dependencies.md](features/F018-reliability-recovery/dependencies.md) — Dépendances F018
- [features/F018-reliability-recovery/interfaces.md](features/F018-reliability-recovery/interfaces.md) — Interfaces F018
- [features/F018-reliability-recovery/requirements.md](features/F018-reliability-recovery/requirements.md) — Exigences F018
- [features/F018-reliability-recovery/technical-design.md](features/F018-reliability-recovery/technical-design.md) — Conception technique F018
- [features/F018-reliability-recovery/tests.md](features/F018-reliability-recovery/tests.md) — Tests F018
- [features/F019-reusable-automations/README.md](features/F019-reusable-automations/README.md) — F019 — Automatisations réutilisables apprises puis rejouées
- [features/F019-reusable-automations/acceptance.md](features/F019-reusable-automations/acceptance.md) — Acceptation F019
- [features/F019-reusable-automations/dependencies.md](features/F019-reusable-automations/dependencies.md) — Dépendances F019
- [features/F019-reusable-automations/interfaces.md](features/F019-reusable-automations/interfaces.md) — Interfaces F019
- [features/F019-reusable-automations/requirements.md](features/F019-reusable-automations/requirements.md) — F019 — Exigences atomiques
- [features/F019-reusable-automations/technical-design.md](features/F019-reusable-automations/technical-design.md) — F019 — Design technique proposé
- [features/F019-reusable-automations/tests.md](features/F019-reusable-automations/tests.md) — Tests F019

## hooks

- [hooks/git_guard.py](hooks/git_guard.py)

## plans

- [plans/critical-path.md](plans/critical-path.md) — Chemin critique et expériences de décision
- [plans/dependencies.md](plans/dependencies.md) — Dépendances de réalisation
- [plans/readiness-checklist.md](plans/readiness-checklist.md) — Conditions de préparation et de clôture
- [plans/risk-register.md](plans/risk-register.md) — Registre des risques, hypothèses et gates
- [plans/v1-implementation-plan.md](plans/v1-implementation-plan.md) — Plan de réalisation de toute la V1
- [plans/workstreams.md](plans/workstreams.md) — Chantiers et responsabilité des équipes

## research

- [research/browser-automation-sources.md](research/browser-automation-sources.md) — Automatisation navigateur — sources et conclusions R3
- [research/git-knowledge-sources.md](research/git-knowledge-sources.md) — Recherche primaire — Git, connaissance et traçabilité
- [research/local-model-sources.md](research/local-model-sources.md) — Modèles locaux — sources primaires et limites de qualification
- [research/security-sources.md](research/security-sources.md) — Sources sécurité et connecteurs — relevé du 16 novembre 2026
- [research/sources.md](research/sources.md) — Registre des sources et niveau de vérification
- [research/systems-sources.md](research/systems-sources.md) — Sources primaires — systèmes Rust, persistance et vérification
- [research/technology-comparison.md](research/technology-comparison.md) — Comparaison technique et versions observées
- [research/verified-api-capabilities.md](research/verified-api-capabilities.md) — Capacités API vérifiées et limites de vérification
- [research/workflow-skills-sources.md](research/workflow-skills-sources.md) — Sources — workflows, hiérarchie et contrats

## rules

- [rules/design-dossier.md](rules/design-dossier.md) — Règles de modification du dossier de conception
- [rules/gui-renderer.md](rules/gui-renderer.md) — Règles du renderer GUI (React + TypeScript)
- [rules/rust.md](rules/rust.md) — Règles de code Rust d'Unleash
- [rules/testing.md](rules/testing.md) — Règles de test et de preuve
- [rules/worktrees.md](rules/worktrees.md) — Règle Git : worktrees uniquement dans `.claude/worktrees/`

## schemas

- [schemas/README.md](schemas/README.md) — Schémas et fixtures de conception
- [schemas/automation-observation.schema.json](schemas/automation-observation.schema.json)
- [schemas/context-manifest.schema.json](schemas/context-manifest.schema.json)
- [schemas/contract-claim.schema.json](schemas/contract-claim.schema.json)
- [schemas/contract-profile.schema.json](schemas/contract-profile.schema.json)
- [schemas/design-catalog.schema.json](schemas/design-catalog.schema.json)
- [schemas/design-operations.schema.json](schemas/design-operations.schema.json)
- [schemas/design-package-manifest.schema.json](schemas/design-package-manifest.schema.json)
- [schemas/design-traceability.schema.json](schemas/design-traceability.schema.json)
- [schemas/discovery-receipt.schema.json](schemas/discovery-receipt.schema.json)
- [schemas/event.schema.json](schemas/event.schema.json)
- [schemas/expected-execution-binding.schema.json](schemas/expected-execution-binding.schema.json)
- [schemas/expected-test-execution.schema.json](schemas/expected-test-execution.schema.json)
- [schemas/ipc-client-upload-segment.schema.json](schemas/ipc-client-upload-segment.schema.json)
- [schemas/ipc-command.schema.json](schemas/ipc-command.schema.json)
- [schemas/ipc-error.schema.json](schemas/ipc-error.schema.json)
- [schemas/ipc-hello.schema.json](schemas/ipc-hello.schema.json)
- [schemas/ipc-query-response.schema.json](schemas/ipc-query-response.schema.json)
- [schemas/ipc-query.schema.json](schemas/ipc-query.schema.json)
- [schemas/ipc-receipt.schema.json](schemas/ipc-receipt.schema.json)
- [schemas/ipc-subscribe-events.schema.json](schemas/ipc-subscribe-events.schema.json)
- [schemas/operation-name.schema.json](schemas/operation-name.schema.json)
- [schemas/role.schema.json](schemas/role.schema.json)
- [schemas/skill.schema.json](schemas/skill.schema.json)
- [schemas/test-execution-observation.schema.json](schemas/test-execution-observation.schema.json)
- [schemas/workflow-v1.1.schema.json](schemas/workflow-v1.1.schema.json)
- [schemas/workflow-v1.2.schema.json](schemas/workflow-v1.2.schema.json)
- [schemas/workflow.schema.json](schemas/workflow.schema.json)

## skills

- [skills/goal/SKILL.md](skills/goal/SKILL.md) — /goal — agent root autonome

## specs

- [specs/acceptance-criteria.md](specs/acceptance-criteria.md) — Critères d'acceptation de la V1 complète
- [specs/agent-hierarchy.md](specs/agent-hierarchy.md) — Hiérarchie agente et décisions — spécification acceptée (ADR-12)
- [specs/ai-guard.md](specs/ai-guard.md) — AI-Guard : frontière retenue côté Unleash, contrat externe à qualifier
- [specs/browser-automation.md](specs/browser-automation.md) — Automatisation navigateur réutilisable — spécification R3
- [specs/connectors.md](specs/connectors.md) — Connecteurs documentaires — Explore, Pin, Scope, Watch
- [specs/contract-enforcement.md](specs/contract-enforcement.md) — Moteur de contrats et preuves
- [specs/document-package-ingestion.md](specs/document-package-ingestion.md) — Ingestion du dossier .claude/ par une application
- [specs/git-integration.md](specs/git-integration.md) — Spécification — coordination et publication Git
- [specs/goal-management.md](specs/goal-management.md) — Goals, missions, done et clôture
- [specs/gui-cli-tui.md](specs/gui-cli-tui.md) — GUI, CLI et TUI : même produit et parité fonctionnelle complète
- [specs/local-models.md](specs/local-models.md) — Modèles locaux — contrat proposé pour la V1
- [specs/model-routing.md](specs/model-routing.md) — Model Router et contexte effectivement transmis
- [specs/product-requirements.md](specs/product-requirements.md) — Exigences produit et convention normative
- [specs/reusable-automations.md](specs/reusable-automations.md) — Automatisations réutilisables — contrat produit proposé R3
- [specs/role-registry.md](specs/role-registry.md) — Role Registry — capacité de premier ordre V1
- [specs/scheduling.md](specs/scheduling.md) — Ordonnancement global des exécutions
- [specs/sessions-and-recovery.md](specs/sessions-and-recovery.md) — Contrat de continuité et de reprise
- [specs/shared-knowledge.md](specs/shared-knowledge.md) — Spécification — connaissance partagée et découverte avant création
- [specs/skills-and-roles.md](specs/skills-and-roles.md) — Skills et rôles dans le travail quotidien
- [specs/terminal-management.md](specs/terminal-management.md) — Gestion du pool de terminaux et services
- [specs/traceability.md](specs/traceability.md) — Spécification — traçabilité, preuves et rapports
- [specs/workflow-compiler.md](specs/workflow-compiler.md) — Compilateur de workflow depuis le langage naturel
- [specs/workflow-engine.md](specs/workflow-engine.md) — Moteur de workflow déterministe

## testing

- [testing/acceptance-scenarios.md](testing/acceptance-scenarios.md) — Scénarios d'acceptation et couverture de l'annexe
- [testing/automation-replay-tests.md](testing/automation-replay-tests.md) — Qualification des automatisations réutilisables et supervisées
- [testing/brief-coverage.md](testing/brief-coverage.md) — Couverture du brief, de l'annexe et des clarifications
- [testing/concurrency-tests.md](testing/concurrency-tests.md) — Tests de concurrence et d'ordonnancement
- [testing/contract-tests.md](testing/contract-tests.md) — Tests transversaux des contrats
- [testing/crash-recovery-tests.md](testing/crash-recovery-tests.md) — Campagne de pannes et restauration
- [testing/document-validation.md](testing/document-validation.md) — Rapport de validation documentaire
- [testing/integration-tests.md](testing/integration-tests.md) — Qualification d'intégration — parcours A à O
- [testing/invariant-coverage.md](testing/invariant-coverage.md) — Matrice exhaustive des invariants
- [testing/local-model-tests.md](testing/local-model-tests.md) — Modèles locaux — plan de qualification F012
- [testing/security-tests.md](testing/security-tests.md) — Qualification sécurité : mécanisme, frontière et preuve
- [testing/test-strategy.md](testing/test-strategy.md) — Stratégie de vérification du produit
- [testing/traceability.json](testing/traceability.json)
- [testing/traceability.md](testing/traceability.md) — Matrice Requirement → Architecture → Feature → Test → Evidence
- [testing/workflow-contract-document-validation.md](testing/workflow-contract-document-validation.md) — Contrôle documentaire — workflows, agents, skills et contrats
