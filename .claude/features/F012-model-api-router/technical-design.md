# F012 — conception technique Rust

## Pipeline Rust proposé

Les modules selection, capabilities, context, budget, transport et normalization restent sous une façade ModelRouter. Le noyau reçoit des DTO internes ; framing des flux, routes et headers restent dans les adaptateurs. Les adaptateurs sont écrits par Unleash ([ADR-11](../../architecture/adr/ADR-11-model-api-abstraction.md), accepté R7) : `reqwest` avec `rustls` et vérification des certificats par le magasin de la plateforme, parseur SSE interne borné, DTO `serde` stricts refusant les champs inconnus du dialecte qualifié. Aucun SDK fournisseur, aucune passerelle tierce dans le chemin critique et aucun SDK CLI d’abonnement. LM Studio, Ollama et llama.cpp sont des profils natifs qui réutilisent leurs moteurs d’inférence externes.

Selection résout les préférences selon l’ordre canonique de la spécification, depuis un snapshot de configuration et des ModelSelectionBinding exacts pour le skill principal. Le contrôle de politique interdit un appel modèle avant tout chargement ou préchauffage si le run est `forbidden`. Capabilities calcule l’intersection des besoins des skills, du contrat et des permissions ; la preuve de placement doit satisfaire la politique de toutes les données et de leurs dérivés. Context résout la filiation et produit un manifest v2 immuable en historique explicite ; Budget réserve avant réseau ; Transport revalide les autorités et conserve l’identifiant local d’action et les IDs retournés ; Normalization produit un résultat fini ou incertain. Le mode de génération n’est pas modifié en milieu d’appel.

## Persistance et concurrence

Stocker provider_profiles, model_profiles et model_selection_bindings en versions, model_catalog_snapshots (Q-14), model_calls par action_id, context_manifests adressés par hash, politiques/filiations immuables, décisions de déclassification, budget_reservations et usage_records. Les noms de stockage sont proposés ; aucune migration SQL n'est prétendue livrée. Une invocation peut faire plusieurs appels dans ses plafonds, mais ne possède qu’une tentative active admise. L’autorisation d’un fallback est recalculée ; elle n’est pas un retry de la même requête vers une autre entreprise.

La table d’usage conserve les composantes tarifaires et la version de prix appliquée. Les estimations avant envoi, usages fournisseur, coût estimé et coût confirmé ne sont pas fusionnés. Un tarif périmé bloque par défaut l’admission (BudgetExceeded/PRICING_STALE, retry_class after_refresh) ; `warn` n’est possible que par choix explicite du profil, avec alerte et marquage des réservations ; un modèle distant sans tarif applicable bloque toujours (BudgetExceeded/PRICING_UNKNOWN, retry_class after_human_decision) ; un profil local déclare `pricing=none_local` et reste borné par tokens, temps et ressources ; jamais de valeur zéro. Une réservation garde son `pricing_version` : un nouveau catalogue ne s’applique qu’aux admissions suivantes (Q-14).

Le binding modèle du skill est une configuration externe à sa version publiée : clé exacte skill/version/namespace et scope de configuration, versions append-only, unicité du binding actif et suppression par tombstone. `SetSettings` engage mutation et reçu sous révision. Le resolver choisit repository > projet > global à ce niveau seulement, puis applique la priorité inter-niveaux ; il épingle profil et binding dans le bundle. Une mutation concurrente force une résolution cohérente du snapshot, sans assemblage de références anciennes/nouvelles. Les paramètres natifs ne traversent pas implicitement un changement de profil.

## Contexte explicite et politiques dérivées R6

Le graphe de filiation porte les sources effectivement utilisées et leurs politiques exactes. Le module Context applique la fermeture transitive et l'intersection d'ACL, localité, destinations et contraintes de conservation décrites dans [model-routing](../../specs/model-routing.md). Les index et extraits de récupération documentaire gardent les restrictions de leurs objets sources ; un résumé ou une sortie de modèle hérite par défaut de tous les segments inclus du producteur. Une extraction ne devient pas publique parce qu'elle ne cite plus son document.

Construire les politiques et la filiation avant la persistance des contenus dérivés ; si le stockage lui-même n'est pas autorisé, ne conserver que le diagnostic et les métadonnées permises. Une décision de déclassification autorisée est séparée, enregistrée par la seule commande `RecordDeclassificationDecision` ([interfaces](interfaces.md)), liée au hash exact, bornée et réévaluée avant dispatch. Une référence manquante, un cycle ou une politique inconciliable bloque les contenus nécessaires ; exclure un segment facultatif exige un motif et un nouveau payload/hash cohérent. Les tombstones empêchent qu'une purge transforme un dérivé en source non classifiée. Une indisponibilité du contrôle n'autorise aucune valeur permissive par défaut.

Le sérialiseur utilise exclusivement `explicit_unleash_history` ; son allowlist interdit les références de continuation serveur et leurs équivalents qualifiés dans chaque dialecte. Les IDs fournisseur observés sont des métadonnées de reçu, jamais une entrée automatique du prochain appel. Les options de stockage applicatif sont explicites lorsqu'elles existent et compatibles avec la politique de rétention qualifiée du modèle/endpoint/compte ; les appels OpenAI Responses portent `store=false` (Q-07). Leur désactivation ne démontre pas l'absence de tout journal fournisseur.

Les objets natifs opaques nécessaires sont archivés sous la politique de leurs antécédents et explicitement sérialisés avec hash et métadonnées de qualification. Un objet qui nécessite un historique distant caché n'est pas qualifié V1. Une exclusion/révocation d'antécédent invalide sa réutilisation ; la reconstruction portable ne prétend pas reconstruire les octets privés perdus. Estimations de contexte et réservations comptent tous les éléments explicitement fournis selon les règles qualifiées du dialecte, dont les blocs opaques ; si leur coût ne peut être borné, la demande attend une qualification au lieu de compter zéro.

## Streaming et fin de tour

Un assembleur borné traite le stream par index d’item et identifiant d’outil. Un fragment d’arguments ne déclenche jamais l’exécuteur. Une réponse terminale valide engage résultat et intentions d’outils, puis la boucle continue via le broker. Une interruption avant fin ne devient ni succès ni preuve de tests.

Un worker qui demande une décision d’un manager effectue un checkpoint ; il n’attend pas en conservant son slot. Une génération distante dont le client est coupé peut continuer chez le fournisseur : la limite locale ne promet pas son arrêt physique.

## Erreurs et confidentialité

Distinguer InvalidModelConfig, ModelConfigurationConflict, CapabilityUnavailable, ContextBudgetExceeded, ContextLineageIncomplete, DataPolicyConflict, RemoteContextReferenceForbidden, BudgetExceeded, ProviderRateLimited, ProviderUnavailable et OutcomeUnknown. Un refus ou une troncature reçus du fournisseur sont des ModelOutcome `refusal`/`truncated` (événement ModelCallCompleted au statut non réussi), jamais une Error de transport ; ProviderRefused et OutputTruncated ne servent que de reason_code lorsqu'une invocation échoue pour cette cause. Seule l'absence de réponse exploitable produit ProviderUnavailable ou OutcomeUnknown. Une erreur comprend retry_class et éventuel retry_at, jamais une commande automatique de relance. Les conflits de configuration/politique et références de contexte interdites ne se retentent pas sans correction des préconditions. La reprise est décidée par le runtime selon idempotence et mandat.

Projection wire enregistrée par F012 (Error commun de l'[IPC](../../architecture/ipc-and-apis.md)) : InvalidModelConfig → InvalidArgument/INVALID_MODEL_CONFIG (never) ; ModelConfigurationConflict → ContractViolation/MODEL_CONFIGURATION_CONFLICT (after_human_decision) ; CapabilityUnavailable → CapabilityDenied/CAPABILITY_UNAVAILABLE (after_human_decision) ; ContextBudgetExceeded → BudgetExceeded/CONTEXT_BUDGET_EXCEEDED (after_human_decision) ; ContextLineageIncomplete → ContractViolation/CONTEXT_LINEAGE_INCOMPLETE (after_human_decision) ; DataPolicyConflict → ContractViolation/DATA_POLICY_CONFLICT (after_human_decision) ; DeclassificationAuthorityRequired → CapabilityDenied/DECLASSIFICATION_AUTHORITY_REQUIRED (after_human_decision) ; RemoteContextReferenceForbidden → CapabilityDenied/REMOTE_CONTEXT_REFERENCE_FORBIDDEN (never) ; BudgetExceeded → BudgetExceeded/BUDGET_EXHAUSTED (raison transverse BudgetExhausted de l'IPC, after_human_decision) pour une réservation impossible, BudgetExceeded/PRICING_STALE (after_refresh) pour un tarif périmé sous `block`, ou BudgetExceeded/PRICING_UNKNOWN (after_human_decision) pour un modèle distant sans tarif applicable ; ProviderRateLimited → RateLimited/PROVIDER_RATE_LIMITED (after_backoff) ; ProviderUnavailable → ResourceBusy/PROVIDER_UNAVAILABLE (after_backoff) ; OutcomeUnknown → OutcomeUnknown/MODEL_CALL_OUTCOME_UNKNOWN (after_reconciliation) ; ProviderRefused → ContractViolation/PROVIDER_REFUSED et OutputTruncated → ContractViolation/OUTPUT_TRUNCATED (after_human_decision).

Valeurs R7 de transport et d'activation (Q-07) : au plus 3 tentatives pour 429, 5xx ou indisponibilité lorsque l'envoi n'est pas engagé ou que le fournisseur a refusé ; backoff exponentiel 1 s, 2 s puis 4 s avec jitter complet ; `Retry-After` respecté jusqu'à 300 s, au-delà `RateLimited` visible. Ces tentatives sont des dispatchs de la même action. Une issue inconnue après envoi n'est jamais rejouée comme un échec certain. Un profil cloud n'est actif qu'après `TestProviderProfile` réussi ; si l'inventaire du compte ne liste pas le modèle recommandé (`gpt-6.1-sol`, `claude-fable-5-2`), le profil reste `blocked` avec motif, sans substitution silencieuse.

Les credentials ne transitent que du coffre au transport autorisé. Les documents et tous leurs dérivés ont des destinations et obligations de conservation effectives ; certains modèles ont des règles de rétention propres. Le contrôle précède l’assemblage et le fallback, puis vérifie les révocations avant dispatch. Les blocs opaques restent rattachés au fournisseur et à leurs sources, aucune UI ne les présente comme pensée privée déchiffrée.

## Ordre interne de construction

La préparation interne d’un serveur Ollama géré est enregistrée séparément
des sondes et de la configuration, dans [le schéma V019](../../architecture/sqlite-design.md#préparation-dun-serveur-géré-v019).
Le writer fige le profil local exact, le binaire, les digests de commande,
d’environnement et de confinement et la date limite. Ce document conserve
le plan à vérifier par le producteur natif. Le dispatch et le permis de
démarrage restent fermés jusqu’à la qualification du chemin Strict et à
son engagement durable dans le broker.

Figer les DTO puis créer les doubles déterministes ; implémenter la voie nominale ; ajouter persistance et idempotence ; injecter les pannes aux frontières ; qualifier les adaptateurs réels ; raccorder clients et rapports. Les interfaces de feature sont proposées, et les APIs de bibliothèques doivent correspondre aux versions verrouillées au jalon de build.

Ne pas ajouter de service séparé sans besoin démontré. Le daemon possède la coordination ; les adaptateurs ne créent ni workflow parallèle ni gestionnaire de budget caché.

## Qualification et admission locales

La [spécification locale](../../specs/local-models.md) fixe les propositions de détail. Un snapshot de capacités est lié à un profil, au build, au modèle et à sa configuration, avec preuves et inconnues. Les champs natifs restent confinés au dialecte approprié. Une liste de modèles ne constitue pas à elle seule un handshake réussi pour tool use, vision, sorties contraintes ou annulation.

L’admission locale applique `max_inflight=1` par serveur, défaut V1 R7 ([ADR-17](../../architecture/adr/ADR-17-local-model-providers.md), Q-19), relevable seulement sur preuve consignée par tuple. Elle s’ajoute aux slots F003 et aux budgets ; elle ne transforme pas le plafond absolu partagé de cinquante unités actives, agent_invocation, replay_worker ou provider_probe en promesse de cinquante générations GPU. Les attentes sont durables et les ressources libérées selon l’état effectivement observé. Délais de connexion, file, chargement, premier token, génération et arrêt sont distincts.

Un endpoint déjà géré par l’utilisateur est la voie initiale. F011 intervient seulement si le lancement d’un processus de serveur est explicitement géré. Le router ne télécharge pas de poids, ne change pas le routage cloud et n’arrête pas un serveur partagé pour résoudre une simple erreur de génération. Un OOM ne déclenche pas un fallback hors politique.

### Lancement explicite d’un service confiné sur macOS

Le bootstrap privé `strict_service_v1` de `unleash-process-host` exige une
ressource Service, le mode Strict et un contexte d’allocation. Il applique le
profil de commande Seatbelt avec toutes les exceptions de sortie désactivées.
Le programme règle son adresse d’écoute. Les chemins privés du host,
notamment le token de contrôle, les canaris et le profil, sont protégés
contre la lecture et l’écriture de l’enfant. Le répertoire de travail privé
et son `child-tmp` se trouvent dans une racine d’écriture assignée, séparée
du runtime du host.

L’environnement fourni au lancement est une carte fermée de valeurs non
secrètes, avec un `TMPDIR` géré. Le host calcule son empreinte canonique à
partir des clés et valeurs, contrôle l’empreinte attendue avant spawn, puis
la restitue dans le manifeste et l’accusé de handshake. Cette empreinte
couvre l’environnement fourni par le host. Le processus et les bibliothèques
chargées peuvent ensuite modifier leur environnement.

L’attestation de lancement reçoit le bootstrap autorisé et l’identité
OS complète du host détenu. Sur le socket Unix privé, le client compare
l’UID et le PID du pair fournis par le noyau, puis l’incarnation OS
avant d’envoyer le secret conservé par le propriétaire du lancement.
Après le handshake corrélé à la lignée, à l’époque et au fence attendus,
il relit le manifeste, le profil Seatbelt et l’identité fraîche de
l’enfant. Les empreintes du contexte, de l’environnement et du profil
doivent correspondre. Le handshake suit le protocole privé existant
sans rotation du secret. Le rattachement durable d’un lancement F012
au profil exact reste une obligation du daemon et du writer.

Le banc natif couple ce lancement à une fixture TCP loopback. Il vérifie
l’identité OS issue de l’enfant détenu, l’écoute, le socket accepté et le
descripteur client exact avant d’envoyer des octets synthétiques. Un second
listener de fixture permet de contrôler le refus d’une connexion sortante.
L’admission Q-19 exige en plus le serveur et le modèle réels, leur build,
l’autorité de placement et la vérification du FD utilisé par le transport.

## Localité, transport et erreurs

`data_locality=local_only` exige un placement qualifié sur la machine du daemon ; `approved_remote` permet seulement les destinations déjà autorisées. Les profils conservent une preuve de placement et sa fraîcheur. Les fonctions de relais telles que celles relevées dans les [sources](../../research/local-model-sources.md) rendent insuffisante une vérification de l’URL seule. Un proxy ou placement inconnu est bloqué dans le mode strict.

HTTP sans TLS est une exception explicite pour le loopback qualifié, limitée à l’endpoint enregistré ; redirections et proxies implicites ne doivent pas transférer le contexte ailleurs. Les endpoints non loopback suivent authentification et TLS qualifiés. Le mode sans secret n’est admis que lorsque le backend et la politique le permettent. Les credentials éventuels restent des SecretRef.

Dans la variante interne à pair géré de Connect et Inventory, l’appelant
fournit l’adresse loopback exacte et l’identité OS complète du processus
attendu. La requête gelée lie ces deux données à l’empreinte du transport
sous `unleash.owned_loopback_request.v1`. L’intention et la décision du
broker autorisent cette empreinte. La variante passive garde son empreinte
propre.

L’exécuteur observe le listener avant l’unique connexion. Il vérifie
ensuite le FD client et le socket accepté de l’autre côté avant TLS ou
HTTP. En attendant `accept`, il réessaie l’observation sur le même FD
dans le délai d’origine. Les écarts d’identité, de listener ou de
descripteur et la fin du délai ferment la connexion. Un échec après
dispatch reste à réconcilier. Le rapprochement du service avec le profil
durable appartient au daemon qui détient le lancement. La qualification
Q-19 exige en plus les preuves de serveur, de modèle et de confinement.

Erreurs Unleash supplémentaires : ModelPolicyForbidden (wire `CapabilityDenied/MODEL_CALLS_FORBIDDEN`, raison transverse d'[ipc-and-apis](../../architecture/ipc-and-apis.md), même raison que F019, never), DataLocalityViolation (CapabilityDenied/DATA_LOCALITY_VIOLATION, never), InferenceLocalityUnknown (CapabilityDenied/INFERENCE_LOCALITY_UNKNOWN, after_human_decision), CapabilitySnapshotStale (ContractViolation/CAPABILITY_SNAPSHOT_STALE, after_human_decision : probe explicite), LocalModelUnavailable (ResourceBusy/LOCAL_MODEL_UNAVAILABLE, after_backoff), LocalQueueFull (ResourceBusy/LOCAL_QUEUE_FULL, after_backoff), LocalLoadTimeout (ResourceBusy/LOCAL_LOAD_TIMEOUT, after_backoff) et LocalResourceExhausted (ResourceBusy/LOCAL_RESOURCE_EXHAUSTED, after_human_decision). Elles ne sont pas présentées comme des codes retournés uniformément par les fournisseurs. L’adaptateur conserve le code observé et la raison de normalisation. Une annulation sans preuve d’arrêt reste incertaine, même pour un processus situé sur la même machine.

Le manifest conserve build/modèle/configuration observés et leurs absences. Les compteurs, coûts et durées manquants restent inconnus. La facturation fournisseur éventuellement nulle ne mesure pas le coût matériel. Les fixtures et gates réels sont dans [local-model-tests](../../testing/local-model-tests.md) ; aucun essai local n’a été exécuté dans ce dossier.

## Origine des appels R6

Un ModelRequest possède ModelOrigin invocation ou provider_probe selon [interfaces](interfaces.md). L’adaptateur reçoit dans les deux cas un contexte durable immuable, politiques effectives, budget, action et fence. La variante probe ne peut utiliser de données utilisateur ni d’outils réels et ne crée aucun agent factice. Le scheduler réserve son unité dans le même pool ; un échec ou timeout suit le journal d’effet, pas une boucle ad hoc du panneau Settings.
