# Dépendances de réalisation

Lire aussi la [carte des composants](../architecture/dependency-map.md). Les dépendances ci-dessous expriment ce qui est nécessaire pour **intégrer** les 19 features ; elles n’empêchent pas de développer des contrats et fixtures en parallèle. F001–F018 restent dans la V1. Un projet sans Git et un replay sans agents sont des compositions valides du même socle.

| Feature | Dépendances d’intégration | Peut avancer avant |
| --- | --- | --- |
| F001 Core | Types et protocole communs | Packaging daemon, répertoires et client de diagnostic |
| F002 Persistance | F001 lifecycle | Schémas, transactions et crash tests isolés |
| F003 Runtime | F001 F002 ; ports F011/F012/F017 | Scheduler mixte agent_invocation/replay_worker/provider_probe sur horloge simulée |
| F004 Hiérarchie | F003 F005 F007 F008 | Règles de délégation et mailbox persistante |
| F005 Workflow | F002 F008 ; port F003 | Interpréteur pur, profils v1/v1.1/v1.2, actions et supervision déclarée |
| F006 Compiler | F005 ; port LLM F012 pour conception assistée | Validation/compilation mécanique sans LLM |
| F007 Skills/rôles | F002 F008 ; F009 pour découverte enrichie | Registry, versions, résolution et évaluations isolées |
| F008 Contrats | F002 ; ports outils F017/F011 | Validateurs formels et schémas |
| F009 Connaissance | F002 ; F010 pour les repositories, F013 pour documents ; faits F008/F011/F016 | Parseurs, corpus, discovery et recherche locale sans dépôt obligatoire |
| F010 Git | F002 F011 F017 | Fixtures repos, candidats et CAS |
| F011 Processus | F001 F002 F017 | Process-host et protocole de supervision |
| F012 Modèles API/locaux | F002 et secrets/autorité F017 ; ressources F003/F011 | Adaptateurs HTTP, tuples LM Studio/Ollama/llama.cpp et corpus de qualification |
| F013 Documents | F002 F017 ; F003/F012 seulement pour contexte agent applicable | API fixtures, explorateur, imports autorisés et normalisation |
| F014 GUI | IPC F001, read models F016 ; domaines exposés | Design system et écrans de fixtures dès M0 |
| F015 CLI/TUI | IPC F001, read models F016 et mêmes domaines exposés que F014 ; référentiel de parité partagé, sans dépendance d'exécution à F014 | Parsing, rendu, reconnexion simulée |
| F016 Rapports | F002 et producteurs de faits | Projections, parcours causal et formats |
| F017 Guard/sécurité | Socle F001/F002 ; contrat réel AI-Guard pour qualification | Broker local, secrets, policies et simulateur explicite |
| F018 Fiabilité | Chaque effet de bord de F001, F002, F003, F004, F005, F006, F007, F008, F009, F010, F011, F012, F013, F014, F015, F016, F017 et F019 | Testkit, points de panne et protocoles de reprise dès M0 |
| F019 Automatisations | F002 F005 F006 F007 F008, admission/processus F003/F011, politique modèle F012, autorité/reprise F017/F018 ; F004 pour supervision/conception agente | Candidat manuel, CSV/contrats, fixture navigateur et profils replay/supervised ; F010 seulement si la procédure utilise Git |

## Éviter les cycles de dépendance Rust

Le tableau comporte des dépendances de comportement réciproques (skills cherchent dans knowledge, knowledge observe skills). Elles ne justifient pas des cycles de crates. Les ports typés et événements passent par domaine/protocole ; le daemon compose les implémentations. Un registry peut utiliser la découverte lexicale locale en attendant l’adaptateur d’index, sans retirer la recherche préalable du produit final.

Le Contract Engine émet des demandes d’observation ; il n’importe pas directement chaque backend. Le Workflow Engine reçoit des résultats de contrats ; le validateur ne pilote pas les transitions. Son module automation réutilise ce reducer, la DB et l'admission. La GUI, la TUI et la CLI consomment le protocole public. Le code fournisseur ou navigateur n’est jamais une dépendance de l’interpréteur pur des workflows. Les scripts utilisateur restent des artefacts externes ; leur langage n'oblige pas à créer un backend métier hors Rust.

## Dépendances externes bloquantes

AI-Guard exige ses fichiers et une instance qualifiable. Le client OAuth officiel exige l'enregistrement et les vérifications Google/Microsoft par OVPT (Q-08) ; la voie `bring_your_own` reste disponible. Les tests API réels exigent des comptes de test OVPT dédiés, un plafond de dépense chez le fournisseur et des clés au seul coffre de CI (Q-07). Les trois profils locaux nécessitent leurs serveurs/builds et modèles/configurations réels de la matrice Q-19 sur le matériel qualifié, sans inventer les capacités à partir du nom. Le navigateur nécessite Chrome for Testing et chromedriver épinglés, le mapping `fantoccini`, des credentials de fixture et le profil de confinement qualifiés (Q-18). La distribution macOS exige l'adhésion Apple Developer et les certificats Developer ID d'OVPT (Q-16, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md)). La licence Apache-2.0 étant décidée (Q-06), la publication exige l'inventaire des dépendances, y compris les runtimes externes, et la recherche d'antériorité du nom.

Ces dépendances sont suivies comme gates nommés dans [readiness-checklist](readiness-checklist.md). Une feature reste spécifiée pendant l’attente ; on ne prétend pas qu’un mock clôt sa recette externe.

## Dépendance de parité R2

F014 et F015 dépendent du même catalogue public, de la même négociation IPC et du client partagé `unleash-client`, seule voie d'accès au daemon. Le renderer GUI accepté (React/TypeScript, [ADR-03](../architecture/adr/ADR-03-rust-gui.md)) consomme des types générés depuis les DTO Rust et ne bloque pas le domaine Rust ; les bibliothèques existantes et HTML/CSS sont autorisés. L’éditeur terminal dépend des services de drafts/validation F006/F007, pas de l’éditeur graphique. La configuration connecteurs depuis TUI dépend du service d’authentification F013, pas d’une fenêtre GUI. La recette M5 exige les deux parcours complets et leurs retours d’erreur.

Le parcours guidé F019 dépend des commandes de création/réutilisation idempotentes de F004/F007, de l'aperçu d'effets du backend et des projections F016. Il ne dépend pas d'agents que le novice aurait déjà créés. Replay et supervision, sélection locale, credentials navigateur, erreurs par ligne et réparation ont leurs parcours terminaux équivalents. La visibilité d'un défaut recommandé ne crée aucune permission.

## Frontières critiques R3–R5

F019 dépend tôt de la qualification BrowserPort en M0 et du profil v1.1/v1.2 en M3 ; il ne peut être ajouté après la recette générale comme simple script. F012 local est qualifié en M2 avec local_only, OOM et ressources, puis utilisé par les tâches de conception/supervision autorisées. M5 assemble ces capacités dans les parcours guidés ; M7 réalise la recette W/X/Y/Z. Les risques de H-17 à H-21 et INV-058 à INV-065 restent suivis dans [risk-register.md](risk-register.md), sans imposer tests ou jugements supplémentaires aux workflows utilisateurs qui ne les demandent pas.
