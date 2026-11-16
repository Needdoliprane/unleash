# ADR-17 — Fournisseurs de modèles locaux dans le Model Router

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : dialectes, concurrence et délais par défaut, matrice locale qualifiée (Q-19) ; voir la section « Amendement R7 ». Le support local reste confirmé par R4. Date initiale : 16 novembre 2026. Références : INV-003/004/005/051/063, H-14/20, R-28/29 ; Q-19 ; F003/F011/F012/F014/F015/F019. « CPP » est interprété comme llama.cpp ; aucun identifiant de modèle n’est déduit de cette expression.

## Contexte

La contrainte initiale d’accès par API élimine les sessions d’abonnements CLI. Elle n’exige pas que l’inférence soit distante. Louis demande LM Studio, Ollama et un moteur de la famille llama.cpp. Le système doit les rendre configurables par les mêmes portées que les autres fournisseurs et conserver sa propre mémoire durable.

Un endpoint local ne garantit ni les capacités du modèle, ni le placement local du calcul, ni l’absence de coûts machine. Des produits peuvent acheminer la requête vers une autre machine ou un cloud. Une génération de texte réussie ne qualifie pas les outils, le JSON contraint, les images ou l’annulation.

## Options

| Option | Avantages | Inconvénients et risques | Complexité / effet sur V1 |
| --- | --- | --- | --- |
| Serveurs existants par API, profils natifs explicites | Réutilise moteurs, installation et gestion de poids ; même Router | Variations de dialecte, versions et ressources à qualifier | Retenue, sans service distribué obligatoire |
| Endpoint OpenAI-compatible générique uniquement | Adaptateur HTTP réduit | Perd diagnostics natifs, confond options acceptées/ignorées et localité | Conservé comme extension ; insuffisant seul pour les trois intégrations natives |
| Inference embarquée dans le daemon | Distribution éventuellement unifiée | GPU/ABI/poids/licences/build couplés au core, mémoire et crashes partagés | Aucun besoin démontré pour V1 ; alternative future sur cas mesuré |
| Wrapper CLI de moteur ou abonnement assistant | Certaines fonctions accessibles rapidement | Protocoles texte instables, reprise et auth ambiguës ; abonnement contraire à l’invariant | Pas la voie d’inférence retenue |
| Serveur local géré via pool de processus | Démarrage/arrêt inspectables sous mandat | Gestion de propriété et partage avec autres applications | Option explicite via F011, pas installation ou téléchargement automatique |

## Décision

Les profils `lmstudio`, `ollama` et `llama_cpp` sont de première classe. Le backend Rust négocie leur dialecte, auth, modèle, capacités et métadonnées disponibles. Le profil compatible générique reste distinct. Les appels OpenAI et Anthropic natifs restent inclus ; les contraintes de choix s’appliquent à tous.

Le serveur de référence est fourni/configuré par l’utilisateur. Unleash peut tester connexion et inventaire sans lancer silencieusement une génération. Le probe de capacité est une opération visible, bornée et autorisée ; une interface ne charge pas plusieurs gigaoctets de poids à sa simple ouverture. Un serveur géré requiert un mandat de processus et ne peut pas être tué pour annuler le travail d’un autre client.

SecretRef est facultative si le backend n’exige pas d’authentification. Un secret réel reste dans le coffre du système, jamais dans une URL ou un log. L’exception HTTP loopback est encadrée par la frontière locale ; une destination distante exige le transport et l’identité appropriés. Les redirections, proxys et options de calcul distant sont contrôlés selon la politique.

## Amendement R7

1. Les mécanismes sont adoptés : profils de première classe `lmstudio`, `ollama` et `llama_cpp`, profil compatible générique distinct ; serveur fourni et configuré par l’utilisateur par défaut ; serveur géré via F011 seulement sur mandat, sans téléchargement de poids ; SecretRef facultative ; exception HTTP limitée au loopback qualifié ; localité orthogonale à `model_policy` ; aucun repli cloud silencieux ; manifests immuables avant dispatch.
2. Dialectes : LM Studio en Chat Completions compatible, avec inventaire par son API native ; Ollama par son API native `/api/chat`, avec `/api/tags` et `/api/show` ; llama.cpp en Chat Completions compatible, templates Jinja activés.
3. Défauts V1 : `max_inflight=1` par serveur ; délais de connexion loopback 2 s, de chargement à froid 180 s, de premier token 60 s et de génération 300 s.
4. Matrice qualifiée V1 (Q-19) : LM Studio, Ollama et llama.cpp aux versions figées au gel M2 ; modèle texte de référence Qwen3-8B instruct GGUF Q4_K_M et un modèle d’embedding. Chaque tuple non éprouvé reste « non qualifié » ([modèles locaux](../../specs/local-models.md)).

## Localité, compatibilité et repli

`model_policy=forbidden` interdit toute inférence locale et distante attribuable au Run, y compris embeddings et travaux indirects. Ce choix est orthogonal à `local_only`, qui autorise l’inférence mais contraint son placement. Le profil expose le placement attesté et son niveau de vérification ; une destination inconnue ne passe pas pour locale.

LM Link permet à LM Studio de servir une requête localhost sur une autre machine ; Ollama peut exposer un modèle cloud via son API locale. Le serveur llama.cpp possède également des possibilités de calcul déporté selon sa configuration. Le mode strict vérifie la configuration effective et la frontière réseau, ou refuse de revendiquer local_only. Un modèle local indisponible ne déclenche pas un repli cloud silencieux. Un repli doit être explicitement permis, compatible avec les données et inscrit dans le manifest du nouvel appel.

Le snapshot de capacités dépend du tuple serveur/build, modèle, métadonnées disponibles, template, quantification, dialecte et limites configurées. Le modèle peut ne pas fournir de digest exploitable ; la valeur reste inconnue. Les informations découvertes dans la réponse alimentent ModelCallRecord et ne réécrivent pas rétroactivement le manifest figé avant dispatch.

## Ressources et erreurs

Le pool global partagé reste cinq unités d’exécution actives par défaut, cinquante maximum. Cela ne promet pas cinquante générations GPU. Un appel concurrent par serveur local (`max_inflight=1`) est le défaut V1, relevable seulement sur preuve consignée par tuple (amendement R7). Temps de chargement, délai au premier token, génération totale, contexte et mémoire sont distincts.

OOM, modèle manquant, capacité absente et serveur inaccessible sont des erreurs explicites. Une annulation de transport ne prouve pas l’arrêt physique de l’inférence. Les réservations ne sont libérées qu’en fonction de l’observation disponible et de la politique de récupération. Aucune erreur ne déclenche un téléchargement de poids ou une réduction silencieuse du contexte.

Les métriques distinguent usage mesuré, fourni, estimé et inconnu. Zéro facturation fournisseur n’est pas assimilé à zéro coût machine. GUI et TUI exposent le même profil, ses paramètres, sa localité, ses capacités et ses tests.

## Acceptation et conséquences

REQ-MODEL-013 à 018 et T-E2E-Y doivent qualifier les trois backends avec versions et modèles réels documentés. Les contre-tests comprennent proxy local vers distant, option acceptée mais ignorée, changement d’alias, OOM, annulation inconnue et appel d’embedding sous forbidden. Les tests restent attendus ; aucun runtime ou poids n’a été installé pendant la conception.

L’effort principal est dans la qualification, les diagnostics et les ressources, pas dans une nouvelle abstraction de session. L’intégration locale réutilise les manifests, les autorisations, les contrats, les budgets et le contexte existants. Un autre moteur devient un profil supplémentaire sans modifier l’orchestrateur.

## Sources primaires

[LM Studio — compatibilité](https://lmstudio.ai/docs/developer/openai-compat), [authentification](https://lmstudio.ai/docs/developer/core/authentication), [LM Link](https://lmstudio.ai/docs/developer/core/lmlink), [Ollama — authentification](https://docs.ollama.com/api/authentication), [Ollama — FAQ et désactivation cloud](https://docs.ollama.com/faq), [llama.cpp — serveur](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md), consultés le 16 novembre 2026. Les versions de documentation ne sont pas des dépendances installées ; la recherche détaillée et ses limites sont dans [local-model-sources](../../research/local-model-sources.md).
