# F015 — dépendances

## Contrats métier

| Feature / service | Interaction attendue en CLI et TUI |
| --- | --- |
| F001/F002 | Daemon, client IPC commun `unleash-client`, reçus, snapshots, projections et événements durables. |
| F003/F004 | Agents logiques, sessions/invocations, goals/missions avec owner user ou agent, conversations, interventions et GuidedSetup idempotent sans agent préalable. |
| F005/F006 | Catalogue/éditeur WORKFLOW, compilation, validation, simulation, bindings, versions et runs. |
| F007/F008 | Éditeur SKILL, registre de rôles, composition, contrats, évaluations, diff et publication. |
| F009/F010 | Recherche, provenance, discovery, dépendances, branches/worktrees et intégrations Git. |
| F011 | Inventaire processus, logs/PTY, process_run_uuid, leases et arrêt contrôlé. |
| F012/F013 | Profils locaux/distants, auth/SecretRef conditionnelles, localité/capacités/santé, tests explicites et contexte transmis ; documents, ressources, OAuth et connecteurs. |
| F014 | Référentiel commun des 58 groupes dans dix espaces, guidage/expertise et fixtures de parité ; aucune dépendance d’exécution à la GUI ou à sa WebView. |
| F016/F017 | Rapports, preuves, audit, politiques, identité client, permissions et AI-Guard. |
| F018 | Pannes/reconnexion, réconciliation, sauvegardes, rétention et reprise. |
| F019 | 11 commandes/5 lectures propres du registre d’automatisation, données versionnées, qualification/publication/réparation ; Run et reducer communs, replay forbidden et supervision bornée. |

La TUI dépend de toutes les façades requises par la GUI. Remplacer une absence de service par une injonction « ouvrir la GUI » ne satisfait pas la V1.

## Bibliothèques et qualification

Pile adoptée ([ADR-15](../../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) : Ratatui pour rendu/widgets, Crossterm pour événements et terminal, clap 4 (API derive) pour la CLI. Documentation observée : Ratatui 0.30.2, Crossterm 0.29.0, Rust minimum 1.88 pour cette version de Ratatui. Les [sources, alternatives et limites](technical-design.md#sources-officielles-et-observations) sont documentées ; ces observations ne remplacent ni un lockfile ni une compilation.

Les composants de texte/édition, diff et rendu PTY suivent la règle de choix d’ADR-15 : bibliothèques adaptées lorsqu’elles satisfont les besoins, sinon module interne. Leur choix exige vérification maintenance/licence/API, compatibilité Rust, Unicode, capacité à borner les ressources et intégration au cycle terminal. L’application ne réécrit ni ncurses, ni parser terminal, ni moteur de base de données. Les technologies de présentation de la GUI sont indépendantes du choix terminal.

La qualification évite plusieurs versions incompatibles de Crossterm, vérifie les entrées et le retour au terminal réel, et couvre la matrice de terminaux de la [conception terminal](../../design/cli-tui-design.md#matrice-terminal-qualifiée-v1) ([ADR-14](../../architecture/adr/ADR-14-local-data-oss.md), amendement R7) : Terminal.app, iTerm2, Ghostty, terminal Ubuntu par défaut et tmux ≥ 3.4 en local et via SSH. Le support amont d’une plateforme ne constitue pas une preuve de support produit.

## Travail parallèle et sortie de chantier

Les fixtures sont verrouillées avec le contrat. Doubles et stubs permettent le travail parallèle, mais chaque service externe demande sa recette réelle. Un changement de schéma est annoncé aux trois clients et versionné s’il est incompatible.

La [matrice de parité](../../design/gui-tui-parity.md) et les **17 critères F015** forment le contrôle de couverture. Les 50 groupes avancés R2 sont conservés ; six groupes d’automatisation, un de guidage et un de modèles locaux portent le total à 58. La V1 complète ne peut être déclarée tant qu’une opération métier, un éditeur, un parcours guidé ou une reprise requis reste absent de la TUI. Voir le [chemin critique](../../plans/critical-path.md).

