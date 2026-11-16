# Couverture du brief, de l'annexe et des clarifications

Cette matrice conserve les capacités demandées, indépendamment de leur ordre de construction. Les IDs INV détaillés sont dans [invariant-coverage](invariant-coverage.md) ; les exigences atomiques et preuves sont dans [traceability](traceability.md). Les critères indiqués sont à réaliser, sauf les gates documentaires dont le résultat est dans document-validation.

## Brief initial

| Origine | Capacités couvertes | Invariants | Livrable principal | Feature | Acceptation |
| --- | --- | --- | --- | --- | --- |
| §1.1 | Cœur Rust ; réutilisation de bibliothèques, HTML/CSS et présentation motivée autorisés par R2 | INV-001/002 | [architecture/rust-workspace.md](../architecture/rust-workspace.md) | F001 F003 F009 F011 F014 F015 | AC-F001-008; AC-E2E-T/U |
| §1.2 | API avec authentification adaptée, OpenAI/Anthropic, compatibilité qualifiée et priorité modèles | INV-003/004/005 | [specs/model-routing.md](../specs/model-routing.md) | F012 | AC-F012-001 à012 |
| §1.3–1.4 | V1 complète, incréments distincts, contradiction et statuts | INV-006/048/050 | [plans/v1-implementation-plan.md](../plans/v1-implementation-plan.md) | Toutes | DOC-01/04 |
| §2 | Vision local-first ; modèle décide, harness mécanique | INV-007/032 | [architecture/system-overview.md](../architecture/system-overview.md) | F001 F002 F005 | AC-F005-002; AC-F001-002 |
| §3.1 | N+2 organisation, rapports, budgets, staffing et skills, N directs, réveils utiles | INV-008/009/010/022 | [specs/agent-hierarchy.md](../specs/agent-hierarchy.md) | F004 F007 | AC-F004-002/006; AC-F007-004 |
| §3.2–3.4 | N+1 full feature, équipes dynamiques, N spécialisés, frontières et cycles | INV-008/011/019 | [specs/goal-management.md](../specs/goal-management.md) | F003 F004 | AC-F004-001/003/008 |
| §4.1–4.3 | NL vers workflow compilé, étapes libres, décisions leader, parallélisme | INV-014/015 | [architecture/workflow-dsl.md](../architecture/workflow-dsl.md) | F005 F006 | AC-E2E-B/C |
| §4.4 | Done utilisateur, niveaux de complétion, pas de juge/tests/audit imposés | INV-016/017 | [specs/workflow-engine.md](../specs/workflow-engine.md) | F005 F008 | AC-E2E-L |
| §4.5 | Version engagée stable, invocation/étape/run/cycle distingués | INV-018/053 | [architecture/session-model.md](../architecture/session-model.md) | F003 F005 F007 | AC-E2E-K |
| §5.1–5.3 | Skill/rôle/agent/session distincts ; réutilisation ; N+2 gouverne | INV-019/020/022/024 | [specs/skills-and-roles.md](../specs/skills-and-roles.md) | F004 F007 | AC-E2E-E/P/Q |
| §5.4–5.5 | Studio complet, édition/diff/évaluation/import, contrats et trace | INV-023/026/027 | [design/skill-studio.md](../design/skill-studio.md) | F007 F008 F016 | AC-F007-003/005/006/008/010 |
| §6 | Observations d'outils, déclarations, rapprochement, validations formelles et limites | INV-026/027 | [specs/contract-enforcement.md](../specs/contract-enforcement.md) | F008 F016 | AC-E2E-F |
| §7.1–7.4 | Code/Work/Evidence Intelligence, branches nonmergées, provenance, réemploi sans mutualisation forcée | INV-024/025/028/029 | [specs/shared-knowledge.md](../specs/shared-knowledge.md) | F009 | AC-E2E-D/Q |
| §8.1–8.3 | Worktrees, scopes, leases ressources, intégration spécialisée et conflits fonctionnels | INV-030/031 | [specs/git-integration.md](../specs/git-integration.md) | F010 F004 | AC-E2E-A/G/L |
| §9.1–9.2 | Daemon user, macOS, launchd, SQLite WAL, queues/migrations/backup, pas Redis par défaut | INV-032/033 | [architecture/sqlite-design.md](../architecture/sqlite-design.md) | F001 F002 | AC-F001-001/002/006; AC-F002-001 |
| §9.3 | 5/50, managers comptés, agents logiques, quotas/budgets/priorités | INV-012/013/051 | [specs/scheduling.md](../specs/scheduling.md) | F003 F012 | AC-E2E-I/R |
| §9.4–9.5 | Session durable, worker perdu, PID réutilisé, enfants, crash et effets ambigus | INV-034/035 | [architecture/failure-recovery.md](../architecture/failure-recovery.md) | F003 F011 F018 | AC-E2E-H/N |
| §10 | Événements persistants, dispatch automatique vs invocations, reprise | INV-007/009/045 | [architecture/event-model.md](../architecture/event-model.md) | F002 F004 | AC-F004-005/006; AC-E2E-J |
| §11 | Pool terminaux/processus, ownership, ports/logs, PTY/groupes et broker | INV-036/037 | [specs/terminal-management.md](../specs/terminal-management.md) | F011 F017 | AC-F011-001 à010; AC-E2E-T |
| §12.1–12.3 | Router, capacités fournisseurs, budget/retries/fallback, mémoire/contextes propres | INV-003/004/005/044/051 | [research/verified-api-capabilities.md](../research/verified-api-capabilities.md) | F012 F003 | AC-F012-001 à012 |
| §13 | Drive/OneDrive/Notion, Explore/Pin/Scope/Watch, OAuth et accès explicite | INV-038/039 | [specs/connectors.md](../specs/connectors.md) | F013 | AC-E2E-M |
| §14 | AI-Guard connexion/admin/santé/politiques/audit/révocation, trois frontières | INV-040/052 | [specs/ai-guard.md](../specs/ai-guard.md) | F017 | AC-E2E-V; EXT-GUARD |
| §15.1–15.2 | Backend commun ; dix espaces GUI, état, conversations, Knowledge/Settings | INV-041/042 | [design/gui-screen-specifications.md](../design/gui-screen-specifications.md) | F014 F015 | AC-F014-001/002; AC-E2E-O |
| §15.3–15.5 | Message N+1 visible N+2, contexte réel, intervention et historique | INV-043/044/045 | [design/interaction-flows.md](../design/interaction-flows.md) | F004 F014 F016 | AC-E2E-J/O |
| §16 | Rapports par niveau, navigation causale, journal+état vs event sourcing | INV-026/045 | [specs/traceability.md](../specs/traceability.md) | F002 F016 | AC-F016-001 à012 |
| §17 | Quinze scénarios obligatoires A–O | INV-048 | [testing/acceptance-scenarios.md](../testing/acceptance-scenarios.md) | Toutes | AC-E2E-A àO |
| §18 | Quatorze mandats experts, conclusions/objections/alternatives/acceptation | INV-050 | [agents/README.md](../agents/README.md) | Revue | DOC-04 |
| §19 | Phases A–H : invariants, recherche, audits, red team, arbitrage, arch, features, cohérence | INV-048/050 | [audits/consolidated-verdict.md](../audits/consolidated-verdict.md) | Conception | DOC-01 à07 |
| §20–20.3 | Arborescence minimale initiale, enrichie à 19×7 fichiers feature, ADR et protocoles agents | INV-046/047/048 | [INDEX.md](../INDEX.md) | Toutes | DOC-01/06 |
| §21 | IDs stables, matrices, sécurité mécanisme/frontière/autorité/test, absence de code signalée | INV-048/052 | [testing/traceability.md](../testing/traceability.md) | Toutes | DOC-02/03/05 |
| §22 | 25 questions explicites avec solutions et alternatives | INV-048/050 | [audits/adversarial-findings.md](../audits/adversarial-findings.md) (réponses R1 historiques ; état courant : [registre](../brief/open-questions.md) et ADR) | Toutes | DOC-04 |
| §23 | Critères de réussite documentaire et démarage développeur | INV-047/048 | [plans/readiness-checklist.md](../plans/readiness-checklist.md) | Conception et release | DOC-01 à07 |
| §24 | Simplicité locale, autonomie, aucun retrait V1, fichiers réels et décisions ouvertes | INV-006/007/009/046 | [README.md](../README.md) | Toutes | DOC-01/07 |

## Annexe obligatoire

| Annexe | Précision | Invariants | Livrable principal | Feature | Acceptation |
| --- | --- | --- | --- | --- | --- |
| A | Rust interne vs projets multilangages et outils externes | INV-001/002 | [architecture/rust-workspace.md](../architecture/rust-workspace.md) | F003 F009 F011 | AC-E2E-T |
| B | Rôles first-class persistants/versionnés, scopes et priorité | INV-020/021 | [specs/role-registry.md](../specs/role-registry.md) | F007 | AC-E2E-P |
| C | Recherche avant créations transversales, trace harness et justification | INV-024/025 | [schemas/discovery-receipt.schema.json](../schemas/discovery-receipt.schema.json) | F007 F008 F009 | AC-E2E-Q |
| D | États branches/dirty/intégration, fraîcheur/provenance, reconciliation | INV-027/028/029 | [architecture/knowledge-indexing.md](../architecture/knowledge-indexing.md) | F009 F010 | AC-E2E-D/N |
| E | N+2 gouverne/délègue conception technique skills, édition humaine | INV-022/023 | [specs/skills-and-roles.md](../specs/skills-and-roles.md) | F004 F007 | AC-E2E-E |
| F | Manager urgent parmi 5 N, équité/quotas/réservation/préemption sans dépassement | INV-012/013/051 | [specs/scheduling.md](../specs/scheduling.md) | F003 F012 | AC-E2E-I/R |
| G | Appel/invocation/étape/cycle/run et versions skills/rôles | INV-018/053 | [architecture/session-model.md](../architecture/session-model.md) | F003 F005 F007 | AC-E2E-K/P |
| H | Documentation .claude distincte config/données/SQLite/sessions/artefacts/temp | INV-046 | [architecture/storage-model.md](../architecture/storage-model.md) | F001 F002 | AC-F001-008 |
| I | Clés/coffre/socket/projets/scopes/redaction/injection/rétention/OAuth/Guard | INV-052 | [architecture/security-boundaries.md](../architecture/security-boundaries.md) | F013 F017 | AC-E2E-S/V |
| J | Manifestes complets agent/rôle/session/modèle/skills/workflow/sources/causes | INV-044/053 | [schemas/context-manifest.schema.json](../schemas/context-manifest.schema.json) | F003 F012 F016 | AC-F003-002; AC-F012-009 |
| K | GUI CLI TUI backend commun et états hors ligne/dormant/erreur/reprise | INV-041/042 | [specs/gui-cli-tui.md](../specs/gui-cli-tui.md) | F014 F015 | AC-E2E-O |
| L | Conception pas développement ; structures/interfaces/schémas/tests ; pas dépôt inventé | INV-047 | [architecture/rust-workspace.md](../architecture/rust-workspace.md) | Conception | DOC-02/05/06 |
| M | Matrice exhaustive, seconde revue, invariants préservés et dérogations proposées | INV-048/050 | [testing/invariant-coverage.md](../testing/invariant-coverage.md) | Conception | DOC-01/03/04 |

## Clarifications finales de Louis

Le nom OVPT Unleash (INV-055), la destination open source (INV-049), la politique Rust pragmatique (INV-001), la qualité GUI/TUI (INV-054), la parité complète (INV-056) et la TUI aboutie (INV-057) sont intégrés à la vision, au scope, au design et aux gates V1. La clarification R2 identifie Kivy et Tauri, résout Q-01/Q-17 et autorise les bibliothèques et HTML/CSS. ADR-03 fixe le choix technique (Q-02 résolue R7 : Tauri 2 + React/TypeScript) ; aucune réduction de périmètre terminal n’est déduite de ses contraintes de rendu.

## Contrôle des énumérations à risque d'oubli

Les dix espaces GUI sont tous décrits : Overview, Agents, Conversations, Workspaces, Workflows, Skills Studio, Activity, Reports, Knowledge, Settings. Les trois connecteurs sont inclus, y compris OneDrive. Les cinq langages de projets cités et les monorepos mixtes ont un scénario de qualification. Les modes Explore, Pin, Scope et Watch ont des contrats, limites API et états UX. L'édition manuelle des skills/rôles et la discussion directe N+1 restent accessibles.

Le registre des processus couvre propriétaire, identité au-delà du PID, cwd, commande, ports connus, logs, historique et arrêt/reprise. Les événements distinguent fait observé, état persistant, transition automatique et besoin de jugement N+1/N+2/humain. Les rapports distinguent déclarations, observations et décisions. Les outils externes ne sont pas interdits par la contrainte de langage interne.

## Quatre axes de contradiction renforcée

| Axe | Contre-exemple traité | Correction / alternative | Test |
| --- | --- | --- | --- |
| Worktrees | Archive dirty figée mais mélange temporel ; helper trouvé absent localement | Capture_consistency + disponibilité + commits/trees cohérents ; snapshot OS qualifié en alternative | T-F009-002/007 ; T-E2E-D |
| Invocation N+1 | Parent attend enfants en conservant tous slots ; version nouvelle au milieu d'un run | Checkpoint et attente durable ; run figé, late slots explicites ; réserve dure en alternative | T-F003-004/005 ; T-F005-008/010 |
| Reprise | Effet accompli sans résultat ; backup ancien face à host plus récent | OutcomeUnknown, réconciliation, nouvelle lignée après neutralisation ; high-water externe en alternative | T-F018-004/007 ; T-E2E-N |
| Skills/contrats | Prompt plausible, tests déclarés mais non exécutés ; rôle sans skill d'entrée | Profil fermé, entrée exacte, preuves observées, digests et évaluateur épinglés ; profil de sous-typage fermé en alternative | T-F008-002/005/007 ; T-F007-016 |

Aucune simplification retenue n'autorise à clôturer la V1 avec un connecteur supprimé, une TUI sans une fonction avancée de la GUI, un rôle éphémère, un AI-Guard simulé présenté comme réel ou une GUI jugée suffisante uniquement parce qu'elle compile en Rust.


## Clarification R2 — correspondance explicite

| Décision utilisateur | Invariants | Documents normatifs | Features / acceptation | Limites et preuves attendues |
| --- | --- | --- | --- | --- |
| Rust dans la mesure de l’intelligent ; bibliothèques et HTML/CSS | INV-001 | [politique](../brief/implementation-policy.md), [workspace](../architecture/rust-workspace.md), ADR-15 | F001, F014, F015 ; AC-F001-008 | Cœur Rust confirmé ; renderer et dépendances qualifiés, pas de métier dupliqué |
| Références Kivy et Tauri | INV-054, Q-17 résolue | [comparaison](../research/technology-comparison.md), ADR-03 | F014 ; AC-F014-009/010/011/014 | Noms identifiés, sélection acceptée (ADR-03), sources datées |
| CLI iso GUI : TUI complète | INV-041/042/056 | [parité](../design/gui-tui-parity.md), [catalogue commun](../architecture/client-operation-catalog.md) | F014/F015 ; AC-F014-012, AC-F015-001/010/011/012/016, AC-E2E-O | Socle R2 de 50 groupes, étendu à 58 par R3–R5, avec chaque verbe testé ; création projet, reprise d’effet inconnu et resize compris |
| Beau terminal ncurses ou mieux | INV-054/057 | [conception terminal](../design/cli-tui-design.md), ADR-15 | F015 ; AC-F015-009/013/014/015, AC-E2E-U | Ratatui/Crossterm acceptés (ADR-15) ; accessibilité, Unicode, restauration et charge non encore mesurés |

La [revue indépendante R2](../audits/frontend-terminal-revision-review.md) consigne les objections et leur relecture. Les changements de libellés publics sont définis dans l’IPC avant implémentation, sans alias implicite ni protocole spécifique à une interface.

## Clarifications R3–R5 — couverture atomique

| Demande utilisateur | Invariants | Documents normatifs | Features / acceptation | Limites et preuves attendues |
| --- | --- | --- | --- | --- |
| Développeur et usages personnels/professionnels | INV-058 | [scope](../brief/general-purpose-scope.md), [goals](../specs/goal-management.md) | F004/F019 ; AC-F004-011, AC-E2E-Z | Mission sans repo ni agent ; aucun métier universel prétendu |
| Apprentissage puis rejeu de nouvelles données sans LLM | INV-059 | [profil exécutable](../architecture/automation-execution-profile.md), [artefact](../architecture/automation-artifact-model.md) | F019 ; AC-F019-001 à006, AC-E2E-W | Données immuables, interdiction transitive, preuve réseau/dispatch indépendante |
| Navigateur DOM/locators, moteur existant et script réutilisable | INV-062 | [BrowserPort](../architecture/browser-execution.md), [sources](../research/browser-automation-sources.md) | F019/F011/F017 ; AC-F019-009/010/014/017 | Références de snapshot non persistantes, assertions métier, distinction script/médiation |
| Refaire l’analyse si le site évolue | INV-060 | [reprise et réparation](../features/F019-reusable-automations/technical-design.md) | F019/F018 ; AC-F019-011/012/013 | Effet inconnu non répété, nouvelle version, politique de réparation préautorisable |
| LM Studio, Ollama et « CPP » | INV-003/063 | [modèles locaux](../specs/local-models.md), ADR-17 | F012 ; AC-F012-013 à018, AC-E2E-Y | Dialectes, auth et placement réels ; llama.cpp est l’interprétation explicite du dernier nom |
| Supervision LLM par logs/résultats sans clic LLM permanent | INV-064 | [profil](../architecture/automation-execution-profile.md), ADR-16 | F004/F019 ; AC-F004-012, AC-F019-001/010, AC-E2E-X | Checkpoints/exception déclarés ; logs corroborés, budgets, décisions bornées |
| Parcours accessible sans savoir configurer workflow/agents | INV-061/065 | [guidage](../design/guided-automation-experience.md), [catalogue](../architecture/client-operation-catalog.md) | F004/F014/F015 ; AC-F004-012, AC-F014-015/016, AC-F015-017, AC-E2E-Z | Préparation idempotente, aucune action externe avant lancement, parité TUI |
| Procédure par défaut avec contrôles adaptés | INV-016/017/065 | [expérience guidée](../design/guided-automation-experience.md), [automatisations](../specs/reusable-automations.md) | F005/F019/F014 ; AC-E2E-L/Z | Template proposé, modifiable ; aucun audit universel ni promesse de qualité absolue |

Les profils généralistes s’appuient sur les mêmes contrats, preuves, autorisations, ressources et sessions. Les campagnes A–V restent présentes. Les fichiers de la révision ne remplacent pas le dossier de code par une plateforme RPA séparée et ne réduisent pas les trois connecteurs, la parité TUI, l’intégration AI-Guard ou la V1 initiale.
