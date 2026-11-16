# Audit indépendant — Expert 5, API LLM et contexte

Analyse conduite par l’agent principal avant consolidation, avec sources officielles datées du 16 novembre 2026. Ce regard est distinct des trois groupes de revues déléguées ; il ne prétend pas provenir d’un quatrième processus simultané.

**Statut de révision :** l’analyse initiale cloud ci-dessous reste une trace datée. L’addendum R4/R5, rédigé par le sous-agent documentaire `generalist_product_r3`, complète les conclusions actuelles pour les trois backends locaux et les automatismes. Il ne se présente pas comme une nouvelle revue indépendante de la spécification locale rédigée par le même auteur. Aucun essai réel cloud ou local n’est ajouté.

## Constats

Les intentions de configuration Sol et Fable correspondent aujourd’hui à des IDs documentés. La [matrice vérifiée](../research/verified-api-capabilities.md) conserve endpoints, limites et restrictions. Il n’existe pas de justification pour dépendre d’un SDK d’agent fournisseur ou d’un abonnement CLI. Le transport HTTP/SSE Rust permet de conserver les états et politiques dans Unleash.

Une API qui ressemble à OpenAI peut accepter un champ et l’ignorer. Un parseur commun n’établit ni sorties structurées effectives, ni coût fiable, ni annulation, ni support d’outils parallèle. Le profil fournisseur doit annoncer ses capacités et les qualifications effectuées.

## Objections et risques

API-R1 : le routage par « meilleur modèle du rôle » ne résout pas un conflit entre deux skills qui demandent des modèles différents. Une règle de priorité arbitraire au dernier skill rend les résultats non reproductibles. Recommandation : choix principal de tâche explicite, et erreur de configuration en cas de contraintes incompatibles.

API-R2 : résumer puis supprimer tout l’historique détruit les preuves de contexte. Le résumé doit pointer vers ses sources, porter un hash et signaler les omissions. Sa qualité n’est pas garantie par la validité JSON.

API-R3 : les retries peuvent doubler la facture après rupture réseau, et un appel hébergé peut avoir produit un effet avant timeout. Une clé d’idempotence interne n’oblige pas un fournisseur à dédupliquer. L’état incertain conserve réservation et demande réconciliation.

API-R4 : un fallback d’Anthropic vers OpenAI change le destinataire des documents. Il doit être autorisé explicitement par politique du projet ; une erreur de quota n’autorise pas l’exfiltration vers un fournisseur supplémentaire.

API-R5 : un modèle de raisonnement peut refuser un appel d’outil forcé ou des paramètres de sampling. Il faut valider au niveau modèle, pas seulement fournisseur.

## Hypothèses et alternatives

H-API-1 : garder une représentation portable des messages, des résultats d’outils et des pièces transmises, plus des blocs natifs opaques rattachés à leur fournisseur. Alternative : tout stocker dans une conversation fournisseur. Cette alternative réduit le code local mais échoue à la reprise indépendante, au changement de modèle et à l’inspection durable.

H-API-2 : adaptateurs HTTP natifs Rust, enveloppe commune petite. Alternative : SDK communautaire Rust sous adaptateur. Acceptable après audit et verrouillage ; aucun statut « officiel » ne doit lui être attribué sans preuve.

## Recommandations

Séparer ModelSelection, CapabilityValidation, ContextAssembly, BudgetReservation, ProviderRequest, StreamAssembly et ResultCommit. Les trois premiers produisent une configuration effective immuable. Aucun fragment d’arguments reçu en streaming n’exécute un outil. Le commit du résultat fournisseur et des intentions d’outils précède leur dispatch local.

Ne pas standardiser un « reasoning_effort » à valeurs fictives. Stocker un choix sémantique utilisateur seulement si sa traduction vers le profil fournisseur est explicitement définie, sinon exposer la valeur native validée.

## Vérifications et acceptation

Le client doit rejeter avant réseau un effort non supporté, un fournisseur interdit et une sortie exigée non réalisable. Une interruption SSE au milieu d’un JSON ne lance aucun outil. Une même sortie finalisée ne déclenche pas deux actions. Un plafond budgétaire tient sous cinquante réservations concurrentes. Un changement de fallback est visible dans le contexte et le rapport.

Voir [F012](../features/F012-model-api-router/README.md), [routage](../specs/model-routing.md). L’accès réel des clés, le coût réel et les latences restent à confirmer ; aucun appel API authentifié n’a été effectué pour cet audit.

## Addendum R4/R5 — conclusions actuelles sur les modèles locaux

OpenAI, Anthropic et les profils compatibles restent couverts. **LM Studio, Ollama et llama.cpp sont des profils natifs supplémentaires de F012 en V1.** « CPP » est interprété comme le serveur HTTP llama.cpp ; l’interprétation reste signalée. L’intégration utilise leurs API qualifiées, sans réécrire l’inférence ni présumer un SDK Rust officiel. Les [sources locales](../research/local-model-sources.md) étayent les capacités documentaires, sans démontrer un accès ou un comportement réel de serveur.

| Repère | Objection actuelle | Réponse proposée et preuve nécessaire |
| --- | --- | --- |
| API-R6 | Imposer une clé à tous les profils bloque inutilement les backends locaux qui n’en exigent pas. | SecretRef facultative selon backend/authentification ; exception HTTP limitée au loopback qualifié ; authentification et TLS pour les cibles distantes. Tester absence de secret autorisée et refus des cas non autorisés. |
| API-R7 | Une compatibilité d’URL ne garantit pas outils, JSON, contexte ou interruption. | Qualifier le tuple serveur/build, modèle, template, quantification, dialecte et configuration ; invalidation sur dérive. |
| API-R8 | Un endpoint localhost peut relayer vers une autre machine ou un cloud. | Enregistrer preuve de placement et fraîcheur ; `local_only` refuse distant et inconnu. Les relais documentés incluent Ollama cloud et LM Link ; un proxy opaque reste non qualifié. |
| API-R9 | Une erreur mémoire ou un modèle lent peut déclencher un transfert cloud présenté comme simple secours. | Aucun fallback hors politique de données. File, chargement, génération, OOM et annulation incertaine doivent être distingués ; chaque fallback autorisé conserve manifeste et budget propres. |
| API-R10 | Un petit modèle local, un embedding ou un résumé peut être présenté comme compatible avec zéro LLM. | `forbidden` interdit la chaîne causale modèle locale et distante avant appel, chargement ou préchauffage. `allowed_bounded` permet seulement les points de supervision déclarés. |
| API-R11 | Le plafond de cinquante jobs peut être pris pour une promesse de cinquante générations sur un GPU. | Compter ensemble `agent_invocation` et `replay_worker` dans le 5/50 global ; ajouter admission locale proposée d’un appel par serveur et qualification matérielle. |
| API-R12 | Une identité de modèle reçue après l’appel peut réécrire le manifest déjà engagé. | Métadonnées disponibles à l’admission dans le manifest immuable ; observations ultérieures dans ModelCallRecord. Digest, coût ou métrique absent restent inconnus. |

> **Note R7 (16 novembre 2026) :** complément R6 à API-R11 : `provider_probe` compte aussi dans le 5/50 ([scheduling](../specs/scheduling.md)). Les profils cloud recommandés, le catalogue de tarifs et la matrice locale sont tranchés par Q-07, Q-14 et Q-19 ([révision R7](../brief/revision-2026-11-16-r7.md)).

Le parcours initial connecte un serveur préparé par l’utilisateur. L’installation de runtime, le téléchargement de poids et l’arrêt d’un serveur partagé ne sont pas des réparations automatiques du routeur. Une éventuelle gestion de processus passe par F011 avec propriété et génération contrôlées. La preuve d’annulation du calcul reste distincte de l’arrêt du client HTTP.

Le [contrat local](../specs/local-models.md) et [F012-013 à 018](../features/F012-model-api-router/acceptance.md) définissent les réponses de conception. Les [tests locaux](../testing/local-model-tests.md) doivent apporter une preuve réelle par backend ; des doubles ne suffisent pas à annoncer leur intégration. Ni coût nul ni fiabilité ou performance générale ne sont revendiqués.

Ces contrats servent aussi le [parcours guidé](../design/guided-automation-experience.md) et le [périmètre polyvalent](../brief/general-purpose-scope.md). Un modèle peut préparer une procédure ou superviser des blocs de script aux checkpoints autorisés ; il ne doit pas être invoqué à chaque clic ni imposé comme juge universel. La [recherche navigateur](../research/browser-automation-sources.md) et la [spécification d’automatisation](../specs/reusable-automations.md) conservent les limites propres aux outils et aux effets externes. La parité GUI/TUI inclut configuration, test, sélection, localité, budgets et inspection des erreurs.

**Verdict documentaire :** extension cohérente avec un routeur commun et les garde-fous proposés ; qualification réelle non réalisée. La lecture des sources n’est ni un benchmark ni une preuve d’absence de fuite sur une installation future.

