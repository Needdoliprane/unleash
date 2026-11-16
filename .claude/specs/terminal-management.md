# Gestion du pool de terminaux et services

**Proposition V1, Rust interne, outils projets libres.** Le pool rend découvrables les processus utiles d'une mission : shell interactif, tests, build, serveur local ou tâche longue. Il réutilise une ressource compatible au lieu de lancer des doublons, sans supposer qu'un processus sur un port connu appartient au projet. Les programmes Python/TypeScript/Java/Go/Rust et multilangages sont supportés comme outils externes, sous autorisation et supervision identiques.

## Identité, allocation et découverte

La clé de découverte associe projet, mission, repository/worktree lorsqu’ils existent, cwd ou racine de sources autorisée, purpose, commande normalisée/hash, environnement permis et version/snapshot attendu. Un résultat de discovery expose propriétaire, génération process_run_uuid, PID/PGID observés, backend d'identité, readiness, ports, logs et dernière observation. Trouver une ressource n'autorise pas son contrôle ; l'acquisition exige scope et lease valides.

`AcquireTerminal` peut demander `reuse_compatible` ou `new_required` avec justification. Une incompatibilité de worktree, de version du code ou de permission interdit la réutilisation silencieuse. Deux acquisitions simultanées du même purpose sont sérialisées par lease et contrainte d'unicité ; l'une reçoit le terminal existant ou `ResourceBusy`. Un processus découvert hors registre devient seulement candidat inspectable, jamais automatiquement adopté à partir de son PID.

## États et commandes

| Opération proposée (command/query) | Précondition | Résultat ou erreur |
|---|---|---|
| `DiscoverTerminals` (query) | Scope lecture valide | Résultats avec fraîcheur et compatibilité explicitée. |
| `AcquireTerminal` | Révision/politique/budget disponibles | Lease, terminal_id, process_id, `reused` booléen et état `preparing` ou `ready`. |
| `SendTerminalInput` | Propriétaire/fence valides, entrée autorisée | action_id, input_seq et receipt durable ; unknown possible après interruption. |
| `ResizeTerminal` | Contrôle autorisé | Nouvelle taille observée ou erreur ResourceBusy/TERMINAL_UNAVAILABLE. |
| `ReadTerminalLog` (query) | ACL et curseur valides | Segments et watermark durable, gaps/purge signalés. |
| `ReleaseTerminal` | Propriétaire actuel | Retour au pool ou fermeture selon contrat de ressource. |
| `StopProcess` | Identité vérifiée et permission | Événement `ProcessStopRequested`, puis `ProcessExited` ou observation d'arrêt partiel/inconnu ; jamais succès sur le seul envoi du signal. |
| `ReconcileProcess` | Récupération ou état suspect | Observation attribuable sans duplication de spawn. |

Les états terminal sont `preparing`, `ready`, `busy`, `detached`, `stopping`, `exited`, `suspect`, `unavailable`. `detached` signifie absence de client UI, pas perte de propriétaire ni liberté d'écrire. Une commande interactive peut avoir de nombreux effets ; le broker doit la traiter selon la politique du shell, et non prétendre connaître toutes ses écritures à partir d'une chaîne de texte.

## Portée des ressources

Les PTY et services longs sont détenus par un [process-host](../architecture/process-supervision.md) Rust. Celui-ci maintient le maître, draine la sortie et applique des commandes à capacité restreinte. Une invocation peut se suspendre durablement pendant un test long ; le test consomme le budget processus, et aucun raisonnement supplémentaire ne se déroule sans réadmission d'invocation. Les shells ne peuvent pas lancer secrètement de nouveaux agents via une API modèle disposant de clés globales.

Les ressources auxiliaires sont bornées par configuration utilisateur : nombre de hosts/enfants, PTY, octets de log, ports et capacité OS disponible. Les descendances sont comptabilisées selon observabilité ; les limites strictes de CPU/mémoire dépendent des mécanismes de l'OS et ne sont pas promises comme sandbox portable. Un dépassement déclenche une politique explicite : refus de lancement, backpressure, rotation, interruption ou demande d'attention.

Le pool n'est pas un stock obligatoire de shells préouverts. L'allocation à la demande évite des processus inutiles ; une ressource chaude reste disponible lorsqu'elle réduit un coût mesuré. Le cache de discovery porte son timestamp et doit être revalidé au contrôle. La recherche avant création produit une trace des candidats et de la décision reuse/new avec raison.

## Logs et readiness

Un lancement réussi signifie que le processus est observé, pas que le service est prêt. Chaque type de service peut fournir un critère de readiness versionné : écoute d'un socket attribué, réponse locale bornée, message structuré ou sortie attendue. Ce critère ne devient pas une validation fonctionnelle de tout le projet. Un timeout de readiness conserve le diagnostic et respecte la politique d'arrêt.

Les logs portent terminal_id, process_run_uuid, sequence, timestamp observé, hash de segment et watermark durable. Les octets diffusés avant synchronisation sont signalés comme volatils. Une rotation conserve un manifeste des segments ; un gap est visible. Les séquences de contrôle du terminal sont rendues dans un composant adapté, sans permettre à une sortie de terminal de devenir une commande du harness. L'indexation ou le modèle traite cette sortie comme donnée non fiable.

## Crash, signaux et limites

Un arrêt valide recontrôle l'identité au moment de la signalisation. PID seul, port seul et nom d'exécutable seul sont insuffisants. Le host peut envoyer un signal au groupe qualifié, attendre, puis escalader selon le délai autorisé. Les descendants détachés et les processus non observables restent signalés ; aucune garantie universelle d'absence d'orphelin n'est donnée sans mécanisme de confinement qualifié.

Après crash daemon, le host vivant peut être rattaché après nouveau handshake/fence et rapprochement des logs. Après perte du host, le PTY peut être définitivement indisponible ; la reprise ne crée pas un nouveau shell en prétendant poursuivre l'ancien. Après reboot, les anciennes identités OS sont obsolètes et les anciens processus sont clos en `lost` ([supervision](../architecture/process-supervision.md)) : réputés disparus, effets externes non conclus. Les entrées dont l'écriture au PTY est ambiguë ne sont jamais répétées automatiquement, même si le command_id a un receipt local absent.

Les critères F011 portent sur réutilisation réelle, absence de spawn concurrent en double, attribution de ports, résultat des signaux, conservation des logs, redaction et refus hors scope. La compatibilité macOS/Linux et la survie des hosts au restart du service restent des preuves à produire ; la fermeture GUI seule est un scénario séparé et plus simple.
