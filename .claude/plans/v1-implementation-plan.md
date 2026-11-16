# Plan de réalisation de toute la V1

Statut : plan de construction R7, cadre accepté de la réalisation, pas engagement de dates ni résultat de tests produit. La réalisation commence en M0 dans ce dépôt ([démarrage M0](../dev/m0-kickoff.md)). La V1 comprend F001 à F019 : les 18 features antérieures sont conservées intégralement, F019 ajoute les automatisations réutilisables au même moteur. Les jalons internes ne sont pas des éditions amputées du produit. Les exigences normatives vivent dans les dossiers de features et sont reliées à leur preuve attendue dans [la traçabilité](../testing/traceability.md). INV-058 à INV-065 et H-17 à H-21 raccordent polyvalence, replay, supervision, modèles locaux et expérience guidée.

## M0 — Réduire les inconnues qui changent les fondations

Figer les types d’identité, la sémantique invocation/run/action, les erreurs IPC et le modèle d’autorité. Créer le workspace Cargo à la racine de ce dépôt (`crates/`), à côté de `.claude/`, à partir du [découpage Rust](../architecture/rust-workspace.md) ; le renderer GUI vit sous `apps/gui-renderer/`, les expériences sous `spikes/`, les rapports et preuves sous `qualification/`. La toolchain est épinglée par `rust-toolchain.toml` ; les dépendances sont verrouillées après qualification. Les versions documentées dans la recherche sont des observations, pas un Cargo.lock testé.

Exécuter cinq expériences courtes, avec critères annoncés avant résultat :

1. SQLite : writer unique, journal et outbox, crash injecté entre chaque commit ; vérifier la bibliothèque réellement liée et son source_id.
2. Interfaces : réaliser la tranche verticale de l’écran mission sur la pile acceptée Tauri 2 + React/TypeScript ([ADR-03](../architecture/adr/ADR-03-rust-gui.md)), avec arbre de 1 000 agents logiques, 100 000 événements paginés/virtualisés, diff de 10 000 lignes, rapport, timeline virtualisée et inspecteur (charge d’AC-F014-011). Évaluer esthétique, clavier, IME, accessibilité, consommation et maintenance ; réaliser aussi le parcours en TUI Ratatui/Crossterm ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)), avec édition intégrée, resize et restauration du terminal. Aucun candidat natif n’est construit hors déclenchement du réexamen d’ADR-03 sur défaut éliminatoire mesuré. La pile n’est pas qualifiée sur une capture de démonstration de sa documentation.
3. Runtime : worker et process-host séparés ; tuer daemon, host et enfant à des points identifiés ; observer ce qui reste réellement récupérable. Essayer aussi le mode `strict` de Q-05 sur process-host, et son refus lorsque le confinement manque.
4. Contrats et connaissance : un patch multilangage, un dirty worktree modifié pendant analyse, un faux rapport de tests ; prouver que le contrat indique inconnue ou insuffisance sans inventer une vérité.
5. Navigateur/F019 : qualifier sur une fixture le pilote de référence de Q-18, WebDriver W3C piloté en Rust (`fantoccini`) sur Chrome for Testing et chromedriver épinglés, avec son proxy de sortie ([exécution navigateur](../architecture/browser-execution.md)). Tester identité/cardinalité des cibles, clic techniquement valide au résultat métier faux, contexte/authentification, effet inconnu et arrêt/reprise. Distinguer profil médié et script utilisateur opaque ; mesurer les garanties d'isolation réellement disponibles, zéro appel modèle et limites du canal de contrôle. Aucun achat réel n'est nécessaire et aucune allowlist de domaine ne vaut interdiction d'achat.

Sortie : les questions Q-01 à Q-21 sont tranchées (Q-01 et Q-17 en R2, les autres en R7 par arbitrage délégué) ; inventaire des dépendances, contrats stables, résultats mesurés et ADR corrigés par supersession si une mesure les réfute. La tranche verticale M0 conditionne la construction intégrale des dix écrans. Préparer la matrice de Q-19 (machines de référence et poids, versions figées au gel M2) et les fixtures des six templates de Q-20. Les échanges AI-Guard, les enregistrements OAuth officiels (Q-08), l’adhésion Apple Developer (Q-16) et les comptes de test fournisseurs (Q-07) commencent dès M0 car leur délai est externe. Les mesures de moteur et de navigateur qualifient les choix acceptés ; elles ne les remplacent pas par une préférence.

## M1 — Cœur durable et autorité locale

Construire F001, la tranche transactionnelle F002, le broker interne F017 et le squelette F018. Livrer IPC authentifié, commandes idempotentes, états persistants, journal, outbox/inbox, storage version et sauvegarde/restauration. Les événements sont persistés dans la même transaction que l’état qu’ils décrivent.

Gate : un client disparaît après acceptation sans créer deux commandes ; un daemon tué redémarre avec l’état engagé ; une migration échouée n’ouvre pas une base incompatible ; un acteur non autorisé reçoit un refus historisé sans accès aux secrets. Les clients peuvent commencer avec un serveur de fixtures conforme au schéma, puis basculer vers le vrai service.

## M2 — Exécutions, fournisseurs API/locaux et processus

Construire F003, F011 et F012 avec session durable, Model Router unique et scheduler. Les adaptateurs OpenAI/Anthropic restent explicites ; un endpoint compatible ne reçoit que les capacités qualifiées. Ajouter les profils LM Studio, Ollama et llama.cpp : qualification du tuple serveur/build, modèle/révision disponible, template, quantification et configuration, capacités effectives et placement d'inférence. La voie initiale utilise un endpoint explicitement configuré ; aucun moteur n'est réécrit et aucun poids n'est téléchargé automatiquement à l'ouverture de Settings.

Toute admission réserve unité globale et ressources : agent_invocation, replay_worker ou provider_probe, 5 par défaut et 50 maximum au total. Permis d'action, contextes navigateur et générations locales sont des limites additionnelles. Le sous-plafond local `max_inflight=1` est le défaut V1 de la matrice Q-19, relevable seulement sur preuve consignée par tuple (mémoire, cold start, OOM et annulation). Les attentes durables ne rendent l'unité qu'après checkpoint et neutralisation ; un script continuant à agir ne se cache pas dans les auxiliaires.

Gate : 20 managers logiques, 5 unités, puis cinq N ou une charge mixte agents/replay avec urgence managériale ne créent ni sixième place ni deadlock parent/enfant. Les appels longs, 429, coupures SSE, résultats tardifs et facturations inconnues sont traités. Les versions de la matrice Q-19 sont figées. Les trois backends locaux ont leurs preuves distinctes, y compris capacités absentes, OOM, configuration devenue stale, proxy localhost vers cloud, refus `local_only` et absence de fallback distant silencieux. Un process-host recontacté n’est pas réidentifié par son PID seul. Tests déterministes, serveurs locaux réels et essais API distants restent distingués ; voir [tests locaux](../testing/local-model-tests.md).

## M3 — Workflows, compétences et équipes

Construire F005 et F008 puis F006, F007 et F004 à partir de contrats partagés, avec développement parallèle des modules indépendants. Intégrer le module F019 dans ce moteur : AutomationArtifact, AutomationDataRevision, profils `unleash.workflow.v1.1` et v1.2 (for_each et after_batch pour les données), actions typées, références exactes de drivers/compilateur et contrats métier. Le DSL reste fermé et borné ; les gardes inconnues ne deviennent pas vraies ; la terminaison utilisateur est respectée. Skills/rôles/automatisations ont des versions et leur cycle de publication. Une sélection tardive autorisée est un ajout immuable avant activation, pas une mutation cachée.

Qualifier d'abord une procédure manuelle rejouable sans agent, sans Git et sans modèle, puis la conception assistée et le mode supervised. Celui-ci compose `task leader_decision` et `choice` déjà publiés entre blocs mécaniques, avec fréquence/budgets, sorties d'observation typées et assertions. Un événement d'erreur ou un log ne crée pas un superviseur implicite. La réparation est un job distinct dans un mandat explicite ou préautorisé ; elle produit une version nouvelle sans hot swap.

Gate : scénarios B, I, J et K conservés ; volets workflow et compétences de C et E (T-F005-005, T-F007-001/003/004/005/017) exercés sans clore leurs EV-E2E ; publication v2 sans modification du run v1. Ajouter deux CSV sur une même procédure, erreur statique avant tout effet, plus de 50 actions séquentielles sous borne cumulative, zéro appel modèle transitif en replay, supervision bornée par checkpoints plutôt que par clics et effet inconnu non répété. Les traces distinguent logs, assertions, observations et jugement. Les [tests F019](../testing/automation-replay-tests.md) ne font pas d'un juge LLM une obligation universelle.

## M4 — Travail réel dans les repositories

Construire F010 et F009, enrichir F016. Worktrees isolés, intégration CAS protégée, cible checkout signalée `blocked_target_checkout` et jamais mise à jour (Q-10). Découverte obligatoire dans les contrats de création identifiables ; snapshots et disponibilité distingués ; index réconciliés après rebase, crash ou suppression.

Gate : scénarios A, C, D, E, F, G et L avec dirty worktree, doublon sémantique ambigu, conflit fonctionnel sans conflit textuel et target qui avance pendant revue ; cas Dev→Merge sans audit ajouté sur cible réelle ; cible checkout signalée `blocked_target_checkout` et refusée sans toucher le checkout humain (Q-10, T-F010-001/006). Aucune preuve d’un commit ancien ne valide automatiquement le nouveau candidat.

## M5 — Expérience complète et contexte documentaire

Achever F013, F014, F015 et F016, avec les parcours F019 de préparation, données, publication, replay, supervision et réparation. Les interfaces ont déjà progressé en parallèle dès M1 avec fixtures IPC. Les trois connecteurs fournissent Explore/Pin/Scope/Watch et une provenance réellement inspectable. Les dix espaces GUI et TUI accèdent à toutes les mêmes opérations, y compris éditeurs, configuration de modèles API/locaux et connecteurs ; la CLI les expose pour automatisation. La matrice de parité est un gate V1 complet. Le design visuel et terminal est un gate explicite, pas une amélioration postérieure.

Le [parcours guidé](../design/guided-automation-experience.md) permet de commencer depuis un résultat ou une procédure proposée, sans agents préconfigurés et sans connaissance du DSL. Le backend prépare ou réutilise rôles/objets de manière idempotente dans les permissions actuelles. L'aperçu montre données, effets, mode, inspections prévues, coûts et limites ; un défaut de template n'élargit pas un mandat. Le mode expert reste accessible depuis GUI et TUI, sans imposer Git ou une clé modèle à une procédure qui n'en a pas besoin.

Gate : scénarios M, O et U conservés, couverture complète de la matrice GUI/TUI ; messages N+1 visibles au N+2 sans réveil systématique. Ajouter premier parcours comptable/particulier/développeur, projet sans dépôt, modèle local disponible/inconnu, mode forbidden/supervised distingué et réparation compréhensible. Perte de connexion, agent dormant, fournisseur indisponible, document révoqué et replay d’événements sont traités visuellement. Revue visuelle et tests de compréhension sur build réel, pas seulement wireframe, confrontés aux seuils de Q-20 (sessions GUI et TUI).

## M6 — Qualification AI-Guard et sécurité

Achever l’adaptateur réel F017 à partir des pièces fournies par AI-Guard. Tester identité, décision, révocation, indisponibilité, corrélation d’audit et exécution sous la frontière choisie. Inclure contrôle du runtime navigateur, credentials séparés, DOM hostile, scripts/descendants, transport local/TLS et placement des données. Les protections internes n’attendent pas ce jalon ; une allowlist de domaine ne clôt pas l'autorisation d'un effet sensible.

Gate : contrat réel et versions documentés, scénarios de contournement évalués, mode `strict` (Q-05 : Seatbelt sur macOS, bubblewrap avec seccomp sur Linux, egress par le proxy local d’Unleash) refusé si la frontière requise manque. Un simulateur peut vérifier le comportement d’Unleash face au contrat simulé ; il ne clôt pas la compatibilité réelle AI-Guard. Cette dépendance reste dans la V1.

## M7 — Recette complète, distribution et exploitation

Achever F018 de manière transversale : pannes, charge mixte, restauration, migrations, rétention, accès API, sur la matrice qualifiée d’[ADR-14](../architecture/adr/ADR-14-local-data-oss.md) (macOS 15, 26 et 27 Apple Silicon ; Ubuntu 24.04 et 26.04 x86_64). Rejouer A–O, les compléments d'annexe et les scénarios W/X/Y/Z de consolidation R3–R5 référencés dans [la recette commune](../testing/acceptance-scenarios.md), avec les campagnes F019 et modèles locaux. Vérifier les trois familles d'usage, zéro modèle et supervision bornée, effet au résultat inconnu, OOM/local_only, parcours guidé et parité TUI. Fournir installation, mise à jour, rollback de binaire compatible, export et désinstallation sans perte implicite. La distribution suit Q-16 : Developer ID OVPT, notarisation agrafée, DMG/archives et `.deb`, SBOM et SHA-256 publiés ; une mise à jour signée est appliquée sur commande par l’installeur ou l’updater de l’application, hors protocole daemon.

Gate de V1 : toutes exigences obligatoires ont preuve acceptée ou dérogation utilisateur explicite ; aucune fonction est « reportée V2 » par commodité. Licence Apache-2.0 (Q-06), notices, SBOM, politique de vulnérabilités, politique de marque et provenance des distributions sont établies avant publication open source.

## Règles de pilotage

Chaque lot rend un incrément testable, pas une promesse de robustesse. Les équipes ne réinventent pas les enveloppes communes. Une modification de contrat entraîne mise à jour d’ADR, schéma, tests contractuels et consommateurs dans le même lot coordonné. Aucune durée calendaire n’est inventée sans effectif, niveau d’exigence OS, budget et résultats M0 ; estimer ensuite à partir de tranches verticales mesurées.


## Raccords R6 et R7 au jalon M0

Stabiliser avant implémentation parallèle : autorité par utilisateur indépendante des racines ; CommandEnvelope v2 et lignée de déduplication ; client partagé `unleash-client`, seule voie d’accès au daemon pour CLI, TUI et GUI ; types TypeScript générés depuis les DTO Rust (ts-rs) ; ModelOrigin/probes sans agent ; DiscoveryReceipt multiorigines et TypedSourceRef ; attentes de test/claims typées ; frames for_each v1.2 et admission de diagnostic sous blocked. Les exigences restent réparties dans les features existantes. M0 produit les DTO Rust et schémas complets de payload à partir des contrats documentés ; le catalogue de conception ne prétend pas les générer automatiquement.

Les tests de reprise, de GC/backup et de source/projection documentaire sont des gates spécifiques. Aucune qualification ni mesure ne permet de supprimer une capacité V1. Une application peut importer le paquet pour proposer l’affectation de ces travaux sous mandat, sans interpréter les protocoles d’experts comme des grants.
