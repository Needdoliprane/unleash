# Conception technique F011

## Composants et registre

Le broker normalise les demandes ; `unleash-platform` observe l'OS ; `unleash-process-host` conserve PTY et spool ; le writer maintient processes, terminal_sessions, resource_leases et preuves. Les tables sont définies dans [sqlite-design.md](../../architecture/sqlite-design.md). La crate [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) est retenue pour le PTY et `rustix` pour les appels processus/signaux ([ADR-05](../../architecture/adr/ADR-05-durable-sessions.md), accepté R7), versions verrouillées dans Cargo.lock en M0 ; leur emploi n'élimine pas le protocole de survie et de rattachement.

Une ressource porte projet, mission, agent propriétaire éventuel, purpose_key, worktree éventuel, cwd canonique, hash de commande/environnement, generation process_run_uuid, identité OS et fence. Les arguments sont transmis en tableau ; aucun shell n'est injecté pour des commandes ordinaires. Un shell interactif reste une action explicitement permise, dont les effets ne peuvent pas être entièrement inférés du texte.

Les ProcessSpec catalogués comprennent la classe « installation d'analyseur » (Q-13, [connaissance partagée](../../specs/shared-knowledge.md)) : argv fixe issu de la commande officielle affichée (par exemple `rustup component add rust-analyzer`), effet réseau déclaré, lancement par `AcquireTerminal` seulement après confirmation ou mandat explicite, logs et résultat observés, puis nouvelle détection par F009. Jamais de `sudo`, de modification des manifestes du projet, d'installation automatique ni de mise à jour silencieuse.

## Discovery puis acquisition

Rechercher les entrées compatibles, revalider fraîcheur et identité, produire une trace discovery avec candidats et choix. Deux requêtes concurrentes sont arbitrées par lease/contrainte unique, puis l'une réutilise ou reçoit ResourceBusy. Un serveur d'un autre worktree ou ancien snapshot n'est pas adopté parce que son port répond.

Avant lancement, la transaction enregistre action, génération et budgets/lease. Le host écrit son manifeste privé, ouvre PTY et logs, lance l'enfant, recueille l'identité et répond à un challenge. L'observation de launch est persistée ensuite. Si la coupure arrive entre spawn et receipt, le daemon recherche cette génération/host ; il ne lance pas un doublon. Ready est distinct de live et repose sur un contrat de readiness borné.

## Contrôle et entrées

Le canal host est local privé et authentifié ; chaque commande porte authority_generation, epoch, fence, process_run_uuid, action et command_id. Le host exige lignée identique puis compare epoch/fence dans celle-ci. Une autre lignée est refusée, jamais ordonnée par UUID. Une restauration impose arrêt vérifié des anciens hosts et nouveaux hosts/secrets ; aucun hot adopt inter-lignée. Le host ne stocke pas de clé API globale ni de permission de créer une nouvelle invocation. La rotation de capacité après recovery ordinaire neutralise les anciens canaux avant toute nouvelle entrée.

SendTerminalInput est une action d'effet : séquence unique, payload hash et receipt. Entre écriture au maître et receipt durable, un crash laisse une inconnue ; une déduplication locale n'invente pas l'annulation des octets déjà consommés. Une commande peut avoir produit ses effets même si son echo n'est pas présent dans les logs.

## Arrêt, ports et descendant

La signalisation exige une cible non réutilisable : handle stable qualifié, ou host détenteur vivant conservant un ancrage qualifié jusqu’au signal. L'ancrage V1 d'[ADR-05](../../architecture/adr/ADR-05-durable-sessions.md) est le host parent direct non réapé : il ne signale que son enfant direct non réapé et le groupe dont cet enfant est leader (`setsid`) tant que l'enfant n'est pas réapé, avec SIGCHLD non ignoré, sans `SA_NOCLDWAIT`, signaux et réapage sérialisés par le seul handle propriétaire ; sous Linux, pidfd s'ajoute lorsqu'il est qualifié. Le daemon ne signale jamais un PID observé ; après perte du host, l'identité est `IdentityUnverified` et la réconciliation se fait par observation seulement. Une observation répétée du PID/PGID et du démarrage ne suffit jamais. Le protocole de groupe doit prouver séparément sa non-réutilisation ; sans cela, contrôle individuel sûr ou IdentityUnverified. Le crash du host retire cette présomption de possession. L’exit/disparition doit être observé. Les descendants détachés ou inobservables sont rapportés, sans promesse universelle de nettoyage.

Le port est réservé logiquement puis réellement lorsqu'un descripteur peut être transmis. Sinon la course de bind est gérée comme échec observable et sélection/retry borné, sans arrêter le propriétaire inconnu. La readiness n'est pas une preuve de correction fonctionnelle du projet.

## Logs et fin de vie

Le host draine indépendamment des clients ; segments immuables, séquence, offset, hash et watermark durable permettent la reprise. Les sorties visibles non flushées sont signalées volatiles. Les quotas provoquent rotation/gap/arrêt selon politique déclarée ; ils ne laissent pas gonfler la mémoire sans limite. La sortie terminal est donnée non fiable, y compris ses séquences de contrôle et instructions textuelles.

Detach UI conserve la ressource. Release applique le contrat du pool. RetireWorktree (F010) vérifie absence d'usage ou arrêt/détachement sûr. Crash host peut perdre le PTY ; reboot invalide les identités anciennes et clôt les anciens processus en `lost` (boot différent). Les services externes ou aux effets ambigus restent à réconcilier avant relance. La distinction est visible dans GUI/CLI/TUI et non réduite à un voyant vert « terminal repris ».
