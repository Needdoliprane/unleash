# Modèles locaux — sources primaires et limites de qualification

**Consultation : 16 novembre 2026. Statut : recherche documentaire ; aucun moteur, modèle ou serveur local n’a été installé, lancé ou testé.** Les pages évoluent. Une version de documentation ou la branche `master` ne vaut pas un build qualifié d’Unleash. Les références ci-dessous étayent les faits externes ; les choix du produit sont arrêtés dans [local-models](../specs/local-models.md) (ADR-17 accepté et Q-19 tranchée en R7).

Louis demande LM Studio, Ollama et « CPP » dans la V1. **Interprétation conservée explicitement : « CPP » désigne ici le serveur HTTP de llama.cpp.** Il ne s’agit pas d’une confirmation que ce nom abrégé ne peut désigner aucun autre produit. Le contrat du routeur conserve des adaptateurs distincts ; cette interprétation n’impose aucune réécriture d’inférence en Rust.

## LM Studio

| Source | Fait documentaire vérifié | Conséquence proposée pour Unleash |
| --- | --- | --- |
| LOC-S01 — [OpenAI Compatibility Endpoints](https://lmstudio.ai/docs/developer/openai-compat) | Endpoints documentés : notamment modèles, Chat Completions, Responses et embeddings. | Déclarer et qualifier le dialecte exact ; ne pas déduire toutes les capacités d’un simple préfixe `/v1`. |
| LOC-S02 — [Native REST API](https://lmstudio.ai/docs/developer/rest) | API native `/api/v1/*` introduite avec LM Studio 0.4.0. La comparaison exclut les custom tools de `/api/v1/chat` et les annonce pour les voies compatibles Chat Completions et Responses. | Employer une voie compatible qualifiée pour les outils contrôlés par Unleash ; séparer les fonctions natives de gestion de modèles. |
| LOC-S03 — [Authentication](https://lmstudio.ai/docs/developer/core/authentication) | Tokens disponibles à partir de 0.4.0 ; l’authentification n’est pas activée par défaut. | Autoriser explicitement l’absence de token sur un profil loopback qualifié ; utiliser une référence de secret lorsque l’authentification est activée. |
| LOC-S04 — [List your models](https://lmstudio.ai/docs/developer/rest/list) | La liste native expose des informations de modèles, instances chargées, contexte et quantification ; les champs dépendent du type de modèle. | Collecter les métadonnées disponibles, conserver les absences et requalifier après changement de configuration. |
| LOC-S05 — [Tool Use](https://lmstudio.ai/docs/developer/openai-compat/tools) | Les modèles peuvent proposer des appels de fonctions via les endpoints compatibles. | Les propositions passent au broker Unleash après validation ; elles ne sont pas exécutées par le seul adaptateur de modèle. |
| LOC-S06 — [Using LM Link](https://lmstudio.ai/docs/developer/core/lmlink) | Des requêtes à `localhost` peuvent utiliser un modèle placé sur un appareil distant via LM Link. | Vérifier le placement effectif ; l’adresse loopback ne suffit pas à satisfaire `local_only`. |

Le modèle, son format et le serveur réellement configuré restent à qualifier. Une possibilité documentaire de tool use ne prouve pas que chaque modèle satisfera un contrat d’outil donné. L’usage de MCP ou d’outils hébergés par le serveur n’est pas déduit de sa simple connexion.

## Ollama

| Source | Fait documentaire vérifié | Conséquence proposée pour Unleash |
| --- | --- | --- |
| LOC-S07 — [Authentication](https://docs.ollama.com/api/authentication) | API locale sans authentification ; API cloud directe avec clé. Un serveur local connecté au compte peut relayer vers un modèle cloud. | Aucun faux secret obligatoire en local ; classifier la destination d’inférence indépendamment de l’URL. |
| LOC-S08 — [Local FAQ](https://docs.ollama.com/faq) | La FAQ documente `disable_ollama_cloud` et `OLLAMA_NO_CLOUD=1`, avec redémarrage après modification. Elle distingue capacités mémoire, chargement et parallélisme. | Vérifier la configuration effective et sa fraîcheur pour un profil strict ; ne pas déduire une capacité GPU du plafond d’agents. |
| LOC-S09 — [OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility) | Compatibilité partielle ; Chat Completions et Responses sont documentés. La description de Responses exclut l’état serveur via `previous_response_id` et `conversation`. | Qualifier les champs et conserver les sessions dans Unleash ; ne pas supposer la même mémoire distante que chez un autre fournisseur. |
| LOC-S10 — [Generate a chat message](https://docs.ollama.com/api/chat) | L’API native accepte historique, outils et format de sortie. Elle décrit `keep_alive`, durées de chargement/évaluation et compteurs de tokens. | Adapter ces champs seulement dans le dialecte natif qualifié ; séparer chargement, génération et métriques absentes. |
| LOC-S11 — [List models](https://docs.ollama.com/api/tags) et [Show model details](https://docs.ollama.com/api-reference/show-model-details) | La liste documente digest et quantification ; les détails documentent capacités, template et paramètres lorsque disponibles. | Enregistrer identité et configuration observées. Un alias ou un champ de capacité déclaré ne remplace pas une fixture de compatibilité. |

La page de compatibilité affiche aussi une liste de fonctionnalités dont le rendu texte peut perdre l’état des cases. La phrase explicite et les notes sur les paramètres stateful priment pour ce relevé. La qualification d’un build précis doit résoudre toute divergence ; elle ne peut activer la capacité seulement parce que le mot apparaît dans une liste.

Le réglage de désactivation cloud est une propriété d’un serveur configuré, pas une garantie fournie par la chaîne `localhost`. Le contrôle d’Unleash doit conserver ce qui a été vérifié et refuser le mode strict si le placement reste inconnu. Aucun réglage de serveur utilisateur n’a été modifié pendant cette recherche.

## llama.cpp

| Source | Fait documentaire vérifié | Conséquence proposée pour Unleash |
| --- | --- | --- |
| LOC-S12 — [HTTP Server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) | Serveur C/C++ ; routes compatibles Chat Completions, Responses et embeddings ; clés API optionnelles ; `/health` public ; options de calcul déporté RPC. | Qualifier build et dialecte ; une santé HTTP ne prouve ni authentification ni inférence sur la machine d’Unleash. |
| LOC-S13 — [Function calling](https://github.com/ggml-org/llama.cpp/blob/master/docs/function-calling.md) | L’usage des outils dépend des templates et handlers ; le parallélisme des appels n’est pas une propriété uniforme de tous les modèles. | Épingler template/configuration et tester les contrats de tool use requis avant admission. |

Les informations de `master` constituent une observation documentaire datée. Un build peut offrir un sous-ensemble différent ; aucun numéro de release ni binaire opérationnel n’est inventé. Les capacités supplémentaires mentionnées par le serveur ne sont pas automatiquement exposées dans Unleash.

## Confirmé, décidé, à confirmer

**Confirmé par les sources :** existence des voies HTTP et des distinctions ci-dessus. **Décidé par le produit (R7) :** profils natifs `lmstudio`, `ollama`, `llama_cpp`, `data_locality = local_only | approved_remote`, exception HTTP limitée au loopback qualifié, admission de ressources locale (`max_inflight = 1` par serveur par défaut) qui s’ajoute au plafond global 5/50 sans le remplacer ; matrice de référence Q-19 : LM Studio, Ollama et llama.cpp aux versions figées au gel M2, modèle texte Qwen3-8B instruct GGUF Q4_K_M et un modèle d’embedding, tout tuple non éprouvé restant « non qualifié ». **À confirmer sur chaque cible :** version/build, modèle et licence, contexte effectif, quantification, capacités, placement, configuration d’authentification, mémoire, annulation et performance.

La présence de ces backends dans la V1 est une exigence de périmètre. Elle n’équivaut pas à leur qualification effective. Les scénarios et preuves attendues sont dans [local-model-tests](../testing/local-model-tests.md) et [F012](../features/F012-model-api-router/README.md).

## Sources des modèles de référence Q-19 avant gel M2

**Consultation documentaire du 22 novembre 2026.** Les dépôts des éditeurs publient les fichiers suivants. Les révisions Git identifient les états consultés. Les tailles et les SHA-256 proviennent des métadonnées LFS publiées pour ces fichiers. Le [journal M2](../state/goals/g0003-m2/journal.md#reprise-de-la-fixture-ollama) conserve la mesure du fichier Nomic et l’identité de son import Ollama.

| Référence | Révision Git du dépôt | Fichier GGUF Q4_K_M | Octets | SHA-256 LFS déclaré | Licence déclarée |
| --- | --- | --- | ---: | --- | --- |
| Texte, [fiche Qwen](https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/7c41481f57cb95916b40956ab2f0b139b296d974/README.md) | `7c41481f57cb95916b40956ab2f0b139b296d974` | [Qwen3-8B-Q4_K_M.gguf](https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/7c41481f57cb95916b40956ab2f0b139b296d974/Qwen3-8B-Q4_K_M.gguf) | 5 027 783 488 | `d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785` | [Apache-2.0](https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/7c41481f57cb95916b40956ab2f0b139b296d974/LICENSE) |
| Embedding, [fiche Nomic](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/blob/0188c9bf409793f810680a5a431e7b899c46104c/README.md) | `0188c9bf409793f810680a5a431e7b899c46104c` | [nomic-embed-text-v1.5.Q4_K_M.gguf](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/blob/0188c9bf409793f810680a5a431e7b899c46104c/nomic-embed-text-v1.5.Q4_K_M.gguf) | 84 106 624 | `d4e388894e09cf3816e8b0896d81d265b55e7a9fff9ab03fe8bf4ef5e11295ac` | Apache-2.0, déclaré dans la fiche |

Les tailles exactes, les SHA-256 LFS et les licences figurent aussi dans les métadonnées JSON des révisions. [Qwen](https://huggingface.co/api/models/Qwen/Qwen3-8B-GGUF/revision/7c41481f57cb95916b40956ab2f0b139b296d974?blobs=true), [Nomic](https://huggingface.co/api/models/nomic-ai/nomic-embed-text-v1.5-GGUF/revision/0188c9bf409793f810680a5a431e7b899c46104c?blobs=true).

La référence de format décrit la structure binaire des modèles GGML. La version du serveur est à relever sur le binaire utilisé. [Spécification GGUF](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md).

Lors de la qualification, comparer l’empreinte du fichier chargé par LM Studio et llama.cpp à celle publiée. Relever le digest propre au modèle exposé par Ollama dans son inventaire. La qualification des tuples et de leurs capacités reste ouverte.
