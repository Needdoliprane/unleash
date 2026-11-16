# Modèles locaux — contrat proposé pour la V1

**Date : 16 novembre 2026. Statut : spécification, non implémentée.** LM Studio, Ollama et « CPP », interprété ici comme le serveur llama.cpp, entrent dans la V1 de F012. L’interprétation du dernier nom reste explicite. Les faits fournisseurs sont vérifiés dans [les sources primaires datées](../research/local-model-sources.md). Les mécanismes Unleash ci-dessous sont adoptés par [ADR-17](../architecture/adr/ADR-17-local-model-providers.md), accepté en R7 ; la matrice qualifiée est celle de Q-19 ([plan de qualification](../testing/local-model-tests.md)), qui ne restreint pas les modèles que l'utilisateur peut configurer.

## Périmètre

Le cœur Rust appelle des serveurs d’inférence existants par HTTP. Il conserve sélection, autorisation, budgets, contexte, traces et résultats. Les moteurs natifs externes gardent leurs langages et leur cycle de vie ; aucun moteur d’inférence n’est réécrit pour satisfaire la contrainte Rust.

La voie initiale est un endpoint déjà configuré et géré par l’utilisateur. Unleash ne télécharge pas un modèle, n’installe pas un runtime et ne lance pas un processus lourd à l’ouverture de Settings. Le lancement d’un serveur explicitement géré peut ensuite utiliser F011 avec executable/configuration identifiés, mandat, logs, génération et arrêt contrôlé. Une commande de maintenance n’accorde pas l’autorisation de modifier les modèles d’un autre client utilisant le même serveur.

## Profils natifs, dialectes distincts

| Profil | Inférence et inventaire de référence (R7, ADR-17) | Qualification spécifique |
| --- | --- | --- |
| `lmstudio` | Inférence par Chat Completions compatible ; inventaire par l'API native de LM Studio (version ≥ 0.4.0). | Version du serveur, modèle, capacités, options et placement effectif, dont configuration LM Link. |
| `ollama` | Inférence par l'API native `/api/chat` ; inventaire par `/api/tags` et `/api/show` ; une voie compatible éventuelle reste un profil de dialecte distinct. | Modèle, template/paramètres, métadonnées, configuration locale/cloud (relais cloud désactivé) et métriques réellement disponibles. |
| `llama_cpp` | `llama-server` au build figé, templates Jinja activés ; inférence par Chat Completions compatible. | Build, modèle/poids, template, quantification, configuration de contexte et éventuel calcul déporté. |

Ces identités permettent une configuration lisible et des diagnostics propres au backend ; elles ne promettent pas une API uniforme. Une extension OpenAI-compatible générique reste distincte. Le profil épingle le dialecte, les routes utilisées et les options autorisées. Aucune option inconnue n’est ignorée pour donner l’apparence d’un succès.

La liste des modèles est une observation, pas une preuve de capacité. La qualification porte sur le tuple serveur/build, modèle, révision ou digest disponible, template, quantification et configuration. Elle distingue données déclarées par le backend, tests exécutés, résultats inconnus et limites. Un changement significatif invalide le snapshot de capacités applicable.

## Sélection et règles de priorité

La [priorité canonique](model-routing.md) s’applique sans voie locale parallèle : override autorisé de tâche/invocation → agent → rôle épinglé → skill principal → étape/workflow → projet → profil global, puis valeurs par défaut du fournisseur. La sélection expose la provenance de chaque champ. Les contraintes de données, capacités, sécurité et ressources s’intersectent ; une préférence plus spécifique ne peut les élargir.

Un modèle local peut donc être choisi pour la préparation d’une procédure, un agent spécialisé, un checkpoint de supervision ou une mission de développement, si les capacités et données le permettent. Ce choix ne prouve pas qu’il est assez compétent pour chaque tâche. Le contrat de sortie et les preuves du travail restent contrôlés indépendamment de l’endroit où le modèle tourne.

## Deux axes indépendants : appeler un modèle et localiser les données

| Politique | Sens proposé |
| --- | --- |
| `AutomationRun.model_policy = forbidden` | Aucun appel modèle local ou distant, y compris manager, résumé, embedding, réparation et dépendance transitive. Aucun préchauffage ou démarrage d’un moteur pour ce run. |
| `AutomationRun.model_policy = allowed_bounded` | Appels déclarés et bornés autorisables, notamment aux checkpoints de supervision, sous les autres contraintes. |
| `data_locality = local_only` | Les données soumises à cette politique restent sur la machine où tourne le daemon Unleash pour leur traitement modèle. Le placement doit être qualifié ; loopback, marque et déclaration utilisateur seuls ne suffisent pas. |
| `data_locality = approved_remote` | Des destinations distantes explicitement autorisées sont éligibles, selon identité, scope, rétention, capacités et budgets. Ce n’est pas une autorisation générale de tout cloud. |

Un serveur LAN appartient à une autre machine et relève du distant, même s’il est personnel. Un proxy loopback dont l’aval n’est pas vérifié reste de placement inconnu. LM Link, Ollama cloud et un calcul déporté doivent être pris en compte ; les [sources](../research/local-model-sources.md) documentent pourquoi l’URL ne suffit pas.

Pour `local_only`, le profil conserve une preuve de placement liée à sa configuration et à sa fraîcheur : serveur/processus identifié lorsque observable, modèle choisi, paramètres de routage, destinations auxiliaires et méthode de vérification. Une simple case cochée « local » n’établit pas la garantie. Une observation réseau bornée n’est pas une preuve universelle d’absence future de fuite ; la qualification doit préciser les contrôles effectifs, leur portée et leurs limites. Si les garanties nécessaires ne peuvent pas être vérifiées, l’admission stricte échoue avec `InferenceLocalityUnknown`.

Le fallback conserve cette politique. Une indisponibilité, un dépassement mémoire ou une mauvaise réponse locale ne permet pas d’envoyer le contexte au cloud. Un fallback local autorisé reste possible s’il satisfait les mêmes capacités et budgets ; une cible inconnue est refusée. Passer à une destination distante exige une nouvelle autorisation applicable et les règles communes de nouvelle invocation/manifeste, sans réécrire l’appel précédent.

Le contrôle `forbidden` précède chargement, préchauffage, probe de génération et transmission. Il interdit aussi le recours à un « petit modèle local » présenté comme simple vérification. Une lecture de métadonnées réellement passive reste distincte ; elle ne doit pas déclencher implicitement le chargement. Un test utilisateur indépendant de configuration possède sa propre cause et son propre mandat ; il ne peut servir à contourner la politique d’un run.

## Transport, authentification et frontière de confiance

| Cas | Mécanisme proposé | Vérification |
| --- | --- | --- |
| HTTP loopback qualifié | Exception explicite limitée à l’adresse, au port et au profil autorisés. Adresses effectives contrôlées, redirections non suivies par défaut, proxy implicite désactivé pour cette voie. | Cible non loopback, changement de résolution ou redirection refusés avant transfert du contexte. |
| Backend local sans authentification | Mode `none` uniquement si supporté et explicitement enregistré dans le profil loopback qualifié. Aucun faux token exigé par un client tiers. | Probe prévu du profil ; aucune clé absente n’est convertie en erreur universelle. |
| Backend local avec token | Secret référencé, résolu seulement dans le transport autorisé ; permissions et révocation visibles. | Secret absent des événements, erreurs et renderer ; rejet d’un token invalide. |
| Endpoint LAN ou distant | Destination autorisée, authentification et TLS qualifiés directement ou via une frontière de proxy explicitement enregistrée. | Identité serveur/certificat et authentification vérifiés ; HTTP non loopback refusé dans ce profil V1. |
| Serveur à outils ou MCP intégrés | Capacités désactivées pour les effets de bord tant qu’un contrat d’autorisation, de trace et de localité n’est pas qualifié. | Aucun appel hébergé ne contourne le broker ni n’envoie les données à une destination non autorisée. |

Une santé HTTP positive ne prouve pas que l’authentification protège la route d’inférence. Les contrôles sont donc séparés. L’exception loopback n’est ni une preuve d’isolation des processus du même compte, ni un certificat d’absence d’intermédiaire. F017 porte la frontière de sécurité ; ce document ne promet pas une protection OS non implémentée.

Unleash ne découvre pas tous les services du réseau par scan et ne modifie pas automatiquement pare-feu, liaison du serveur ou paramètres cloud. Les endpoints sont sélectionnés explicitement. Une modification de configuration proposée reste concrète, inspectable et soumise au mandat applicable.

## Capacités et handshake

Le handshake proposé sépare validation de configuration, connexion, inventaire et probe de génération. Il peut s’arrêter après une phase ; consulter Settings ne lance pas un benchmark. Un probe qui peut charger un modèle ou consommer des ressources est annoncé, borné et traçable.

Le snapshot de capacités décrit texte, images si disponibles, streaming, outils, choix d’outil requis, JSON/contraintes de sortie, contexte, usage et interruption. Les capacités nécessaires au rôle sont éprouvées par des fixtures déterministes côté client et des demandes bornées côté serveur. Des arguments d’outil doivent être entièrement reçus et validés avant effet. Un serveur qui accepte un champ mais l’ignore n’est pas conforme à ce champ.

Les limites de contexte effectivement configurées priment sur un maximum annoncé pour la famille du modèle. Les paramètres d’effort, de pensée, de sampling et de sortie suivent le dialecte qualifié. Un changement de template ou de modèle derrière le même alias est visible et peut produire `CapabilitySnapshotStale`. Les sessions et décisions restent durables dans Unleash, indépendamment d’une mémoire de conversation offerte par le serveur.

## Ressources, chargement et annulation

Le plafond global reste celui de F003 : cinq jobs actifs par défaut, plafond absolu cinquante, en totalisant `agent_invocation`, `replay_worker` et `provider_probe`. Les invocations agentes constituent un sous-compteur de ce total. Ce plafond ne signifie pas cinquante générations simultanées possibles sur un GPU. **Défaut V1 (R7, ADR-17) : `max_inflight = 1` par serveur local**, relevable seulement sur preuve consignée par tuple ; une admission de ressources s’ajoute aux slots globaux et aux budgets.

Une file durable borne l’attente locale. Connexion, attente d’admission, chargement à froid, premier token, génération totale et annulation possèdent des délais distincts. Défauts V1 (Q-19) : connexion loopback 2 s, chargement à froid 180 s, premier token 60 s, génération totale 300 s. Les observations cold/warm, mémoire ou GPU ne sont affichées que si réellement disponibles ; l’absence de télémétrie ne devient pas « mémoire suffisante ». Le partage avec d’autres clients du serveur reste une source de variation.

Un dépassement mémoire, un modèle introuvable ou un chargement échoué produit une erreur précise. La politique peut attendre, réduire un paramètre autorisé dans un nouvel appel visible, ou proposer un autre profil compatible. Elle ne télécharge pas de nouveaux poids ni ne modifie le serveur silencieusement. Une réduction de contexte ne tronque pas le goal ou les sources sans décision explicite.

Annuler le client ne prouve pas l’arrêt physique du calcul. L’implémentation utilise le mécanisme qualifié du backend et conserve l’état inconnu si nécessaire. Elle ne lance pas immédiatement une deuxième génération comme si toute capacité avait été libérée. L’arrêt d’un serveur géré exige propriété et génération correctes ; un serveur partagé fourni par l’utilisateur n’est pas tué pour annuler un seul appel.

## Manifest, usage et coût

`ModelCallManifest` conserve avant dispatch le profil et dialecte exacts, l’identifiant demandé, les métadonnées observées à l’admission, version/build, référence de snapshot de capacités, contexte réellement préparé, politique de données et preuve de placement. Digest des poids, template, quantification, configuration de contexte et matériel sont conservés lorsqu’ils sont alors disponibles et autorisés ; sinon la valeur reste absente avec une raison. Il ne faut pas fabriquer un hash à partir du seul nom du modèle et le présenter comme celui des poids.

Une identité ou métrique révélée dans la réponse est ajoutée comme observation liée dans `ModelCallRecord`, sans enrichir rétroactivement le manifest immuable. Une divergence entre modèle attendu et modèle rapporté est explicite et peut invalider la qualification pour les prochains appels ; elle ne réécrit pas la provenance préparée.

Les métriques distinguent durée de chargement, génération, tokens déclarés, estimations et valeurs inconnues. Une absence d’usage retourné n’est pas un zéro. Une inférence sans facturation fournisseur peut enregistrer ce fait, tandis que temps machine et éventuelle estimation de coût matériel restent séparés ; aucun coût énergétique précis n’est supposé.

L’interface affiche connexion, disponibilité, modèle chargé ou inconnu, localité vérifiée ou inconnue, capacités, erreur et prochaine action utile. GUI et TUI permettent même création de profil, paramétrage, test, inspection, sélection par portée et configuration du fallback. Les contrôles avancés restent accessibles depuis le parcours guidé, sans imposer à un novice leur configuration avant toute tâche.

## Acceptation et limites

Les chaînes REQ-MODEL-013 à 018 dans [F012](../features/F012-model-api-router/requirements.md) couvrent les trois backends, capacités, localité, ressources, manifests et parité avec la politique sans LLM. Le [plan de tests locaux](../testing/local-model-tests.md) distingue doubles, serveurs réels et contrôles négatifs. La réussite d’un test de texte ne qualifie pas les outils, la vision ou la reprise. Aucun test local ni téléchargement de modèle n’a été effectué dans la présente mission.

## Premier test avant création d’agent

Le [contrat ProviderProbeContext](../features/F012-model-api-router/interfaces.md) permet de qualifier une génération depuis Settings sans agent ni session de mission. Les données sont synthétiques, l’origine control_plane est authentifiée et les droits d’un run éventuel sont hérités. Le probe partage les réservations globales 5/50 et locales ; il ne constitue pas une invocation N cachée. Aucune lecture de configuration ne lance ce probe implicitement.
