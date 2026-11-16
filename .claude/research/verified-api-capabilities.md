# Capacités API vérifiées et limites de vérification

Consultation : 16 novembre 2026. Sources primaires publiques, sans accès à un compte client, appel facturé ou inférence locale. Une disponibilité documentaire ne démontre pas l’accès du compte ou du serveur configuré. Le relevé cloud initial est conservé ; la révision R4 ajoute les trois backends locaux. Les IDs cloud ci-dessous sont documentés, sans être encodés dans le moteur de workflow ; les modèles locaux effectifs seront inventoriés sur les serveurs qualifiés.

## Profils recommandés pour les intentions du brief

| Intention | Identifiant documenté | Contraintes documentées utiles |
| --- | --- | --- |
| Développement Sol | gpt-6.1-sol | Tool calling par Responses ; fenêtre 1 050 000, sortie max 128 000 ; effort low/medium/high/xhigh/max |
| Audit Fable 5.2 | claude-fable-5-2 | Fenêtre 1M, sortie max 128K ; thinking adaptatif toujours actif ; pas tool_choice any/tool ; rétention spécifique 30 jours sauf autorisation contraire ; blocs de thinking liés au modèle producteur et au préfixe exact de l’appel (system, outils, messages antérieurs) : toute édition, suppression, compaction côté client ou resérialisation non identique d’un tour antérieur invalide les blocs postérieurs (la compaction côté serveur n’en est pas une), rejet 400 pour les comptes créés à partir du 31/08/2026 (documenté pour Fable 5.1, Opus 5.5, Sonnet 5.5 et Haiku 5.5 par la référence API Anthropic en cache du 06/10/2026) — à confirmer pour 5.2 |

Sources : [Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol), [Fable](https://platform.claude.com/docs/en/models/fable-5-2/overview). Les noms alias ne sont pas des snapshots datés inventés. Les limites effectives et droits sont revérifiés lors du paramétrage ; un profil recommandé n’est actif qu’après `TestProviderProfile` réussi (Q-07, [ADR-11](../architecture/adr/ADR-11-model-api-abstraction.md)). Le terme « ultra » rencontré dans un outil ou abonnement n’est pas ajouté aux valeurs API ci-dessus.

Au tarif standard documenté consulté, Sol affiche 2 USD/M tokens d’entrée et 10 USD/M de sortie ; Fable 8 et 40. Sol documente des majorations de contexte long et de modes de service. Ces valeurs servent seulement à illustrer l’intérêt d’un catalogue tarifaire versionné ; elles ne deviennent pas des constantes perpétuelles. Elles seront la première entrée sourcée et datée du catalogue de release `ModelCatalogSnapshot` (Q-14), revérifiée à chaque publication. Les quotas et taxes du compte ne sont pas vérifiés.

## Différences de protocole à conserver

| Sujet | OpenAI natif | Anthropic natif | Compatible OpenAI |
| --- | --- | --- | --- |
| Adaptateur V1 | Responses API pour Sol | Messages API | Profil déclaré Responses ou Chat Completions |
| Outils | Function calls identifiés, résultats rattachés au call | Blocs tool_use et tool_result | Formats/capacités testés, pas déduits du nom |
| Structure | Schéma JSON avec sous-ensemble supporté | output_config.format, strict tool use selon capacité | Peut manquer ou être seulement suggestif |
| Streaming | Événements propres au protocole | Événements/blocs propres au protocole | Parsing et limites spécifiques |
| Raisonnement | Paramètres par modèle | Thinking/effort par modèle | Aucun champ universel présumé |
| Mémoire | Identifiants de réponse/conversation documentés, continuation distante désactivée dans la V1 Unleash | Historique explicite et blocs natifs qualifiés selon protocole | Historique explicite ; aucune continuation distante présumée |
| Caching | Paramètres et comptabilité propres | Cache read/write et durées propres | Pas présumé |
| Refus/troncature | États à traiter avant parsing métier | Refusal/max_tokens peuvent invalider la structure attendue | Doit être qualifié |
| Usage/coût | Conserver champs bruts pertinents et normalisation | Idem, distinguer cache et tokens | Usage absent = inconnu, pas zéro |

Références : [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling), [sorties structurées OpenAI](https://developers.openai.com/api/docs/guides/structured-outputs), [état de conversation OpenAI](https://developers.openai.com/api/docs/guides/conversation-state), [caching OpenAI](https://developers.openai.com/api/docs/guides/prompt-caching), [outils Claude](https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview), [sorties structurées Claude](https://platform.claude.com/docs/en/build-with-claude/structured-outputs), [erreurs Claude](https://platform.claude.com/docs/en/api/errors), [caching Claude](https://platform.claude.com/docs/en/build-with-claude/prompt-caching).

## Backends locaux de premier ordre — R4

LM Studio, Ollama et llama.cpp, interprétation explicite du « CPP » demandé, sont des profils V1 de F012 distincts du profil OpenAI-compatible générique. Les [sources locales](local-model-sources.md) consignent les pages officielles et leurs limites.

| Backend | Différences documentées à conserver | Qualification actuelle |
| --- | --- | --- |
| LM Studio | API native et voies compatibles distinctes ; custom tools sur les voies compatibles qualifiées ; authentification configurable ; placement éventuellement distant via LM Link. | Documentation examinée, aucun serveur ou modèle configuré testé. |
| Ollama | API native et compatibilité partielle ; API locale pouvant relayer vers le cloud ; métadonnées et usage propres au dialecte. | Configuration locale/cloud réelle et inférence non testées. |
| llama.cpp | Build, modèle et template participent aux capacités ; clés optionnelles ; santé et inférence distinctes ; calcul déporté à prendre en compte. | Branche documentaire courante examinée, aucun build ou poids qualifié. |

Le contrat qualifie serveur/build, modèle, template, quantification, contexte et dialecte. SecretRef peut être absente pour une voie loopback qualifiée sans secret ; elle reste requise lorsque le backend l’exige. Les endpoints distants suivent authentification et TLS. Une liste de modèles et une réponse de santé ne démontrent ni capacités complètes ni placement local.

## Décisions d’architecture dérivées

Le contrat métier est validé localement même avec structured outputs. Un refus, une sortie incomplète ou un outil non appelé peut être un résultat valide du fournisseur et un échec du contrat de tâche. Les retries ne contournent ni un refus ni une politique de rétention. Un fallback modifie le fournisseur destinataire des données : il est soumis à l’autorisation du projet et à une nouvelle réservation budgétaire.

Une clôture est possible par réponse structurée validée sans forcer un tool call. Les outils côté fournisseur susceptibles d’avoir des effets de bord ne sont pas activés implicitement : ils contourneraient l’Action Broker local. Un usage de web search hébergé, par exemple, doit être exposé comme capacité distincte dont les traces et limites sont qualifiées.

Le contexte durable appartient à Unleash. La décision V1 est `explicit_unleash_history` : chaque appel fournit explicitement l'historique autorisé, sans previous_response_id, conversation ni équivalent de continuation serveur. Les éléments opaques nécessaires sont conservés et transmis explicitement sous leur filiation/politique qualifiées ; ils ne sont ni traduits en faux raisonnement ni transférés à un autre fournisseur. Une exclusion d'antécédent interdit leur réutilisation ; leur perte peut imposer reconstruction portable ou blocage si une pièce essentielle manque.

**Vérification documentaire R6 du contexte distant.** La [documentation officielle OpenAI sur l'état de conversation](https://developers.openai.com/api/docs/guides/conversation-state), relue le 16 novembre 2026, décrit le partage de contexte par `previous_response_id`, la facturation des entrées précédentes de la chaîne et une conservation distincte des objets Conversation. Elle montre aussi l'historique fourni manuellement avec `store=false`. Ces possibilités documentées ne constituent ni une activation dans Unleash, ni une promesse de rétention nulle. Les paramètres de stockage et la rétention réellement applicables restent qualifiés par endpoint/modèle/compte. Aucun appel de vérification réel n'a été réalisé.

Les résumés, extraits retrouvés, sorties d'outils et autres dérivés conservent les ACL, destinations, localité et obligations de conservation de leurs sources selon le [contrat de routage R6](../specs/model-routing.md). L'opacité, la compaction et une nouvelle invocation ne déclassifient pas ces données. Ces règles sont des décisions du produit, pas des capacités promises par les fournisseurs.

La politique `data_locality=local_only` refuse placement inconnu et fallback distant, y compris après OOM ou indisponibilité. `approved_remote` conserve une liste de destinations autorisées. La politique `model_policy=forbidden` interdit aussi les modèles locaux, embeddings, résumés et préchauffages du run. Une supervision par blocs utilise `allowed_bounded` et des checkpoints explicites, sans juge universel. Ces règles sont des décisions Unleash, détaillées dans [local-models](../specs/local-models.md) et [reusable-automations](../specs/reusable-automations.md), pas des garanties apportées automatiquement par un serveur.

## Vérification de qualification requise

Pour chaque couple fournisseur/modèle, et pour chaque tuple de configuration locale : requête texte minimale, streaming interrompu, outil correct/invalide, schéma pris en charge et non pris en charge, refus, limite de sortie, erreur 429 lorsque pertinente, timeout après envoi, reprise et comptage. Ajouter placement, file, chargement, OOM et annulation incertaine pour les backends locaux. Capturer profil, identifiants disponibles, statut, usage et absences. Les [tests F012](../features/F012-model-api-router/tests.md) et la [qualification locale](../testing/local-model-tests.md) sont prévus ; aucun essai réel n’a été exécuté pendant cet audit.

