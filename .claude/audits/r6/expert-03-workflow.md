# R6 — Expert 03 : workflow et exécution déterministe

**Statut : revue documentaire, 16 novembre 2026.** Périmètre : DSL v1/v1.1, états, observations, exemples, F005/F006/F019. Aucun runtime produit exécuté. Trois raccords documentaires restent à préciser ; les risques déjà assumés ne sont pas requalifiés en défauts.

## R6-E03-01 — Parcours de lignes sans contrat de progression complet

**Priorité : élevée. Constat confirmé.** [F019, Paramètres et lots](../../features/F019-reusable-automations/technical-design.md) promet une projection de ligne, un curseur durable et des checkpoints par lot. [Le modèle d’artefact](../../architecture/automation-artifact-model.md) fixe un plafond d’import de 10 000 lignes et calcule les activations après expansion. Cependant, [v1.1](../../schemas/workflow-v1.1.schema.json) ne propose que des entrées `literal/path` et `repeat(body, until, max_iterations)` ; ni liaison de ligne courante, ni avancement du curseur ne sont définis. [Le DSL](../../architecture/workflow-dsl.md) efface les résultats internes au début de chaque itération et borne chaque boucle à 100, la définition à 256 nœuds.

**Contre-exemple et risque.** Une même procédure publiée doit appliquer une action par ligne à deux imports de 2 puis 300 lignes. Un chemin `/input/rows/0` répète la première ligne ; lire le résultat de l’itération précédente n’est pas permis. Déplier 300 feuilles dépasse la borne des nœuds. Des boucles imbriquées augmentent les activations possibles, mais ne définissent toujours pas le choix de la ligne suivante. Deux équipes peuvent inventer des curseurs, comptes d’activations et reprises incompatibles.

**Objection examinée.** Les 10 000 lignes sont un plafond d’import, pas une promesse que tout lot est admissible. Une opération de catalogue peut traiter un lot intérieurement ; cette solution est compatible avec le dossier. Il manque néanmoins son contrat précis, notamment pour la supervision entre lots. Le défaut n’est donc pas une impossibilité du DSL.

**Recommandation.** Fixer une représentation canonique V1.1 des lots dans une opération de catalogue bornée : origine du curseur, révision et plage immuables, progression atomique avec résultats/effets, reprise, sortie de lot et distinction entre activation AST et actions internes. Définir comment un checkpoint publié reçoit le lot suivant sans lecture interdite ni mutation du workflow. Raccorder le calcul des bornes F006 et la validation des données F019. **Alternative :** introduire une itération sur données dans un nouveau profil versionné, avec schéma et portée de ligne explicites.

**Acceptation.** Compléter T-F019-003/005/012 avec 0, 1, 100, 101, 300 et 10 000 lignes : représentation identique expliquée, refus explicite si budget insuffisant, crash entre effet et progression sans saut/doublon, supervision à la frontière déclarée, mêmes données/manifest après reprise.

## R6-E03-02 — Route de supervision sous effet inconnu : transition manquante

**Priorité : élevée. Constat confirmé.** [La machine de Run](../../architecture/state-machines.md) fait passer `running → blocked` sur `EffectUnknown` ; sa reprise passe par `ResolveBlock` avec preuve ou autorisation nouvelle. [F019](../../features/F019-reusable-automations/technical-design.md), [F005](../../features/F005-workflow-engine/technical-design.md) et [le profil v1.1](../../architecture/automation-execution-profile.md) permettent simultanément qu’une collecte réussie rende un ObservationReport `unknown`, puis qu’un `choice` publié active une supervision pour observation/réconciliation/escalade.

**Contre-exemple et risque.** Une mutation a un receipt perdu. La collecte se termine avec `effect_state=unknown`; la branche publiée suivante demande au superviseur de choisir une réconciliation. Si `EffectUnknown` bloque le Run avant dispatch du `choice/task`, aucune preuve nouvelle n’est encore disponible pour `ResolveBlock`. Si la branche reste exécutable, cette exception à l’état `blocked` et l’ordre des événements ne sont pas spécifiés. La doc ne permet pas de déterminer la transition suivante sans convention supplémentaire.

**Objection examinée.** Les textes interdisent correctement répétitions et mutations conflictuelles et autorisent les opérations de résolution. Ce constat vise leur raccord à la machine de contrôle, pas l’absence de barrière de sécurité.

**Recommandation.** Définir dans F005 les transitions permises sous blocage d’effet : traitement du résultat d’observation, évaluation du `choice`, admission d’une tâche de résolution publiée sous politique/budget autorisés, maintien des barrières et événement permettant la reprise normale. Préciser l’ordre durable et le cas sans route. **Alternative :** arrêter toute progression AST et imposer une réconciliation extérieure via les commandes existantes avant reprise ; modifier alors la promesse de supervision interne correspondante.

**Acceptation.** Étendre T-F005-003/006 et T-F019-001/011 : receipt perdu avant checkpoint, route présente/absente, replay/supervised, décision `continue` abusive, crash à chaque frontière. La trace doit montrer un seul chemin autorisé, zéro répétition et aucune clôture fictive.

## R6-E03-03 — Exception de qualification non propagée à F006

**Priorité : modérée.** [F006, Dépendances](../../features/F006-workflow-compiler/dependencies.md) affirme que F005 consomme « uniquement des définitions publiées ». [F005, Interfaces](../../features/F005-workflow-engine/interfaces.md) et F019 admettent un snapshot non publié `qualification_only` sous `purpose=evaluation`.

**Contre-exemple.** L’évaluation autorisée d’un candidat figé est admissible selon F019, interdite selon F006. **Correction :** limiter la règle « publié uniquement » aux runs opérationnels et référencer l’exception avec autorité, digest et cible exacts. **Alternative :** un état de registre dédié explicitement admissible à la qualification. **Acceptation :** même snapshot refusé en opérationnel, accepté en évaluation autorisée ; simulation toujours sans effets. Réutiliser T-F019-002/004/008.

## Vérifications, hypothèses et limites

Contrôles réalisés en lecture seule avec le validateur JSON Schema présent : trois schémas valides ; cinq fixtures workflow/observation conformes. Ces résultats ne vérifient pas la sémantique produit. Restent correctement distingués : done faux/inconnu, succès de collecte/succès métier, binding tardif vierge/substitution engagée, version nouvelle/nouveau run. La frontière de cycle et la migration restent des propositions ouvertes. Aucune bibliothèque ni API externe nouvelle n’est recommandée.

## Addendum — relecture des corrections R6

**E03-01 : mécanisme corrigé, raccord normatif restant.** [Le profil v1.2](../../architecture/workflow-data-iteration.md) définit maintenant tableau immuable, portée lexicale, séparation body/after_batch, curseur transactionnel, reprise et borne cumulative. La supervision d’un effet inconnu dans body est distinguée du checkpoint de lot encore inatteignable. Cependant, REQ-AUTO-003 et AC-F019-003 exigent encore exclusivement v1.1, comme le champ « Procédure compilée » du modèle d’artefact et l’introduction technique F019. Ces clauses doivent autoriser le profil exact v1.1 ou v1.2, ce dernier obligatoire pour for_each ; le schéma historique demeure inchangé.

**E03-02 : corrigé documentairement.** La machine conserve blocked, autorise les microsteps puis les seules activations diagnostiques publiées et qualifiées, et réserve ResolveBlock à une résolution autorisée. Le commit du blocage précède toute admission normale. Une route présente dans body est atteignable ; son absence attend la réconciliation extérieure. Aucun avis continue ne suffit.

**E03-03 : corrigé documentairement.** F006 distingue désormais publication opérationnelle et snapshot qualification_only admis sous autorité d’évaluation.

**Vérification en mémoire :** trois schémas valides ; cinq workflows conformes, dont les quatre fixtures historiques ; dix cas ciblés de forme conformes à leur oracle. Les anciens profils refusent for_each. La borne de la nouvelle fixture est 10 011 activations pour un budget de 20 000. Portées et transitions ont été relues, sans prétendre exécuter un compilateur sémantique ou un runtime.
