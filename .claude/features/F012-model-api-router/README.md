# F012 — Routage API des modèles cloud et locaux

Statut : spécification proposée pour la V1 complète d’OVPT Unleash ; aucun composant applicatif n’est implémenté par ce dossier.

## Objectif et valeur

Appeler les modèles par API directe, rendre le choix effectif explicable et maintenir budgets, contexte et contrats indépendants des fournisseurs.

## Périmètre et frontières

La feature couvre OpenAI Responses, Anthropic Messages, **LM Studio, Ollama et le serveur llama.cpp** ainsi que les profils compatibles explicitement qualifiés. « CPP » est interprété comme llama.cpp dans cette révision ; cette interprétation reste signalée. Les trois backends locaux font partie de la V1. Configuration, authentification adaptée au backend, sélection, capacités, streaming, usage, erreurs et fallback utilisent le même routeur. Celui-ci ne porte pas les états métier d’une mission ni les secrets dans les prompts.

Les appels passent par les API HTTP des moteurs existants. Les clés utilisateur restent nécessaires pour les fournisseurs cloud qui les exigent ; un backend loopback qualifié peut fonctionner sans secret. Aucun abonnement CLI ni réimplémentation d’inférence n’est requis. Le premier parcours local utilise un endpoint géré par l’utilisateur, sans installer ni charger automatiquement un modèle lourd.

Une URL locale ne prouve pas une inférence locale. La politique `data_locality` vérifie la destination effective et interdit le fallback distant sous `local_only`. La politique d’automatisation `forbidden` interdit aussi les modèles locaux, résumés, embeddings et préchauffages liés au run. En mode supervisé autorisé, un modèle local peut intervenir aux checkpoints prévus, selon capacités et budgets.

Le code de cette feature et ses tests développés pour Unleash seront Rust. Les services externes et les outils des projets supervisés gardent leurs langages. Cette séparation respecte l’annexe A et ne dispense pas d’auditer les dépendances natives.

## Parcours de réalisation

Lire la [spécification canonique](../../specs/model-routing.md), le [contrat des modèles locaux](../../specs/local-models.md) et ses [sources primaires](../../research/local-model-sources.md), puis les [exigences](requirements.md), le [design technique](technical-design.md) et les [interfaces](interfaces.md). Le chantier démarre avec les dépendances et doubles décrits dans [dependencies](dependencies.md). Chaque comportement possède un [critère](acceptance.md) et un [test prévu](tests.md), complétés par la [qualification locale](../../testing/local-model-tests.md).

La feature est terminée seulement lorsque les contrats sont implémentés, les cas de panne passent et les preuves attendues existent. Un mock démontre un contrat local, pas l’accès réel à un fournisseur. Les points ouverts qui touchent un compte, une licence ou AI-Guard restent des gates explicitement visibles, sans report artificiel en V2.

