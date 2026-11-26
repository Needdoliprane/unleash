# Exigences produit et convention normative

Sources : brief principal §§1–24, annexe A–M et clarifications utilisateur jusqu’à R7 : OVPT Unleash, Rust avec discernement, GUI/TUI complètes, automatisations réutilisables et supervisées, modèles locaux et démarrage guidé. Inventaire contractuel [INV-001 à INV-065](../brief/invariants.md). Les propositions de solution n’ont pas le statut d’invariant confirmé ; les décisions acceptées en [R7](../brief/revision-2026-11-16-r7.md) font autorité pour l’implémentation sans devenir des invariants.

## Niveaux d’identifiants

INV : contrainte utilisateur stable. REQ : comportement implémentable. AC : critère Given/When/Then. T : test proposé. EV : preuve attendue, jamais présumée existante. ADR : décision `proposed`, `accepted` ou `superseded`, avec ses alternatives. R/H/Q : risque, hypothèse et question. Une nouvelle interprétation d’un invariant exige arbitrage explicite ; elle ne se fait pas en éditant sa formulation pour faire passer un test.

## Catalogue des exigences normatives

| Famille | Document propriétaire | Composant |
| --- | --- | --- |
| REQ-CORE | [F001](../features/F001-core-daemon/requirements.md) | Daemon |
| REQ-PERSIST | [F002](../features/F002-persistence-events/requirements.md) | Store/journal |
| REQ-AGENT | [F003](../features/F003-agent-runtime/requirements.md) | Runtime/scheduler |
| REQ-ORCH | [F004](../features/F004-hierarchical-orchestration/requirements.md) | Organisation |
| REQ-WF | [F005](../features/F005-workflow-engine/requirements.md) | Machine |
| REQ-COMP | [F006](../features/F006-workflow-compiler/requirements.md) | Compiler |
| REQ-SKILL / REQ-ROLE | [F007](../features/F007-skill-studio-registry/requirements.md) | Registres distincts |
| REQ-CONTRACT | [F008](../features/F008-contract-engine/requirements.md) | Contrats |
| REQ-KNOW | [F009](../features/F009-knowledge-engine/requirements.md) | Knowledge |
| REQ-GIT | [F010](../features/F010-git-coordination/requirements.md) | Git |
| REQ-TERM | [F011](../features/F011-terminal-pool/requirements.md) | Processus |
| REQ-MODEL | [F012](../features/F012-model-api-router/requirements.md) | Model Router |
| REQ-DOC | [F013](../features/F013-document-connectors/requirements.md) | Documents |
| REQ-UI | [F014](../features/F014-gui/requirements.md) | GUI |
| REQ-CLI | [F015](../features/F015-cli-tui/requirements.md) | CLI/TUI |
| REQ-OBS | [F016](../features/F016-reporting-observability/requirements.md) | Rapports |
| REQ-SEC | [F017](../features/F017-ai-guard-integration/requirements.md) | Autorité/AI-Guard |
| REQ-REC | [F018](../features/F018-reliability-recovery/requirements.md) | Reprise |
| REQ-AUTO | [F019](../features/F019-reusable-automations/requirements.md) | Automatisations réutilisables |

Cette table pointe vers les définitions uniques, plutôt que recopier des règles susceptibles de diverger. La [matrice](../testing/traceability.md) relie individuellement les exigences à architecture, feature, tests et preuves.

## Exigences transversales

Le cœur métier et système, les clients terminal et les tests du cœur sont Rust ; les bibliothèques existantes et HTML/CSS sont permis selon la [politique d’implémentation](../brief/implementation-policy.md). Les choix de présentation et d’outillage sont documentés sans réimplémenter leurs moteurs. Le runtime traite des projets multilangages. GUI et TUI couvrent le même catalogue fonctionnel, y compris édition et configuration. L’API modèle, les interfaces et AI-Guard ne possèdent pas les états de workflow. Les droits ne sont pas des préférences surchargées. Les sorties modèles restent déclarations tant qu’aucune preuve ne les étaye.

La V1 est complète seulement après tous les critères des features et scénarios transversaux. Une feature peut être développée sur mock puis intégrée ; son état mock-qualified n’est pas completed. Une limite documentée doit conserver la capacité fonctionnelle en offrant un comportement vérifiable, par exemple réconciliation guidée d’un effet inconnu.

## Préparation open source

INV-049 et INV-055 exigent l’attribution du produit OVPT Unleash et prévoient sa publication open source. Q-06 fixe la licence Apache-2.0 et la politique de marque. La mention `Signed-off-by` est facultative pour les contributions. EXT-OSS vérifie la compatibilité des dépendances. Le plan inclut la documentation d’installation, de migration et d’exploitation, la politique de sécurité, les notices et des artefacts reproductibles. [ADR-14](../architecture/adr/ADR-14-local-data-oss.md) et [ADR-20](../architecture/adr/ADR-20-contribution-signoff.md)

Le dossier vit dans le dépôt de réalisation, à côté du code ; le produit ne lit jamais `.claude/` à l’exécution et ne dépend pas de Claude Code. Les données runtime privées, clés et historiques opérationnels ne doivent pas être ajoutés à .claude ni au dépôt ; les preuves réelles de qualification vont dans `qualification/evidence/`.

## Compléments transversaux R3–R5

Le même moteur sert les missions de développement et les procédures générales. Une mission peut avoir un propriétaire humain et aucun repository ; une procédure mécanique peut fonctionner sans agent actif. Le mode replay interdit les appels modèles locaux ou distants ; la supervision les autorise aux seules tâches publiées et sous budgets. Le parcours guidé prépare les mêmes objets que le parcours expert et reste intégralement accessible en TUI. Le template recommandé ne modifie pas les règles de complétion d’un workflow personnalisé. Les critères détaillés relèvent de AC-F004-011/012, AC-F012-013 à 018, AC-F014-015/016, AC-F015-017 et AC-F019-001 à 018.
