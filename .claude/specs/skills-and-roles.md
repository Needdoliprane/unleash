# Skills et rôles dans le travail quotidien

## Usage et gouvernance

Un skill décrit une compétence réutilisable avec instructions, entrées, sorties, outils et preuves attendues. Un rôle organise l'utilisation de compétences autour d'un objectif et de responsabilités. Trois agents peuvent utiliser le même rôle et le même skill tout en gardant des scopes, sessions et missions distincts. Leur identité ne se réduit ni au nom du skill ni à son processus.

N+2 porte la responsabilité de maintenance des skills ; « environ 99,9 % » est une intention organisationnelle du brief, pas une métrique à optimiser par création artificielle. Il peut confier la rédaction technique à un N spécialisé sous contrat, comparer les résultats et décider la publication dans ses permissions. N+1 compose et réutilise les rôles. L'utilisateur peut créer, éditer, comparer, révoquer et sélectionner des versions via les mêmes services GUI/CLI/TUI.

## Studio et parcours de publication

Le Studio présente catalogue, portée, statut, version, auteur, provenance, dépendances, schémas, outils, contrats, évaluations et usages actifs. Il distingue texte d'instruction, vérification exécutable et appréciation qualitative. Une recherche avant création, suivie de `RecordDiscoveryDecision`, produit un discovery receipt ; la décision `reuse` ou `create` et sa stratégie (dont `adapt`, `copy` ou `new`) gardent justification et candidats examinés.

Le parcours est : recherche → draft NL ou manuel → résolution des dépendances → validation → évaluations pertinentes → diff → publication. Les évaluations déterministes vérifient les fixtures de contrat et les permissions. Les évaluations métier sont définies par politique ; un second juge LLM n'est pas obligatoire. Une erreur garde le draft et son diagnostic ; elle n'efface pas la dernière version publiée.

Un skill importé conserve source et licence. Son Markdown est de l'information non fiable jusqu'à classification. Le champ tiers `allowed-tools` n'accorde aucune permission effective. Les ressources externes ne sont pas téléchargées ou exécutées simplement parce qu'une instruction les mentionne. Les projets peuvent être multilangages ; l'outil externe d'un projet est contrôlé par le broker, tandis que le code interne Unleash reste Rust.

## Bundles et versions

Prompt, outils déclarés, schémas, contrat, ressources et ACL déclarée sont publiés ensemble sous une version immuable. Le rôle référence des versions exactes de skills et un modèle ou une politique de modèle explicitement versionné. Une incompatibilité de capacités fournisseur est détectée au binding ou au démarrage d'invocation ; aucun nom de modèle supposé n'est inventé pour rendre le rôle publiable.

Un binding engagé conserve ses versions pour toutes ses tentatives et invocations. Les slots tardifs d'un workflow peuvent être liés une fois avant activation par commande explicite, sous plafond de capacités et contrat compatible. Les versions candidates suivantes sont montrées séparément. La révocation de permission demeure immédiate à l'action, indépendamment du pinning de contenu.

## Résultats et qualité

Le résultat d'un skill possède output structuré, artefacts référencés, claims et observations. Le moteur applique les règles qu'il sait vérifier. Le N+1 juge l'adéquation et choisit acceptation, correction ou escalade. Un claim « tests passés » n'est pas une observation : lorsqu'il doit être vérifiable, il prend la forme `test.expectation_satisfied` liée à l'attendu scellé et au binding courant, et sa vérité est calculée sur les observations autorisées de l'action liée ([contract-enforcement](contract-enforcement.md)).

Une variante de skill ne doit pas être créée pour masquer un échec de permission. Une création légitime peut répondre à un autre domaine, autre contrat ou autre contexte, et sa justification est conservée. La recherche de similarité aide à décider ; elle ne démontre pas l'équivalence et n'impose pas une mutualisation nuisible.

Les structures précises se trouvent dans [le modèle](../architecture/skills-contract-model.md), [le Role Registry](role-registry.md), [le moteur de contrats](contract-enforcement.md) et [F007](../features/F007-skill-studio-registry/README.md).
