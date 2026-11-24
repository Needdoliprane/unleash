# Compilateur de workflow depuis le langage naturel

## Deux phases de nature différente

La génération NL est une assistance probabiliste qui produit un draft structuré avec hypothèses et ambiguïtés. La compilation est une transformation déterministe de ce draft vers une définition normalisée et un rapport. Le modèle n'a pas le droit de publier ni de modifier un run en émettant seulement du texte. Une interface GUI, CLI, TUI ou agente utilise les mêmes opérations backend.

L'entrée `WorkflowIntent` contient texte utilisateur, contexte autorisé, définition de done, contraintes de budget, versions de ressources disponibles et définition précédente éventuelle. L'output de génération contient draft, `assumptions`, `unresolved_questions`, `source_spans` et `suggested_checks`. Une hypothèse non critique peut être rendue explicite ; une permission, une cible de merge ou un type de sortie indispensable manquant interdit le démarrage. Cela ne signifie pas demander systématiquement une validation humaine : le N+1 autorisé peut résoudre les points à l'aide des données disponibles.

## Pipeline proposé

| Passe | Contrôle | Produit |
|---|---|---|
| Parse borné | Taille, UTF-8, doublons de clés, profondeur | AST brut ou diagnostics localisés |
| Structure | Profil JSON Schema local, champs inconnus refusés | AST typé |
| Références | Nœuds uniques, bindings, rôles/skills/contrats exacts ou slots tardifs explicités | Table de symboles |
| Contrôle | Portées de résultats, domination, jointures, boucles, done atteignable | Graphe de dépendances vérifié |
| Types | Guards et sorties, profils de contrats compatibles | Diagnostics de type et pointeurs source |
| Autorité | Capacités demandées sous plafonds, scopes, outils catalogués | Besoins autorisés ou refus motivés |
| Bornes | Coût statique, activations, profondeur, budgets effectifs | Rapport de limites |
| Normalisation | Defaults explicites, ordre stable, JCS de publication | Octets canoniques et digest |
| Publication | Compare-and-swap version, rapport attaché | Définition immuable et événement |

La normalisation peut ajouter une valeur par défaut documentée, mais pas une étape métier cachée. Un workflow Dev → Merge reste sans test et sans audit si c'est l'intention. Le rapport montre le diff draft/normalisé et les valeurs ajoutées. Les textes de description ne sont pas réinterprétés à l'exécution.

## Diagnostics et simulation

Pour les slots tardifs, la validation adopte le [profil fermé V1](../architecture/skills-contract-model.md) et son [schéma](../schemas/contract-profile.schema.json). Elle épingle les versions de compilateur, évaluateur et catalogue avant le contrôle, compare exactement les digests input/output et conserve toutes les règles obligatoires. Une inclusion générale de schémas JSON n'est pas calculée. Des règles additionnelles peuvent uniquement restreindre l'acceptation par conjonction sous allowlist. Les diagnostics distinguent InputSchemaMismatch, OutputSchemaMismatch, RequiredRuleChanged et AmbiguousEntrySkill.

Un diagnostic possède code stable, sévérité, JSON Pointer, extrait d'intention source, règle, contexte et suggestion. `UnknownBinding` distingue rôle absent et rôle non autorisé. `UnboundedLoop` signale une itération dont la borne n'est pas exploitable par l'analyse ; `BoundOverflow` signale une borne calculée supérieure à 100 000 activations ou un débordement arithmétique ; un `max_iterations` absent est une erreur de structure. `IncompatibleOutput` pointe le producteur et le guard consommateur. Le même draft dans le même catalogue snapshot et la même version de compilateur doit produire mêmes diagnostics et mêmes octets normalisés.

Le [contrat du rapport F006](../features/F006-workflow-compiler/interfaces.md#rapport-de-validation) précise les coordonnées figées avec le résultat, la lecture du rapport et les conditions de réutilisation à la publication. L’identité du rapport et le watermark de sa lecture sont des métadonnées IPC distinctes du résultat déterministe du compilateur.

Le simulateur déroule des scénarios d'événements, pas des appels réels. Il peut montrer succès, correction, annulation, budget dépassé et résultat inconnu. Chaque rapport porte `simulation: true`, sa seed de fixture et les branches explorées. Le fait qu'une simulation atteigne un état terminal ne prouve ni l'accessibilité d'une ressource réelle ni la qualité d'une sortie LLM. Les scénarios non explorés restent visibles.

## Versions et concurrence

Deux éditeurs peuvent dériver un brouillon de la même version publiée. Chaque brouillon engage les octets et la filiation exacte selon le [contrat F006](../features/F006-workflow-compiler/interfaces.md#brouillon-dérivé-dune-version-publiée). Son texte se relit par blocs de la révision demandée. Les éditions font évoluer le digest du texte et conservent l’origine. Publier utilise version attendue et clé idempotente ; une course ne peut écraser un contenu déjà publié. Une publication ajoute une version et peut déplacer un alias candidat pour les prochains runs. Elle ne modifie aucun manifest engagé.

L'AST de workflow reste épinglé pendant son run. La compilation peut définir des slots tardifs pour préserver l'autonomie N+1 : chaque slot déclare niveau, plafond de capacités et contrat minimal. Lier un rôle nouvellement créé est une opération runtime distincte qui prouve sa compatibilité avant première activation. Une révision de rôle sur un slot déjà lié n'est pas un moyen de migrer le workflow.

## Sécurité et absence de compilation magique

Le parseur ne suit aucun `$ref` réseau et ne charge aucune ressource hors bundle autorisé. La taille cumulée de schémas et le coût de validation sont bornés. Les expressions libres, scripts, interpolations shell et évaluations de code sont refusés. Des instructions hostiles dans l'intention ou la documentation jointe restent des données ; seules les commandes autorisées et les éléments du DSL fermé produisent des effets.

La demande « garantir zéro bug » produit un critère qualitatif non prouvable ou un diagnostic d'ambiguïté ; elle ne crée pas un validateur magique. Le compilateur peut proposer tests et revue, mais ne doit pas déclarer qu'ils garantissent la propriété universelle. Les erreurs, critères d'acceptation et fixtures du compilateur sont détaillés dans [F006](../features/F006-workflow-compiler/README.md).

## Extension compatible de format

Les profils [v1](../schemas/workflow.schema.json), [v1.1](../schemas/workflow-v1.1.schema.json) et [v1.2](../schemas/workflow-v1.2.schema.json) sont validés séparément. Les tâches agentes gardent leurs bindings ; les actions du nouveau profil résolvent opérations et contrats versionnés du catalogue. Sous forbidden, task récursive et budgets modèle positifs sont refusés par la forme, puis les dépendances réelles sont vérifiées par la passe sémantique. Les templates guidés et les procédures F019 passent par ces mêmes opérations, sans copie du compilateur dans une interface.
