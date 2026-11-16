# Regard 2 — Rust Systems Architect

> **Addendum R2 — 16 novembre 2026.** Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). Bibliothèques existantes et HTML/CSS sont autorisés ; Kivy et Tauri sont identifiés ; la GUI et la TUI doivent avoir une parité fonctionnelle complète. Les choix actuels sont dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md). Les autres observations techniques restent à lire avec leurs limites initiales.

**Nature : audit de conception, avant implémentation.** Ce regard est distinct des regards fiabilité et QA, mais porté par le même sous-agent de rédaction ; il ne constitue pas trois revues humaines indépendantes. Référentiel externe : [sources systèmes](../research/systems-sources.md), consultées le 16/11/2026.

## Conclusion et constats confirmés

Un daemon utilisateur Rust avec clients GUI/CLI/TUI et workers éphémères est une proposition réalisable. Les primitives documentées couvrent runtime asynchrone, processus, PTY et stockage local. Cela établit la disponibilité de briques, pas les performances ni la justesse de leur composition. Le [workspace Cargo](https://doc.rust-lang.org/cargo/reference/workspaces.html) permet un verrou commun ; les services métier doivent rester indépendants du framework GUI.

La contrainte « code développé en Rust » est compatible avec l'utilisation explicite de bibliothèques natives. La variante « aucun code transitif non Rust » serait incompatible avec le choix SQLite C et les interfaces système déjà nécessaires. Le dossier doit publier cette distinction, les licences et la nomenclature logicielle. Il ne doit pas présenter `rusqlite` comme une base écrite intégralement en Rust : le [mainteneur](https://github.com/rusqlite/rusqlite) documente sa liaison native.

## Risques et objections

| Objection | Conséquence | Recommandation proposée |
|---|---|---|
| Async assimilé à annulation sûre | Un appel bloquant continue après abandon du handle et l'arrêt peut se bloquer. | Writer sur thread propriétaire ; travail bloquant court borné ; opération incontrôlable en processus supervisé. |
| Daemon propriétaire de tous les maîtres PTY | Son crash perd les handles indispensables à la continuité des terminaux. | Process-host Rust dédié aux PTY/services longs, sans modèle ni orchestration. |
| Groupe de processus assimilé à sandbox | Descendant détaché ou même compte utilisateur peuvent échapper aux attentes. | Promettre supervision, non confinement total ; articuler explicitement le broker et les limites OS. |
| Bibliothèque GUI imposant JS ou glue native développée | Contradiction avec le brief. | Spike de build et inventaire du code livré avant choix GUI ; garder services et DTO Rust indépendants. |
| Un sémaphore mémoire traité comme source de vérité | Après crash, admission divergente et dépassement de la limite. | Réservations SQLite atomiques, miroir mémoire seulement dérivé. |

La documentation [Tokio spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) confirme que l'annulation d'une tâche déjà démarrée n'est pas générale. Le moteur ne peut donc pas promettre qu'un timeout HTTP ou un arrêt de future annule son effet externe. La distinction entre intention d'arrêt, interruption locale et résultat externe doit traverser toutes les interfaces.

## Hypothèses et alternatives

Hypothèse proposée : un seul daemon par UID et répertoire de données, avec verrou OS détenu pendant sa vie, epoch durable croissant et socket privé. Plusieurs workspaces et vingt leaders logiques utilisent ce même ordonnanceur. Autoriser plusieurs daemons indépendants sur la même base apporterait des pannes nouvelles sans répondre au besoin local ; cela n'est pas retenu.

Alternative stockage : un pool SQLx asynchrone est possible, mais multiplier les writers ne supprime pas la sérialisation SQLite. Un propriétaire `rusqlite` avec une file bornée donne un lieu explicite pour les transactions et la saturation. La contrepartie est un goulot potentiel : mesurer les latences et limiter les blobs, les lectures longues et les indexations dans la voie des commandes. La décision reste réversible derrière un port de stockage, sans créer un faux ORM universel.

Alternative PTY : accepter la perte du terminal au crash et relancer son programme après décision. Elle est moins complexe mais ne satisfait pas la continuité utile attendue du pool. Le process-host est retenu comme proposition, avec preuve de rattachement et qualification launchd. Il ne survit pas à une extinction machine et ne restaure jamais une pile RAM.

## Contrats recommandés

Le domaine propose des décisions déterministes à partir d'un état et d'une commande validée ; seul le service de persistance valide une transaction. Le broker d'effets exécute après commit et rapporte une observation. Les workers ne possèdent ni connexion SQLite d'écriture ni clé fournisseur générale. Un appel modèle caché dans un outil reste une invocation soumise au même compteur. Aucun rôle N+2 ou N+1 ne possède un prompt moteur concurrent du harness.

La surface FFI doit être petite, recensée et commentée : credentials IPC, informations de processus, signalisation, PTY, SQLite et éventuel enregistrement du helper macOS. Les méthodes citées dans les specs sont soit documentées dans la version indiquée, soit nommées « contrat Unleash proposé ». Aucune signature issue d'une supposition de SDK n'est admise.

## Vérifications et critères

Le spike doit produire un daemon minimal et un process-host Rust sur macOS et Linux, fermeture GUI sans arrêt daemon, crash daemon avec PTY encore inspectable, et aucun signal envoyé à un PID réutilisé. Le build doit lister les sources Rust et les dépendances natives, avec toolchain et source SQLite réels. Les chiffres de latence sont à établir sur une machine de référence documentée.

Critère d'accord : les frontières de [runtime](../architecture/runtime-lifecycle.md), [workspace](../architecture/rust-workspace.md) et [supervision](../architecture/process-supervision.md) sont démontrées par les tests futurs F001/F003/F011. Critère de refus : une promesse de sandbox totale macOS, une reprise de PTY sans détenteur vivant, une action externe exécutée avant enregistrement de son intention ou un appel modèle échappant à l'admission. Ces refus ciblent des incohérences vérifiables, pas une préférence de framework.
