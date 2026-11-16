# Audit Expert 3 — Workflow et exécution déterministe

**Émis avant les spécifications de ce périmètre, 16 novembre 2026.** Regard spécialisé réalisé par le sous-agent `workflow_agents_contracts`. Aucun moteur existant n'a été exécuté. Sources : [registre documentaire](../research/workflow-skills-sources.md), SRC-WF-01 à 04 et SRC-CT-01/02.

## Constats

**Confirmé par le brief.** Le langage naturel produit un draft ; la version validée devient l'entrée d'un moteur à états. Les workflows sont configurables, les étapes de test et d'audit ne sont pas universellement obligatoires, le succès suit la définition de done choisie. Le harness est seul propriétaire des transitions. Les appels N+1/N+2 sont des invocations bornées ; ils ne doivent pas reconstruire un second moteur dans leurs prompts.

**Constat de conception.** Un appel de modèle n'est pas une frontière de workflow fiable. Il peut servir à demander un plan, interpréter une erreur ou examiner la même étape. Remplacer la définition au prochain appel ferait dépendre les transitions restantes du comportement du modèle. Une version nouvelle pourrait supprimer l'étape courante, changer son contrat ou rendre un merge déjà préparé invalide. L'épingle doit donc désigner le contenu complet de la définition, pas simplement son numéro d'affichage.

**Confirmé par les sources.** SCXML fournit une sémantique explicite des transitions mais ne borne pas tous les macrosteps. Temporal illustre la séparation entre décisions rejouables et interactions externes ainsi que la nécessité d'une stratégie de versionnement. Ces sources motivent des questions de conception ; elles ne suffisent pas à sélectionner un runtime pour Unleash.

## Risques classés

| Risque | Conséquence | Réponse proposée |
|---|---|---|
| R-WF-01, critique : hot swap au prochain appel LLM | Replay divergent, contrôle utilisateur trompeur | Run épinglé ; publication séparée de l'adoption |
| R-WF-02, critique : code libre dans guards | Accès non contrôlé, non-termination, nondéterminisme | AST fermé, valeurs typées, aucun `eval`, aucune I/O dans les guards |
| R-WF-03, critique : retry après effet inconnu | Double merge, double action distante | État `outcome_unknown`, réconciliation avant toute répétition |
| R-WF-04, élevé : join sans règle d'annulation | Branche orpheline après succès annoncé | Join documenté, arrêt demandé puis acquitté ; inconnues exposées |
| R-WF-05, élevé : choix fondé sur fait absent | Fin prématurée ou branche incorrecte | Trois valeurs vrai/faux/inconnu ; inconnu bloque au lieu d'inventer |
| R-WF-06, élevé : done confondue avec sécurité | Audit arbitraire imposé ou contrôle contourné | Critère utilisateur distinct de préconditions d'autorisation |
| R-WF-07, élevé : temps réel lu pendant le replay | Divergence silencieuse | Événements de timer et réservations de budget journalisés |

## Hypothèses et questions ouvertes

**H-WF-01 proposée.** Le « cycle » est un run dont la définition et les bindings restent immuables jusqu'à `succeeded`, `failed` ou `cancelled`. Une invocation éligible est le démarrage d'un nouveau run explicitement autorisé, pas n'importe quel tour de conversation. Les relances techniques, reprises après crash, corrections internes et boucles sont toujours dans le même run.

**Q-WF-01 ouverte.** Louis pourrait vouloir qu'un cycle soit une itération de correction à l'intérieur d'une mission. Cette alternative n'est pas écartée par une reformulation du brief. Elle exigerait un checkpoint de migration explicite : état durable compatible, actions en vol réconciliées, mapping versionné des résultats et décision d'adoption. La conception candidate préfère une unité plus simple et observable en conservant cette divergence dans les décisions.

**H-WF-02 proposée.** Le DSL est un arbre de blocs structurés. Une branche peut exécuter des tâches en parallèle ; les seuls retours sont des répétitions avec maximum entier explicite. Un budget global réduit encore la borne. Cette structure privilégie la vérification et l'édition visuelle au détriment des graphes arbitraires.

## Objections et alternatives

**Objection : un DSL restreint freinera l'autonomie.** L'autonomie utile porte sur le choix des tâches, skills, scopes et corrections. Elle peut s'exprimer par des propositions de plans et par des décisions structurées dans des points prévus. Le moteur contrôle l'application de ces propositions. Ajouter une boucle non bornée pour préserver l'autonomie rendrait les budgets peu crédibles.

**Alternative : workflow codé en Rust.** Elle offre un langage familier aux développeurs mais complique les créations NL, les inspections utilisateur et la vérification du pouvoir d'un workflow importé. Elle reste pertinente pour les adaptateurs internes ; elle n'est pas retenue comme format d'auteur V1.

**Alternative : intégrer Temporal ou SCXML.** Ce serait une décision de dépendance et d'exploitation substantielle. Le produit vise un daemon local ; le dossier propose d'abord un reducer pur et un stockage local. Une comparaison mesurée des coûts est nécessaire avant de changer ce choix. Réimplémenter toutes les fonctionnalités de ces produits n'est pas un objectif.

**Objection : pinning bloque les correctifs urgents.** Une révocation de permission est immédiate et ne change pas les octets du workflow. Elle peut suspendre une étape devenue interdite. Pour une définition dangereuse, on arrête le run, réconcilie les effets, puis crée un autre run. On conserve donc l'audit et la sécurité sans substitution cachée.

## Recommandations

Adopter une version immuable et un hash de contenu ; conserver les anciennes définitions tant que leurs runs ou preuves sont conservés. Définir commandes, états et raisons terminales indépendamment des prompts. Séparer le résultat structurel d'une tâche, le jugement technique du N+1 et la réussite de la mission. Valider les références, types, bornes, budgets et permissions à la publication ; ne pas prétendre que JSON Schema suffit. Le simulateur doit consommer des événements fictifs et publier un rapport qui identifie clairement ce qui a été simulé.

Pour le parallélisme, définir `all`, `any_success` et `quorum`. Une branche gagnante peut déclencher l'arrêt des autres ; la terminalité du run exige leur réconciliation ou un incident explicite. Un résultat tardif garde sa preuve, mais ne ressuscite pas une étape ni ne réécrit un choix déjà engagé. Les deadlines et compteurs s'appliquent aussi aux décisions des leaders.

## Vérifications et critères de décision

### Addendum de consolidation : autonomie et bindings tardifs

L'hypothèse initiale d'un manifest intégralement résolu au début pouvait empêcher N+1 de sélectionner une compétence manquante créée pendant la mission. Après objection du chief architect, la recommandation courante garde l'AST de workflow immuable et chaque binding engagé immuable, mais autorise une liaison append-only sur un slot tardif déclaré et encore vierge. Le slot impose contrat minimal et plafond de capacités ; la liaison exacte précède la première activation et ne change plus aux retries ou itérations. Cette révision préserve l'objection initiale et ne l'assimile pas à une décision utilisateur sur la migration. La publication d'une v2 ne modifie toujours aucun binding existant.

Les tests devront rejouer une même séquence canonique et comparer les états ainsi que les intentions d'effets, sans refaire les appels externes. Publier v2 pendant un run v1, faire plusieurs appels LLM, crasher puis reprendre : chaque transition doit encore référencer v1. Une seconde exécution doit adopter v2 si elle est la version sélectionnée au moment de sa création. Un workflow Dev → Merge doit réussir sans test ni audit ajouté par le moteur, tout en respectant les autorisations du merge.

Injecter guards absents, boucles à leur borne, budget épuisé, branches concurrentes gagnantes et événement doublé. Le critère de sélection de l'architecture est l'absence d'ambiguïté observable et l'existence d'un résultat borné ou d'un blocage expliqué. Ces vérifications sont planifiées, pas réalisées sur un produit. Elles deviennent REQ-WF et REQ-COMP dans les features correspondantes.
