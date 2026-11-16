# F006 — Compilateur de workflows

## Objectif et parcours

OVPT Unleash permet de décrire le travail attendu en langage naturel puis de disposer d'une définition structurée, contrôlée et versionnée. Cette feature couvre la génération de draft, les diagnostics, la normalisation, la simulation et la publication. Le résultat est utilisable par le moteur F005 sans devoir relire la prose utilisateur pour choisir les transitions.

Le parcours commence par intention, contexte autorisé, done et contraintes. Un modèle peut proposer le draft et signaler ses hypothèses. Le compilateur Rust vérifie ensuite la structure, les références, les types, les bornes et les permissions. Le diff de normalisation explique chaque valeur ajoutée. Une publication crée une version immuable ; elle ne démarre pas automatiquement un run et ne modifie aucune exécution existante.

## Périmètre V1

Le DSL comprend séquence, tâche, parallèle, choix, répétition bornée et échec explicite, ainsi que la feuille action (profil v1.1) et for_each borné (profil v1.2). Les guards sont structurés et typés. Les scripts, évaluations de code et expressions réseau n'en font pas partie. Le compilateur traite les slots de rôle fixes et tardifs, contrôle leurs contrats et plafonds de capacités, et produit une borne supérieure d'activations. Les tests ou audits sont présents seulement si le workflow les demande.

Le simulateur affiche des chemins de succès, correction, blocage, annulation et épuisement. Ses événements sont des fixtures, clairement marqués comme simulés. La GUI peut rendre le graphe et les diagnostics ; CLI/TUI/agents utilisent le même service. Une ressource externe ou modèle indisponible produit une information précise, pas un nom d'API inventé.

## Limites

Le compilateur ne peut pas prouver qu'une phrase ambiguë a une seule intention ni garantir la qualité d'un résultat LLM. Il peut démontrer des propriétés structurelles de son langage restreint et refuser ce qui sort du profil. Une compilation réussie n'est pas un test de l'application produite. La migration d'un run actif est écartée de la V1 (Q-03) et ne peut pas être simulée par la publication d'une nouvelle version.

Références canoniques : [spécification du compilateur](../../specs/workflow-compiler.md), [DSL](../../architecture/workflow-dsl.md), schémas [v1](../../schemas/workflow.schema.json), [v1.1](../../schemas/workflow-v1.1.schema.json) et [v1.2](../../schemas/workflow-v1.2.schema.json), ainsi que l'[audit](../../audits/workflow-determinism.md). Tous les comportements sont proposés pour une future implémentation Rust. Les documents de ce dossier précisent les interfaces que plusieurs équipes peuvent développer indépendamment.
