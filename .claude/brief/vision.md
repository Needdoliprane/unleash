# Vision — Unleash, produit OVPT

Statut : vision confirmée par le brief principal et son annexe A–M, reçus le 16 novembre 2026. Architecture technique proposée, dont les décisions structurantes sont acceptées depuis la [révision R7](revision-2026-11-16-r7.md) ; aucun dépôt applicatif existant n’a été fourni ou découvert dans le répertoire de travail désigné. Le présent dossier décrit la V1 à construire ; sa réalisation commence au jalon M0, dans ce même dépôt.

Unleash reste spécialisé dans le développement logiciel et coordonne aussi des procédures professionnelles ou personnelles réutilisables. Un développeur, un comptable ou un particulier utilisent les mêmes mécanismes de missions, données, autorisations et preuves, avec une présentation adaptée. L’utilisateur confie un objectif, choisit ses procédures et peut intervenir. Le produit conserve les missions, les décisions, les effets observés et les preuves, même lorsque ses fenêtres sont fermées ou qu’un worker disparaît. Le modèle décide dans son mandat ; le moteur possède les états et les transitions.

La valeur recherchée est la continuité d’un travail vérifiable : savoir ce qui est engagé, ce qui existe réellement, ce qui attend un arbitrage, ce qui coûte, et pourquoi une mission est déclarée terminée. Une multiplication d’agents ou une interface animée ne constitue pas un indicateur de succès.

## Propriétés revendiquées et limites

| Propriété | Sens vérifiable | Limite explicite |
| --- | --- | --- |
| Orchestration déterministe | Même état, même commande validée, même révision de définition et mêmes faits enregistrés produisent la même décision de transition | Le texte du LLM, le réseau et les programmes externes ne deviennent pas déterministes |
| Sessions durables | Une incarnation suivante retrouve l’historique engagé et un contexte reconstruit | Pas de sauvegarde de RAM du modèle ni du worker |
| Capitalisation | Recherche préalable traçable et index reconstruits sur snapshots identifiés | Ni équivalence sémantique universelle ni index instantanément à jour |
| Autonomie | Staffing et décisions techniques permis par les mandats | Les budgets, permissions et états ne sont jamais modifiables par simple texte |
| Local-first | État et orchestration locaux, service utilisateur | Les appels modèles et documents privés utilisent les services distants autorisés |
| Open source | Intention explicite OVPT de distribuer le produit sous licence open source | Licence Apache-2.0 décidée (Q-06, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md)) ; les noms OVPT et Unleash ne sont pas concédés par la licence (politique de marque, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md)) ; recherche d’antériorité du nom avant la première publication publique |

## Public et périmètre

Le public comprend développeurs et responsables de projets, mais aussi des personnes qui souhaitent automatiser une tâche quotidienne sans configurer des agents ou connaître un workflow. Le parcours guidé propose une procédure adaptée, explique sources et effets, puis prépare les rôles nécessaires selon le mandat. Les réglages experts restent accessibles ; ils ne sont pas un prérequis pour commencer. Les projets cibles peuvent être Python, TypeScript, Java, Go, Rust ou multilangages. Le cœur applicatif et système d’Unleash est développé en Rust ; les bibliothèques existantes et la présentation HTML/CSS sont autorisées selon la [politique précisée par Louis](implementation-policy.md). L’exécution d’un compilateur Java ou d’une suite de tests Python appartient au projet cible, conformément à l’annexe A.

L’utilisateur choisit une GUI soignée ou une TUI plein écran avec les mêmes fonctions, états, autorisations et preuves. Les éditeurs et réglages avancés sont inclus dans cette parité ; seule leur présentation et leur navigation s’adaptent au terminal. Une CLI non interactive complète ces deux interfaces pour l’automatisation.

Dans ce livrable, `.claude/` désigne le dossier de conception, référence documentaire de la réalisation. Ce nom coïncide avec le répertoire de configuration de projet de Claude Code, qui y trouve depuis R7 l’outillage optionnel des agents de développement ; leur coexistence suit la [révision R7](revision-2026-11-16-r7.md) et le [contrat d’ingestion](../specs/document-package-ingestion.md) : `state/`, `worktrees/` et `settings.local.json` sont exclus du manifest. Le code de la réalisation vit à la racine du même dépôt (`crates/`, `apps/gui-renderer/`, `spikes/`, `qualification/`), jamais dans `.claude/`. Le produit ne lit jamais `.claude/` à l’exécution ; ni lui, ni les experts documentaires, ni les fichiers de référence ne requièrent Claude Code. L’utilisation des API OpenAI et Anthropic n’utilise aucun abonnement CLI.

## Épreuve de développement sur lui-même

Unleash pourra ultérieurement prendre ce dossier comme contexte d’un goal, sélectionner les chantiers, exécuter des workflows et lier les preuves aux exigences. Ce test intervient après une base indépendante de récupération et d’autorisation. Le produit ne doit pas être autorisé à s’attribuer lui-même une preuve de conformité : le passage à la V1 complète reste contrôlé par les scénarios et artefacts de validation définis ici.

Lecture suivante : [périmètre V1](product-scope-v1.md), [invariants](invariants.md), [architecture](../architecture/system-overview.md).

## Apprendre, exécuter et superviser — clarifications R3–R5

Une procédure peut être préparée avec un modèle, publiée et réutilisée avec de nouvelles données sans nouvel appel modèle. Ce mode n’interdit pas un second usage : un LLM peut superviser des blocs d’exécution, lire résultats et logs, puis décider aux points déclarés ; il n’a pas à choisir chaque clic. Les modes sont visibles et leurs coûts séparés. Une réparation crée une version distincte selon mandat, jamais une modification furtive du run.

L’apprentissage et la supervision peuvent utiliser un fournisseur distant ou un modèle local via LM Studio, Ollama ou un serveur équivalent tel que llama.cpp. Une exécution annoncée sans modèle n’en appelle aucun, local compris. Le moteur ne devient pas un moteur d’inférence à réécrire ; les API des serveurs sont intégrées au router Rust.

La polyvalence ne signifie pas compatibilité prouvée avec tout site ou tout métier. Elle exige un domaine sans Git obligatoire, des procédures paramétrées, des connecteurs/outils qualifiés et un onboarding compréhensible. [Périmètre généraliste](general-purpose-scope.md), [procédures](../specs/reusable-automations.md), [modèles locaux](../specs/local-models.md), [parcours guidé](../design/guided-automation-experience.md).
