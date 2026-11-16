# Modèles locaux — plan de qualification F012

**Date : 16 novembre 2026. Statut : tests futurs, non exécutés.** Ce document complète [F012/tests](../features/F012-model-api-router/tests.md), le [contrat local](../specs/local-models.md) et les [sources primaires](../research/local-model-sources.md). Aucun runtime, poids, pilote GPU ou processus d’inférence n’a été installé ou démarré pour écrire ce plan.

## Principe de preuve

Un double valide le comportement du routeur ; il ne démontre pas qu’un modèle réel comprend les outils ni que ses ressources suffisent. Une qualification réelle fixe backend/version/build, modèle et provenance/licence, digest disponible, template, quantification, contexte, système, matériel pertinent et configuration de routage. Une donnée absente est explicitement absente. La même marque avec un autre modèle ne reçoit pas automatiquement les mêmes capacités.

Les tests sont bornés en durée, tokens, mémoire lorsque mesurable et coût. Les endpoints de test sont explicitement configurés ; aucune découverte réseau large n’est nécessaire. Les données sont synthétiques. Les captures d’erreur, demandes et résultats sont masquées avant export ; un canari secret permet de vérifier l’absence de fuite dans les logs.

## Matrice de qualification par backend

| Cible | Variantes nécessaires | Ce qu’il faut conserver |
| --- | --- | --- |
| LM Studio | Mode loopback sans token, mode avec token ; dialecte compatible sélectionné ; modèle aux outils qualifiés ; placement local et cas LM Link distant/non vérifiable. | Version serveur, identifiant/instance, contexte, quantification disponible, preuve de placement et résultat par capacité. |
| Ollama | API native qualifiée et profil compatible distinct si livré ; local sans secret ; modèle local ; cas cloud via serveur loopback et cloud désactivé. | Configuration effective, modèle/digest, template et capacités disponibles, observations de destination et usage. |
| llama.cpp | Build fixé, poids et template fixés, mode sans/avec API key ; capacité d’outil ; cas sans et avec déport/proxy non autorisé. | Build, configuration, métadonnées disponibles, authentification par route, placement et résultats des fixtures. |

L’interprétation « CPP = llama.cpp » est conservée dans le dossier. Aucun test d’un autre produit nommé CPP n’est prétendu couvert.

## Scénarios rattachés aux six nouvelles chaînes

| Test | Variantes et faute injectée | Oracle | Preuve |
| --- | --- | --- | --- |
| T-F012-013 | Configurer chaque backend ; absence de token locale autorisée ; token invalide ; HTTP non loopback ; redirection vers autre origine ; secrets canaris. | Requêtes adressées au seul endpoint autorisé, protocole conforme, refus des cas interdits avant envoi des données, secret absent des sorties publiques. Santé seule insuffisante. | EV-F012-013 : profils masqués, destinations captées, réponses d’authentification, refus et versions. |
| T-F012-014 | Texte, stream fragmenté, outil complet/incomplet, schéma invalide, contexte trop grand ; champ accepté mais ignoré ; changement de template/alias. | Capacités requises réellement exercées ; aucun outil sur fragment, refus des capacités manquantes, snapshot périmé après changement. | EV-F012-014 : fixtures, tuple qualifié, sorties structurées, traces de broker et diagnostics. |
| T-F012-015 | `local_only` avec modèle local ; Ollama cloud derrière localhost ; LM Link distant ; proxy inconnu ; cible LAN ; fallback cloud après panne locale ; cas distant explicitement autorisé. | Zéro transfert aux destinations interdites. Placement non vérifié refusé avant contexte. Le cas autorisé crée un appel/manifeste distinct avec sa cause. | EV-F012-015 : politique, preuves de placement, capture des destinations et canaris, refus, manifests de fallback. |
| T-F012-016 | Serveur froid, chaud, file pleine, chargement lent, mémoire insuffisante, arrêt pendant stream, annulation sans confirmation d’arrêt, deux clients partageant la ressource. | Plafonds global et local respectés, délais séparés, file durable, état inconnu conservé, aucune relance qui suppose une capacité libérée. | EV-F012-016 : transitions, admissions, réservations, mesures disponibles et raisons d’absence. |
| T-F012-017 | Métadonnées complètes puis digest/build/usage absent ; alias qui change ; redémarrage entre préparation et dispatch ; coût non facturé avec métriques machine non mesurées. | Manifest immuable et honnête, champs absents avec raison, aucune identité de poids inventée, coût/tokens inconnus distincts de zéro. | EV-F012-017 : manifest, payload masqué, comparaison de révisions, usage observé/estimé et provenance. |
| T-F012-018 | Même configuration depuis GUI/TUI ; sélection par rôle/skill/workflow ; test manuel borné ; run `forbidden` tentant appel local, résumé, embedding ou préchauffage ; run supervisé autorisé à un checkpoint. | Parité des commandes et permissions. Aucune requête de génération, de chargement ou de démarrage pour le run interdit. Checkpoint autorisé avec budget et trace. | EV-F012-018 : reçus communs, capture des appels et processus, inspections GUI/TUI, trace du checkpoint autorisé. |

Ces chaînes complètent REQ/AC/T/EV 001–012 ; elles ne les remplacent pas. Les assertions de sécurité utilisent les effets captés, pas seulement l’absence d’une ligne dans un journal applicatif.

## Protocole d’essai proposé

1. Identifier le backend, son propriétaire, l’environnement de test et les contraintes de données. Conserver la configuration initiale et la politique applicable.
2. Exécuter les doubles Rust de sélection, politiques, transport, streaming, budgets et reprise, avec horloge contrôlée et pannes reproductibles. Un échec empêche la qualification correspondante.
3. Connecter un serveur réel déjà préparé pour cet essai ; vérifier la destination et l’authentification par la route prévue. Ne pas confondre inventaire, santé, chargement et génération.
4. Réaliser un petit probe explicite, puis les fixtures de capacités effectivement nécessaires. Relever chaque sortie et refus sans transformer une réussite ponctuelle en garantie générale de qualité.
5. Réaliser les cas de localité, ressources et coupure dans un environnement contrôlé. Les fautes susceptibles de saturer une machine sont simulées d’abord ; un vrai test de charge reste borné et distinct.
6. Rejouer les opérations de configuration et d’inspection en GUI/TUI sur le même backend. Comparer commandes, révisions, reçus et erreurs, puis sauvegarder le paquet de preuves autorisé.

Le lancement d’un moteur ou le téléchargement de poids n’est pas une étape tacite de ce plan. Lorsque nécessaire à la recette, ces opérations doivent être prévues avec leur taille, provenance, ressources et mandat. La présente rédaction n’a effectué aucune de ces opérations.

## Matrice de référence R7

Décision Q-19 ([révision R7](../brief/revision-2026-11-16-r7.md), [ADR-17](../architecture/adr/ADR-17-local-model-providers.md)). Cette matrice qualifie des tuples précis ; elle ne restreint pas les modèles que l'utilisateur peut configurer. Un tuple non éprouvé reste « non qualifié » et ses capacités absentes restent indisponibles.

| Axe | Valeurs de référence |
| --- | --- |
| Backends et dialectes | LM Studio ≥ 0.4.0 : inférence Chat Completions compatible, inventaire par l'API native. Ollama : release stable au gel M2 documentant la désactivation du relais cloud ; `/api/chat`, inventaire `/api/tags` et `/api/show`. llama.cpp : `llama-server` au build figé au gel M2, templates Jinja activés, Chat Completions compatible. Versions exactes consignées au gel M2. |
| Modèles | M1 = Qwen3-8B instruct, GGUF Q4_K_M : texte, streaming, appel d'outil unique, sortie JSON contrainte, limite de contexte et annulation. M2 = nomic-embed-text v1.5 : embeddings et contre-tests `forbidden`. LM Studio et llama.cpp chargent le même fichier GGUF (digest consigné) ; le digest Ollama est consigné. |
| Plateformes | H1 = Mac Apple Silicon, 16 Go de mémoire unifiée (bloquant). H2 = Linux x86_64, CPU seul, 16 Go (bloquant, Ubuntu retenu par Q-11). Aucun GPU dédié n'est requis ni promis ; un GPU NVIDIA relève d'une qualification étendue non bloquante. |
| Configuration figée | Contexte configuré à 8 192 tokens ; sortie de fixture ≤ 1 024 tokens ; `max_inflight=1` par serveur ; deadlines connexion loopback 2 s, chargement à froid 180 s, premier token 60 s, génération totale 300 s ; modes d'authentification `none` et `token` lorsque le backend les offre. |
| Cases obligatoires | 3 backends × M1 × H1, plus H2 ; M2 sur chaque backend en H1 ; contre-tests T-F012-013 à 018 : proxy loopback vers distant, relais Ollama cloud activé puis désactivé, LM Link distant (`blocked` motivé si indisponible), OOM simulé puis réel borné, alias changé, annulation incertaine, appel interdit sous `forbidden`. |

Les sources primaires datées des modèles M1/M2 (fiche, licence, format) sont ajoutées au [registre local](../research/local-model-sources.md) au gel M2. Une mesure justifiant `max_inflight>1` est une preuve par tuple, jamais une extrapolation.

## Critères de sortie

Chaque profil livré possède une matrice de capacités réussies, refusées ou non qualifiées, liée aux preuves et à la configuration exacte. Les trois backends V1 doivent chacun avoir leur preuve réelle applicable ; trois doubles ne constituent pas trois intégrations validées. Les fonctionnalités optionnelles non testées restent indisponibles plutôt qu’annoncées implicitement.

La localité stricte doit être refusée lorsque la preuve de placement manque ou devient périmée. Un fallback interdit, une reprise répétant un effet ou un appel caché dans `forbidden` est bloquant. Les erreurs de configuration et d’authentification sont également testées en TUI ; aucun passage par la GUI Unleash ne doit être obligatoire pour les corriger.

Un rapport de qualification indique commandes réellement exécutées, exit codes, version du produit, dates, durées, limites et incidents. Une référence EV est une preuve attendue tant que ces artefacts n’existent pas. Ce document ne contient aucun résultat de benchmark ou succès d’inférence supposé.
