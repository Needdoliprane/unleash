# Tests de concurrence et d'ordonnancement

**Plan futur ; aucune exécution produit revendiquée.** Ces suites couvrent principalement T-F001-001/005/009/010, T-F002-001/003/004/008/009, T-F003-003/004/005/006/008/010 et T-F011-001/007. Les preuves restent celles des features ; les scénarios ci-dessous organisent les campagnes communes.

## Invariants sur chaque préfixe

Le nombre total de réservations d’exécution held/reconciling, agent_invocation, replay_worker ou provider_probe, respecte toujours occupation≤effective_limit≤50. Une baisse sous occupation écrit requested_limit pending et conserve l'ancienne effective_limit avec admissions gelées. La nouvelle effective_limit est activée atomiquement seulement après drainage à son niveau. Il n'existe aucune exception où l'occupation dépasserait la limite effective. Tous les niveaux N/N+1/N+2 comptent ; aucun processus auxiliaire ne devient un agent modèle sans admission.

Au plus une tentative autorisée par invocation et un propriétaire par ressource exclusive ; fences et révisions ne décroissent jamais. Un coût inconnu conserve sa réservation. Toute commande reconnue a son receipt et ses événements atomiques. Un événement livré plusieurs fois n'applique qu'une mutation interne contractuelle grâce à l'inbox. Les payloads d'actions engagées sont immuables.

## Scénarios dirigés

| Campagne | Mise en concurrence | Oracle |
|---|---|---|
| Admission globale | 20 leaders logiques, N+2, nombreux N, projets distincts, C=1/5/50 | Maximum par préfixe, aucun rôle exempté, C=51 refusé. |
| Budget atomique | Deux actions prennent les dernières unités global/projet/fournisseur | Toutes les réservations de l'action gagnante, aucune de la perdante ; ledger cohérent. |
| Révision concurrente | Deux commandes différentes sur expected_revision identique | Une seule gagne ; seconde RevisionConflict, pas de mutation perdue. |
| Retry identique | Plusieurs connexions envoient même actor/command_id | Un receipt et une décision, réponses cohérentes malgré request_id distincts. |
| Retry divergent | Même clé, payload changé ou ordre JSON normalisé | Conflit seulement si sens canonique diffère ; aucune seconde action. |
| Outbox/inbox | Duplicates, reorder permis et crash entre application/ack | Déduplication transactionnelle et causalité vérifiable. |
| Ancien résultat | Worker A retardé, lease réaffectée à B, A répond | StaleFence ; aucune clôture attribuée à la mauvaise génération. |
| Compaction | Nouveau message arrive entre lecture et CAS du manifeste | Message conservé, révision explicite, nouveau manifeste rebasé/recalculé. |
| Publication | v2 publiée pendant deux branches v1, l'une suspendue, et slot déclaré vierge | Trois assertions : définition/bindings engagés stables ; liaison explicite rôle/skill v2 du slot vierge avant activation et dans ceiling ; reprise/itération conserve binding. Nouvelle définition workflow au run suivant quiescent. |
| Pool terminal | Deux demandes purpose identique et course de bind | Un seul propriétaire compatible ; conflit réel visible sans kill étranger. |
| Lecteur WAL et export | Lecture retenue par barrière, writer alimenté et export dépassant son quota | Bornes de lecture/file/WAL/export, backpressure observée, aucun faux receipt ; sous-campagne T-F002-002/007 détaillée dans [F002](../features/F002-persistence-events/tests.md). |

Les outils de projet ou les retries qui voudraient utiliser un modèle doivent passer par le même router/admission. Le test exige la restriction des capacités et secrets disponibles ; il ne revendique pas l'impossibilité universelle d'un programme hostile du même UID d'atteindre un réseau hors confinement qualifié.

## Cinq travailleurs et manager urgent

Le cas principal démarre cinq N dans des fragments contrôlés, puis rend un manager N+1 ou N+2 urgent. En réserve souple (défaut de Q-15, une unité réservée), le manager est queued ; une seule demande de checkpoint est adressée à un N préemptible choisi dans l'ordre déterministe : déjà à une frontière sûre, projet le plus au-delà de sa part pondérée, admission la plus récente, plus petit reservation_id. Sans checkpoint au bout du délai d'escalade (60 s par défaut), la demande est aussi adressée au candidat suivant ; une unité rendue en surplus retourne à la file normale. La fixture garde ce N actif jusqu'à une barrière, ce qui permet de vérifier qu'aucune sixième admission n'apparaît. Après checkpoint durable et neutralisation de continuation, le slot est libéré et le manager démarre.

Variantes obligatoires : cinq N attendant owner ; N+1 attendant enfants ; requête modèle avec stream ouvert ; outil long avec host indépendant ; deadline locale expirée ; provider qui continue après socket fermée ; aucune invocation préemptible. Waiting_owner et waiting_children doivent libérer après checkpoint pour éviter le deadlock. Le stream local actif conserve son slot. L'outil long conserve seulement le budget processus après suspension sûre.

La réserve dure d'une place avec C=5 doit limiter les N à quatre et garder une disponibilité logique pour le contrôle. Un quota fournisseur épuisé peut néanmoins bloquer le manager. Avec C=1, le mode dur est refusé (`InvalidArgument`, reason_code `CONTROL_RESERVE_INCOHERENT`) sans ajustement silencieux, et le mode souple ne doit pas affamer définitivement le travail. Une baisse de 50 à 5 avec 50 occupés doit garder effective_limit=50, afficher requested_limit=5 pending et geler admissions ; activation atomique de 5 seulement à occupation≤5, sans états artificiellement terminés.

## Équité et latence

Les poids projets, vieillissement, quota de rafale contrôle et deadlines sont fixés dans la configuration du test. Mesurer attente max/médiane par classe et projet, progression minimale et nombre de checkpoints. Une arrivée continue de contrôle doit laisser progresser les workers admissibles : en mode souple, après `control_burst_limit` (2 par défaut) admissions control consécutives alors qu'un travail admissible attend, l'admission suivante va au travail admissible le plus ancien. Un projet lourd ne doit pas épuiser le budget d'un autre.

La borne de démarrage proposée est calculée pour les fixtures contrôlées : temps restant du fragment sûr, délai de checkpoint/commit, admissions contrôle déjà prioritaires et quota fournisseur. Le rapport sépare violation de politique et absence d'hypothèse de plateforme. Il ne transforme pas cette borne en promesse universelle sous blocage noyau ou panne API.

## Techniques et reproductibilité

Des tests de propriétés génèrent permutations de Create/Admit/Checkpoint/Complete/Cancel/Retry/Crash/Reconcile. Une horloge injectée rend le vieillissement déterministe. Les petites primitives mémoire peuvent être explorées avec [Loom](https://docs.rs/loom/latest/loom/), mais les tests SQLite/OS restent distincts : une preuve sur un modèle réduit n'est pas une exploration exhaustive de toute l'application.

Les campagnes réelles utilisent une base temporaire et le même writer que le produit, des barrières explicites et une trace globale d'opérations début/fin/commit/effet. L'oracle de capacité est indépendant du compteur UI. Chaque échec conserve seed, ordre exact, snapshot DB, logs des fixtures et contre-exemple minimisé. Le nombre de répétitions est déclaré ; il n'est jamais présenté comme preuve d'absence de course.

La sous-campagne WAL ne se réduit pas à injecter disque plein : elle retient une vraie lecture, mesure le fichier WAL et la file writer pendant une arrivée contrôlée, puis force expiration, quota d'export et reprise. Les seuils et la taille maximale d'une transaction sont fixés dans le manifeste avant l'essai. L'oracle compare les commandes admises et receipts avec les lignes réellement durables ; il ne déduit pas la maîtrise de la pression d'un compteur d'interface revenu à zéro. Statut : `not_run`.

## Charge mixte R3–R5

T-F019-007 et T-E2E-I/R/W/X/Y : lancer vingt leaders dormants, des invocations N actives, des replays navigateur et des tâches de supervision. Pour chaque préfixe, total_agent_actif + total_replay_actif + total_provider_probe_actif ≤ effective_limit ≤ 50 ; un provider_probe de configuration est lancé pendant la saturation. Le défaut cinq ne devient pas cinq de chaque classe. Zéro invocation modèle pendant un replay ne signifie pas absence de consommation de ressources. Les transitions pures du reducer n’occupent pas une unité par nœud.

Le test maintient deux contextes navigateur, le défaut V1, puis suspend l’un d’eux : son unité peut être libérée après checkpoint/neutralisation, mais sa réservation mémoire/processus reste comptée si le contexte est conservé. Saturer aussi le sous-quota local modèle, fixé à un par défaut (`max_inflight=1`, Q-19) ; un manager admis globalement peut rester en attente de fournisseur sans dépasser le quota. Un provider dont l’annulation est inconnue ne rend pas sa capacité disponible par simple fermeture de socket. Les réservations et le statut de cette attente sont distincts et visibles.

Créer en continu des événements de logs pendant un replay forbidden : ils sont persistés/coalescés sans appels managers. Le même test en supervised n’invoque qu’aux checkpoints déclarés et respecte la fréquence/budget. Une réparation préautorisée séparée compte dans la capacité et le budget mission ; elle ne crée aucun slot invisible.
