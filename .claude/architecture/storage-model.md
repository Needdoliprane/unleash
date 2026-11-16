# Modèle de stockage et identités canoniques

**Statut : schéma métier proposé.** La base stocke les décisions et activités observées ; Git reste l'autorité du code ; les index sont dérivés. Ni le journal d'activité ni un résumé modèle ne remplacent une preuve d'état réel. Les relations ci-dessous évitent les mots « agent », « session » ou « run » employés comme synonymes.

## Identités

Tous les identifiants sont opaques, immuables et distincts par type dans le domaine Rust. Leur encodage proposé est un UUID canonique en texte ; leur ordre lexical ne définit pas l'ordre métier. Les timestamps UTC servent à l'audit ; `event_seq`, révisions et fences servent à l'ordre de causalité contrôlé.

| Identité | Objet et durée de vie | Relations essentielles |
|---|---|---|
| `goal_id` | Objectif utilisateur, pouvant traverser plusieurs missions | Possède des missions et une définition de succès. |
| `mission_id` | Engagement avec owner utilisateur ou agent et sources explicites | Liée à goal/projet et runs ; repository/worktree facultatifs, normalement dédiés pour une mission de code. |
| `run_id` | Exécution d'une version effective de workflow | Liée à mission, workflow/version et instances d'étapes. |
| `iteration_id` / `iteration_vector` | Identité d'itération d'une boucle interne | Regroupe les instances de nœuds de cette itération ; ne constitue pas une frontière de publication/adoption. |
| `step_id` | Instance d'étape exécutée, y compris itération | `definition_node_id` désigne le nœud de définition, distinct. |
| `task_id` | Unité de travail avec contrat et scope | Rattachée à mission/run/step selon le moteur ; peut nécessiter plusieurs invocations. |
| `agent_id` | Identité logique N/N+1/N+2 | Rôle organisationnel distinct du rôle versionné ; conserve sa session même dormant. |
| `session_id` | Continuité durable de contexte et de décisions | Appartient à un agent ; fragments et manifestes immuables. |
| `invocation_id` | Tour/segment actif matérialisé pour cet agent | Session, événement déclencheur, task, bundle effectif ; conserve son identité en reprise. |
| `attempt_id` | Tentative éphémère d'exécuter une invocation | Processus worker, authority_generation, epoch, dates, résultat ; au plus une tentative locale active admise. |
| `action_id` | Intention d’effet observé et réconciliable | Origine typée invocation, replay ou control_plane ; payload hash, clé d’idempotence, autorisation, dispatchs et observations. |
| `model_call_id` | Appel API modèle distinct | Lié à une action, un manifeste d'appel immuable et exactement un propriétaire : invocation ou provider_probe ; un retry de transport reste un dispatch de la même action ; fallback autorisé = nouvel appel/action. |
| `process_id` | Enregistrement durable d'un processus contrôlé | Action, process-host, identité OS, lifecycle et logs. |
| `process_run_uuid` | Génération de lancement OS | Nouveau à chaque spawn ; distinct du `run_id` de workflow et du PID. |
| `evidence_id` | Observation immuable avec provenance | Action/run/snapshot, artefact hash, producteur, résultat et éventuelle supersession. |

`role_version_id`, `skill_version_id`, `workflow_version_id`, `contract_version_id`, `model_profile_version_id` et `scope_revision` sont des références de versions. Un rôle n'est pas un skill ; une invocation stocke son bundle de versions/autorités, configuration modèle demandée, politique figée de modèles autorisés et manifeste initial. Chaque appel possède un ModelCallManifest distinct fixant modèle/provider/capacités/contexte réellement utilisés. Une requête API possède `provider_request_id` si le fournisseur le retourne et un `action_id` local même sans identifiant distant. Les résultats/usage sont des receipts ou manifestes de résultat immuables liés au manifeste de requête, jamais une réécriture silencieuse de celui-ci.

Selon la décision Q-03 ([ADR-04](adr/ADR-04-workflow-semantics.md), R7), un **cycle métier correspond à un run**, terminé et quiescent avant adoption d'une nouvelle définition de workflow ; l'adoption passe par un nouveau Run à référence exacte, avec `predecessor_run_id` pour un successeur, et aucune migration en place n'existe en V1. Une boucle interne reste une itération. Les bindings déjà engagés restent stables, y compris en reprise. Un slot déclaré encore vierge peut recevoir une liaison explicite de rôle/skill nouvellement publié avant sa première activation, dans le plafond d'autorité déclaré ; cette liaison ne modifie pas la définition du workflow ni un bundle engagé.

`authority_generation` est un UUID de lignée, renouvelé après restauration/import de snapshot et conservé au redémarrage ordinaire. Il n'est jamais ordonné : deux UUID différents sont deux autorités incompatibles. `daemon_epoch` et resource_fence sont monotones seulement dans la même lignée. Chaque capacité, tentative, lease, host et CommandEnvelope inclut cette lignée. Le client l'épingle avec son intention durable, pas à chaque connexion ; une ancienne commande ne retrouve donc pas son autorité lorsque la restauration ramène expected_revision à une valeur déjà utilisée. Le writer refuse la lignée avant la recherche idempotente. Les receipts historiques gardent leur provenance.

## Sources de vérité et rétention

| Information | Autorité | Réconciliation |
|---|---|---|
| Fichiers, commits, refs, worktrees | Git et filesystem observés | Diff/snapshot exact, provenance, événement moteur après modification. |
| Contrats, workflows, rôles, skills publiés | Registres versionnés et hashes en DB/artefacts | Versions immuables ; publication nouvelle distincte d'adoption. |
| État de mission et activité | Transactions SQLite | Révision, journal, commande causale et receipt. |
| Contexte réellement envoyé | ModelCallManifest et context manifest persistés avant chaque envoi | Provider/modèle/capacités effectifs, sélection, exclusions et hash ; usage dans receipt lié. |
| Index symboles/recherche | Vue dérivée versionnée | Rebuild, génération, statut de fraîcheur, ACL et provenance. |
| Processus et ports | Observation OS + handshake propriétaire | L'état DB est une dernière observation, pas une preuve que le processus existe encore. |
| Résultat externe | Receipt ou preuve externe attribuable | Unknown conservé en absence de preuve suffisante. |

Les données sont classées par [politique de rétention](#politique-de-rétention-par-défaut-q-12). Effacer les logs ne supprime pas une décision, mais peut rendre sa preuve indisponible : l'état doit le dire. Une suppression d'artefact référencé exige un tombstone, une décision applicable et l'impact explicite sur les futures reprises. Les références métier sont inscrites transactionnellement dans object_references ; seules leurs entrées live autorisent la lecture normale. Une politique de confidentialité peut supprimer du contenu sensible tout en conservant une métadonnée minimale légitime ; elle ne doit pas laisser croire que le contexte reste intégralement inspectable. Les limites d'effacement physique du stockage et des copies externes restent celles de [security-boundaries.md](security-boundaries.md).

Pour les commandes, la réponse intégrale est conservée au moins pendant la fenêtre annoncée au commit : 7 jours par défaut, réglable de 1 à 90 jours, fixée au commit (ADR-01, Q-12). Après cette fenêtre, une purge explicite peut la remplacer par un tombstone de déduplication, sans effacer la clé ni sa causalité pendant toute la lignée active. Un retry sur tombstone est refusé avec IDEMPOTENCY_WINDOW_EXPIRED ; une autre lignée est refusée avec STALE_AUTHORITY_GENERATION. Réduire une durée de rétention ne raccourcit pas les fenêtres déjà promises. La rétention ordinaire ne renouvelle pas la lignée et ne remet jamais un ancien command_id dans l'état « jamais vu » ; voir [le schéma et les erreurs](sqlite-design.md#tables-dautorité-et-commandes).

### Politique de rétention par défaut (Q-12)

**Décision Q-12 (R7).** Cette section est l'autorité de la politique `unleash.retention.default.v1`. La politique est affichée et modifiable dans Settings GUI et TUI (SetSettings, PAR-SETTINGS-05). Un balayage quotidien du daemon l'applique comme action d'origine control_plane, avec receipt, événements et protocole de collecte du writer ; le balayage est suspendu si l'horloge est incertaine.

Ne sont jamais purgés automatiquement :

- un objet référencé par un run, une mission ou une invocation non terminal ;
- une action `outcome_unknown`, un run `blocked` ou un incident non résolu ;
- une preuve exigée par un contrat actif ;
- une version publiée non révoquée (workflow, skill, rôle, automatisation) et ses preuves d'évaluation ;
- un objet sous pin ou marqué « conserver » par politique.

Le délai court à partir de l'état terminal du propriétaire.

| Classe | Défaut |
| --- | --- |
| Journal métier : événements, décisions, interventions humaines, ledger d'actions/dispatchs/coûts, métadonnées de receipts, reçus discovery, tombstones | Sans échéance automatique ; purge explicite PurgeHistory seulement ; clés de commande conservées pendant toute la lignée |
| Réponse complète de commande | 7 jours après le commit, puis tombstone ; échéance fixée au commit |
| Sessions durables, manifestes de contexte et objets fournisseur opaques nécessaires à la session | Tant que l'agent existe |
| Payloads exacts envoyés et reçus des appels modèle | 90 jours après l'état terminal de la mission |
| Sorties d'outils, logs de terminaux et de processus | 30 jours après la fin du processus et l'état terminal de la mission |
| Artefacts navigateur : captures, DOM, traces | 30 jours après le run terminal |
| Captures dirty de worktree | 30 jours après l'état terminal de la mission |
| Données importées d'automatisation | 90 jours après le dernier run terminal qui les référence |
| Copies de documents connectés | Tant que l'affectation existe ; 30 jours après retrait, déconnexion ou révocation observée, sans réinjection entre-temps |
| Rapports scellés | Sans échéance ; les preuves citées suivent leur classe, puis apparaissent EvidenceUnavailable |
| Artefacts d'export préparés | 7 jours après disponibilité |
| Logs de diagnostic | Rotation quotidienne, 14 jours, plafond 512 MiB |
| Cache | Reconstructible, LRU plafonné à 2 GiB |
| Index dérivés | Suivent leurs sources ; reconstruits |
| Worktrees gérés | Jamais retirés s'ils sont dirty, non publiés, candidats de conflit ou utilisés par un processus ; un worktree propre, sans processus, dont le commit est atteignable depuis une ref publiée est retiré 7 jours après l'état terminal de la mission |

Une purge retire la référence de la classe expirée. Les octets ne sont supprimés qu'en l'absence de référence live et de pin, par le protocole de collecte ci-dessous ; les références tombstone exposent alors `purged`/EvidenceUnavailable. Chaque purge se termine par `PRAGMA wal_checkpoint(TRUNCATE)` sous `secure_delete=FAST` ([SQLite](sqlite-design.md#profil-de-connexion-et-version)) ; `VACUUM` reste une maintenance explicite. Réduire une durée affiche d'abord l'impact (nombre, taille) ; l'effet commence au premier balayage au moins 24 h après l'enregistrement, et les fenêtres de commande déjà promises ne sont jamais raccourcies.

Archiver ne supprime rien et ne change aucun état backend : GUI et TUI masquent par défaut, de façon identique et réversible, les missions et runs terminaux depuis plus de 30 jours, les agents `retired` et les agents dormants sans invocation depuis 90 jours. L'archive hors ligne d'un projet est un ExportBundle.

PurgeHistory affiche ses limites. Les sauvegardes gérées qui contiennent les octets purgés sont listées et peuvent être supprimées explicitement ; sinon ces octets y subsistent au plus 35 jours, le temps de la rotation. Unleash n'efface ni les sauvegardes manuelles ou externes, ni les snapshots APFS/Time Machine, ni les exports déjà remis, ni les copies chez les fournisseurs de modèles ou de documents ([frontières de sécurité](security-boundaries.md)). Les valeurs par défaut peuvent être ajustées sur mesure M7, avec trace documentée et sans changer la structure de la politique. Les sections `[retention]` et `[backup]` de l'[exemple de configuration](../examples/unleash-config.toml) projettent cette politique avec les mêmes valeurs par défaut ; en cas d'écart, la présente section prévaut.

## Écriture d'objets volumineux

Les payloads volumineux, logs et manifestes sont adressés par hash hors DB. Une incarnation physique porte une object_generation et un chemin propres ; recréer des octets identiques après collecte conserve le hash mais utilise une autre incarnation. Le writer possède états, références et pins. Une période de sûreté peut retarder la collecte ; elle ne prouve jamais l'absence de producteur et ne constitue pas l'exclusion.

1. Le producteur soumet le hash, la taille et son propriétaire au writer. Une transaction obtient un pin preparation sur une incarnation live ou crée l'incarnation prepared et son pin. Un objet deleting refuse tout nouveau pin/rattachement avec OBJECT_DELETING. Un unique propriétaire réalise la préparation physique ; les producteurs concurrents du même hash attendent son résultat sous pins.
2. Pour une nouvelle incarnation, écrire un temporaire privé, synchroniser selon l'adaptateur filesystem qualifié, renommer dans le même filesystem puis synchroniser le répertoire si requis. Les I/O s'effectuent hors transaction SQL. Le chemin d'incarnation empêche une ancienne collecte de viser une future réapparition du même hash.
3. Après vérification de hash/durabilité, le writer revérifie pin, propriétaire et génération, passe prepared à live et publie ensemble la référence métier et object_references.live. Il rend ensuite le pin dans cette même transaction. Un objet déjà live suit la même exclusion entre pin, publication et collecte. Si le producteur disparaît, son pin passe reconciling ; aucun timeout ne le supprime tant qu'il peut encore publier ou écrire.
4. Le collecteur peut présélectionner des candidats, mais doit réserver la suppression sous le writer : zéro référence live, zéro pin held/reconciling de l'incarnation et aucune backup_barrier active. Cette transaction écrit deleting et l'intention durable object_deletions.prepared avec hash, génération, chemin et motif. Une purge explicitement autorisée rend d'abord les références concernées tombstone et expose CONTEXT_INCOMPLETE/EvidenceUnavailable ; elle ne contourne pas un pin vivant.
5. Seulement après ce commit, supprimer le chemin d'incarnation et synchroniser les métadonnées selon l'adaptateur qualifié. Finaliser object_records.tombstone et object_deletions.completed dans une transaction. Aucun rattachement ni nouvelle incarnation n'est accepté avant cette clôture. Un échec d'I/O laisse deleting et le diagnostic ; un crash entre intention, unlink, synchronisation et commit reprend la même opération sans produire de référence cassée.

La récupération vérifie le chemin et la génération enregistrés, neutralise l'ancien producteur/collecteur avant de réaffecter son mandat et conserve les pins suspects. Un fichier absent n'autorise la finalisation qu'après vérification de l'intention durable correspondante et de la synchronisation requise. Un fichier étranger ou un hash divergent conduit à quarantined/diagnostic, sans unlink opportuniste. Les états prepared, live, deleting, tombstone et quarantined ainsi que les pins restent inspectables.

Une preuve est vérifiée en recalculant son hash et en contrôlant sa provenance. Le hash donne une identité de contenu, pas l'authenticité d'un programme ni la vérité d'une assertion. `observation`, `claim` et `judgment` restent des catégories séparées : une phrase « tests réussis » ne crée pas de receipt d'exécution.

## Transaction métier et événements

La transaction proposée contient validation de révision, modification d'état, append d'événements, entrées outbox, receipt de commande et réservations nécessaires. Aucun appel fournisseur, commande Git, accès réseau de connecteur ou spawn de processus n'est effectué pendant cette transaction. L'exécution d'effet intervient ensuite et produit sa propre observation transactionnelle. Le [schéma SQLite](sqlite-design.md) décrit la correspondance.

L'outbox transporte au moins une fois ; une inbox déduplique chaque consommateur. Cela évite des doublons d'application internes sous son contrat de rétention : une ligne inbox n'est purgeable que 7 jours au moins après l'acquittement de l'entrée outbox correspondante ([ADR-02](adr/ADR-02-event-architecture.md)). Une action externe garde son propre protocole d'idempotence/réconciliation. L'expiration d'un lease ou une nouvelle tentative de worker ne change ni l'identité de l'action engagée ni son payload.

## Sauvegarde et restauration

Le backup associe snapshot SQLite, schema_version, build/source SQLite, authority_generation, stream_id, high-water mark, receipts/tombstones, ledger d'actions/dispatchs/budgets, hashes des objets inclus et état des références Git. Les worktrees peuvent être sauvegardés séparément selon la politique projet ; le manifeste indique précisément ce qui est couvert. Une base seule n'est pas présentée comme sauvegarde complète du code non committé.

Politique de sauvegarde par défaut (Q-12) :

- cadence : RequestBackup automatique toutes les 24 h de fonctionnement du daemon, et obligatoire avant toute migration et toute RestoreBackup ; une sauvegarde automatique est différée si l'espace restant estimé descend sous 5 GiB ;
- destination : `<données>/backups/` par défaut ; une destination externe (disque externe, dossier synchronisé) est admise pour des jeux complets, jamais pour la base active ;
- rotation : 7 quotidiennes et 4 hebdomadaires ; les sauvegardes manuelles sont conservées jusqu'à suppression explicite ;
- couverture : snapshot SQLite cohérent, objets live référencés, ledger, receipts/tombstones et manifeste ; sont exclus les secrets du coffre OS, le cache, les logs de diagnostic et les worktrees (option par projet pour les captures dirty) ;
- vérification : hashes, puis `integrity_check` et `foreign_key_check` de la copie avant `complete` ([ADR-01](adr/ADR-01-sqlite-persistence.md)) ;
- protection : permissions 0700/0600 ; pas de chiffrement applicatif en V1 ; le chiffrement disque du système est recommandé et affiché.

Avant toute copie DB, le writer crée backup_jobs et un pin global durable backup_barrier ; il attend la clôture des suppressions déjà engagées et empêche tout nouvel unlink. Les producteurs peuvent continuer à publier sous leurs pins. Une fois le snapshot SQLite cohérent terminé, lire son watermark et son graphe exact de références depuis la destination ; poser transactionnellement les pins backup de toutes les incarnations nécessaires, y compris manifests d'effets et preuves, avant de relâcher la barrière globale. Un objet nécessaire manquant ou incohérent interdit complete. Copier ensuite les objets sous ces pins, vérifier hashes/couverture, synchroniser destination et manifeste final selon l'adaptateur qualifié, puis seulement publier complete et libérer les pins. Aucun manifeste final ne précède ses octets durables. Une sauvegarde interrompue conserve barrière/pins jusqu'à reprise ou abandon explicite réconcilié ; une expiration seule ne les libère pas.

Restaurer requiert une barrière offline : admission/effets gelés, inventaire des exécuteurs de l'ancienne lignée et arrêt/neutralisation vérifiés avant activation. RestoreBackup est accepté depuis `ready`, `degraded` ou `read_only_recovery` (cas principal après corruption) et laisse le daemon en `read_only_recovery`. Un inventaire local incomplet ou un processus suspect impose RestorationBlocked et quarantaine ; ce blocage est persisté dans `restore_requests` de la lignée source, sans lignée cible ni capsule. Si la base source est illisible ou non inscriptible, receipt, restore_id et RestorationBlocked sont écrits dans le manifeste privé de maintenance hors base, lu par InspectRecovery puis importé par la capsule dans `restore_imports` de la base restaurée ([failure-recovery](failure-recovery.md#disque-plein-corruption-et-restauration)). Avant substitution, conserver une capsule durable, copiée et épinglée dans la destination : lignée source, snapshot choisi, actions/dispatchs/receipts, consommations inbox et coûts postérieurs, y compris effets déjà terminés, références de preuves et couverture connue. Les tombstones/révocations postérieurs utiles à la sécurité des reprises restent applicables. Ce transfert est un artefact de récupération, pas un deuxième compteur d'autorité.

Le writer importe la capsule et les blocages avant ouverture de l'admission ; il réconcilie aussi les actions présentes comme prepared dans le snapshot mais potentiellement exécutées depuis. Les coûts observés et inconnus ne sont pas remis à zéro par rollback. Si la base actuelle est perdue/corrompue et que le delta n'est pas reconstituable, les scopes potentiellement affectés restent bloqués ; si leur étendue est inconnue, le gel des effets est global jusqu'à disposition explicite. Tout scope dont la capsule contient une action, un dispatch, un receipt ou une consommation d'événement postérieurs au watermark reste gelé jusqu'à disposition explicite, même lorsque ces actions sont classées ; avant cette disposition, ni le dispatcher outbox (entrées pending/leased héritées du snapshot) ni le scheduler ne relancent une intention de ce scope. Un boot différent exclut seulement les anciens processus locaux, jamais ces effets distants.

La base restaurée reçoit une nouvelle authority_generation, un nouveau stream_id et de nouveaux secrets ; les anciennes commandes sont refusées avant recherche du receipt, même si leur expected_revision redevient valide. Epoch/fence ne se comparent pas entre lignées. Aucun vieux host n'est adopté à chaud. Un effet postérieur n'est jamais redispatché parce que son marqueur manque au snapshot ; preuve ou décision autorisée explicite est nécessaire. Voir [failure-recovery.md](failure-recovery.md) et [crash-recovery-tests.md](../testing/crash-recovery-tests.md).

## Origines d’action sans agent

Le [contrat SQL](sqlite-design.md) impose une origine typée pour chaque action : invocation, replay ou control_plane, avec ses FKs et contrôles d’appartenance. Un replay a run/step/worker et un manifest durable même s’il n’a aucune session agente. Une opération de contrôle possède un principal/service et un receipt/timer/événement cause. L’absence d’invocation n’est jamais l’absence d’attribution. Le broker et le Router dérivent la lignée du mandat authentifié ; un worker ne la retire pas pour échapper à forbidden ou au budget.
