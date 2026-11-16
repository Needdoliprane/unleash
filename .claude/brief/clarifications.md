# Clarifications utilisateur — R2 à R5 et R7, état consolidé R7

Date : 16 novembre 2026. Les précisions suivantes complètent le brief principal et l’annexe A–M. La dernière clarification utilisateur fait autorité lorsqu’elle corrige une formulation antérieure.

1. **Identité : OVPT Unleash**, produit OVPT destiné à l’open source. La commande technique peut rester `unleash`. La licence n’a pas été choisie implicitement : elle a été arbitrée explicitement en R7 (Apache-2.0, Q-06).
2. **Rust avec discernement.** Le cœur métier, le runtime et les mécanismes système développés pour Unleash restent en Rust. Les bibliothèques existantes, leurs dépendances natives, les moteurs de stockage ou de rendu, HTML et CSS sont autorisés. Il n’est pas demandé de les réimplémenter. La [politique d’implémentation](implementation-policy.md) définit la frontière entre cœur, présentation, outillage et projets supervisés.
3. **Kivy et Tauri sont les références identifiées.** La correction explicite résout l’ancienne question Q-17 sur « kavi ou st ». Elle ne sélectionne pas définitivement un framework. Kivy est un framework Python ; Tauri associe un hôte Rust à une présentation dans une WebView. La [comparaison sourcée](../research/technology-comparison.md) motive Tauri comme candidat de référence proposé, avec alternatives et qualification à réaliser.
4. **GUI et terminal doivent être beaux et utilisables.** Typographie, densité, navigation, lisibilité des états, raccourcis, sélection, recherche et retour d’erreur sont des critères de recette, pas une conséquence supposée du nom de la bibliothèque.
5. **Parité fonctionnelle complète GUI/TUI.** « CLI iso à la GUI » signifie ici une application terminal interactive complète. Les dix espaces, éditeurs, conversations, interventions, réglages, connecteurs et inspecteurs sont disponibles dans la TUI. La CLI non interactive offre en plus un accès scriptable aux mêmes opérations métier. La [matrice de parité](../design/gui-tui-parity.md) formalise la vérification ; le rendu est adapté au terminal sans retrancher une capacité.
6. **Bibliothèque terminal éprouvée.** ncurses est une référence de niveau fonctionnel, pas une dépendance imposée. Ratatui avec Crossterm est la recommandation de conception à qualifier pour cette TUI Rust. Les limites des terminaux et de l’accessibilité sont testées, avec présentation textuelle équivalente et modes dégradés explicites.

La mention de MySQL illustre la réutilisation d’une bibliothèque existante. Elle ne demande pas de changer le choix SQLite du dossier. Le langage des repositories développés par les agents demeure libre selon l’annexe A.

## Décisions et hypothèses

La règle pragmatique de langage et la parité fonctionnelle sont **confirmées**. Tauri, son moteur frontend précis et Ratatui/Crossterm sont **proposés**, avec sources officielles et essais de qualification. Une fine couche de présentation JavaScript/TypeScript peut être proposée si elle apporte un bénéfice documenté ; elle ne déplace ni les règles métier ni les permissions hors du cœur Rust. Rust/WASM est comparé selon son coût réel, sans devenir une obligation nouvelle.

Les audits initiaux restent l’historique de l’analyse sous l’ancien brief ; leurs conclusions linguistiques sont explicitement supersédées par R2. [Historique et propagation](revision-2026-11-16-r2.md), [ADR interfaces](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md).

## Clarifications suivantes — R3, R4 et R5

R3 étend les usages : Unleash reste orienté développement, mais une personne peut automatiser une procédure de navigateur ou de documents avec des données variables. L’apprentissage peut utiliser un LLM ; une procédure publiée peut fonctionner sans modèle, avec réanalyse si ses conditions changent. R4 ajoute explicitement LM Studio, Ollama et modèles locaux équivalents ; « CPP » est interprété comme llama.cpp, sans confondre moteur, serveur et modèle. R5 autorise aussi une supervision LLM par blocs de script/résultats/logs et demande un parcours guidé avec procédure par défaut, même sans agent préconfiguré.

Ces demandes sont cumulatives : replay sans modèle et exécution supervisée sont deux modes visibles, pas deux interprétations concurrentes. Les modèles locaux ne rendent pas un appel LLM déterministe et ne comptent pas comme « aucun modèle ». Les [arbitrages et changements R3–R5](revision-2026-11-16-r3-r5.md) font autorité pour cette extension. Les politiques et templates proposés ne créent pas de juge qualité universel obligatoire.

## Clarifications R7

Louis demande, le 16 novembre 2026 : un audit complet du dossier avec correction des incohérences, problèmes et bugs ; la résolution des arbitrages en attente ; une V1 commercialisable, le produit restant open source ; l’orchestration des agents de développement par un `/goal` dont l’agent root gère le travail en autonomie et choisit le modèle adapté à chaque tâche ; des worktrees de développement uniquement dans `.claude/`, nulle part ailleurs ; des règles et bonnes pratiques de code Rust ; les fichiers nécessaires pour que des agents autonomes réalisent le produit depuis Codex, Claude Code ou Antigravity. Il précise que l’objectif est surtout de corriger et d’améliorer.

Ces demandes sont confirmées. Les arbitrages sont rendus par délégation : ils deviennent des décisions **acceptées**, révisables, et non des invariants. Les worktrees de développement sont placés sous `.claude/worktrees/`, emplacement natif de Claude Code ; ceux que le produit crée pour les projets qu’il supervise restent régis par F010. Le détail, les décisions et l’outillage sont dans la [révision R7](revision-2026-11-16-r7.md).

Précisions complémentaires du même jour : les agents gèrent branches, commits et fusions et peuvent pousser la branche `dev` sur GitHub ; `main` reste réservée aux releases décidées par Louis. Louis fournit des clés API de développement (Anthropic et OpenAI), conservées dans `.env` ignoré par Git. Pendant le développement, les tests payants n'utilisent que des modèles faibles (Haiku, Luna) ; aucun test n'utilise un modèle intelligent (Sol, Fable…). La qualification des profils recommandés avec ces modèles reste une campagne future décidée par Louis (EXT-MODEL). Les autres pièces à secret (signature Apple, OAuth, comptes documentaires, AI-Guard) sont hors de la cible immédiate et traitées par des doubles jusqu'à leur fourniture.

