# Éditeur de workflows

L’éditeur associe description utilisateur, définition structurée et représentation visuelle. Une procédure NL génère un draft ; le panneau de validation indique schéma, types, références, bornes et conditions de terminaison. Les diagnostics ciblent un chemin exact de l’AST, avec explication et suggestion sans modification automatique cachée.

## Modes

Catalogue : versions publiées immuables et leurs usages. Draft : édition de blocs, ordre, branches, répétitions, bindings, contrats et done. Simulation : événements fictifs, trajectoire et raisons de blocage clairement marqués « simulés ». Run : lecture seule des activations et preuves réellement engagées.

Les étapes test/audit sont disponibles, jamais ajoutées obligatoirement. Un workflow Dev → Merge doit être réalisable et affichable sans avertissement de qualité universel qui prétendrait l’interdire.

## Versions et bindings

Publier v2 laisse le run v1 visible avec sa version. L’interface affiche la prochaine frontière éligible proposée et les raisons qui empêchent adoption : enfants actifs, effet incertain ou absence de nouvelle exécution. Un binding tardif peut être résolu avant activation dans son plafond ; un binding engagé ne se remplace pas depuis un sélecteur latest.

## Interactions et erreurs

Les opérations drag-and-drop ont alternative clavier. Une boucle affiche sa borne et le budget global. Une condition inconnue ne paraît pas fausse. Un draft invalide se sauvegarde comme draft mais ne peut démarrer. Une publication concurrente provoque conflit de révision.

Références : [DSL](../architecture/workflow-dsl.md), [F006](../features/F006-workflow-compiler/README.md). L’éditeur n’est pas un moteur d’exécution distinct.

