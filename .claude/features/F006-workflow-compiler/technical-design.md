# F006 — Design technique proposé

## Architecture Rust

`IntentDraftService` orchestre l'appel de modèle à travers le routeur commun et conserve son manifest. `WorkflowParser` applique limites d'entrée et détection de doublons. `WorkflowValidator` contrôle schéma et sémantique. `WorkflowNormalizer` produit octets canoniques et diff. `WorkflowSimulator` consomme des événements fictifs. `WorkflowPublisher` effectue l'écriture atomique au registre. Seule la première étape requiert éventuellement un modèle ; toutes les passes de compilation sont déterministes.

Les types d’AST correspondent aux six variantes du profil v1 et à la variante action supplémentaire du profil v1.1, puis for_each du profil v1.2. Le tag de format distingue les définitions ; le même moteur de frames traite les trois profils. Une table de symboles associe nœud, position, producteur de résultat et binding. L'analyse descendante maintient un environnement de types et les résultats disponibles. Une séquence étend cet environnement après chaque enfant. Une branche parallèle reçoit un environnement isolé et n'exporte ses résultats qu'à travers le bloc joint. Une boucle réinitialise sa frame à chaque itération et exporte uniquement son résultat déclaré.

## Calcul de borne

Le calcul utilise de l'arithmétique vérifiée : une feuille task ou action vaut au plus max_attempts ; séquence et parallèle additionnent ; choix prend la branche maximale ; boucle multiplie par max_iterations ; fail compte une activation interne. Le calcul ajoute les activations des blocs eux-mêmes selon la convention de borne du [DSL](../../architecture/workflow-dsl.md#bornes-et-garanties-exactes) afin que le budget runtime soit comparable. Une borne dépassant 100 000 ou un overflow est un diagnostic, jamais une valeur saturée annoncée comme sûre. Les budgets utilisateurs plus faibles restent autorisés et visibles comme limite d'exécution distincte.

## Normalisation et publication

Le normaliseur explicite les defaults du profil, conserve l'ordre des branches/cases et produit un diff à pointeurs JSON. JCS est proposé pour la représentation canonique ; la crate Rust devra être choisie et vérifiée sur ses vecteurs de conformité. Les numéros de révision et budgets évitent les entiers hors plage sûre du profil. Le digest identifie le contenu, sans prouver provenance ni qualité.

Le registre contient draft mutable révisionné, rapport immuable, définition publiée et lien de filiation. Publier utilise le hash du rapport et du draft normalisé pour empêcher un changement entre validation et commit. Une publication concurrente ne peut écraser une version existante. Un alias `preferred` peut évoluer séparément pour les futurs runs ; le reçu signale explicitement ce déplacement.

## Sécurité et erreurs

Les schémas sont locaux, bornés et sans accès réseau. La prose et les ressources importées sont traitées comme données. Des champs `code`, `script`, `eval` ou inconnus sont rejetés par le profil ; une chaîne dans une description ne s'exécute jamais. L'autorité vérifie les capacités demandées, sans considérer le rôle auteur comme permission d'élargissement. Les slots tardifs doivent déclarer un contrat minimal et un plafond ; un slot sans contrainte ne peut être publié.

Les diagnostics possèdent code, sévérité, pointeur, source d'intention, règle et correction suggérée. Ils distinguent InvalidSyntax, DuplicateKey, UnknownNode, UnknownBinding, IncompatibleOutput, CrossBranchRead, UnboundedLoop, BoundOverflow, PermissionExpansionDenied et PublicationConflict. Un échec conserve le draft. Un problème fournisseur affecte la génération NL mais n'empêche pas l'édition manuelle ni la compilation d'un draft déjà disponible.

## Simulation

Le simulateur réutilise la sémantique pure F005 sans dispatch d'effets réels. Il enregistre scénario, seed de fixture, branches et hypothèses de résultats. Il explore un nombre borné de chemins choisi par l'appelant ; son rapport montre les branches non explorées. Une simulation ne vérifie ni disponibilité de l'outil externe ni permissions actuelles à une date future. Cette limite est affichée auprès du résultat, pas seulement dans une note technique éloignée.

## Validation du profil v1.1

La passe structurelle interdit récursivement les task et budgets modèle positifs sous forbidden. La passe sémantique résout les OperationRef exactes, leur schéma d’entrée/sortie, leurs contrats, leur classe d’effet, leur politique de retry et leur profil de dépendances/driver. Une opération qui délègue à un modèle ou échappe à la garantie requise est refusée dans un replay qualifié. Les paths d’entrée sont résolus depuis input/results/facts, avec contrôle de domination et de type ; aucune interpolation shell n’est ajoutée.

Le rapport calcule séparément activations cumulées, sous-plafond de concurrence d’invocations agentes (`max_agent_executions`) et sous-plafond de workers d’action (`max_concurrent_action_executions`). Leur somme active est soumise au pool global 5/50. Les checkpoints de supervision correspondent à des task leader_decision déjà dans l’AST ; on_exception est une politique de compilation vers un choice sur ObservationReport, pas un moteur de traitement d’exception caché. La qualité de l’adaptateur et de ses assertions reste une qualification produit distincte du compilateur.

## Parcours de données R6

Les lots génériques utilisent le [profil v1.2 for_each](../../architecture/workflow-data-iteration.md), explicitement sélectionné et épinglé. Body reçoit une ligne immuable dans sa portée /items ; after_batch reçoit un résumé/référence de résultats de lot. Le curseur et les activations appartiennent au writer/reducer commun, pas au driver. La borne statique suit workflow-data-iteration : 1 + max_items×B(body) + ceil(max_items/batch_size)×B(after_batch), arithmétique vérifiée et refus au-delà de 100 000 ; validation des données et budgets avant premier effet. Une collecte unknown ne peut faire avancer le curseur pour atteindre une supervision future : une route de diagnostic doit être présente à l’endroit réellement bloqué.

## Attentes de test prépubliées — raccord R6

Les validateurs claim_predicate_matches et test_execution_matches du catalogue v2 utilisent les [contrats F008](../F008-contract-engine/interfaces.md). Chaque règle déclare un expectation_slot littéral ; compiler/registry dérivent les slots requis et vérifient leur type, sans lire un attendu proposé dans la sortie future du worker. L’admission résout chaque slot depuis une version acceptée du plan/contrat/mandat, puis scelle ExpectedExecutionBinding avec action et digest avant dispatch. Une absence, ambiguïté ou tentative de substitution depuis un log/output bloque l’effet concerné.

Les profils/skills v1 existants gardent leur garantie historique d’intégrité des liens. Exiger la vérité observable d’un claim ou la réussite d’une commande déterminée nécessite les validateurs explicites et une attente scellée ; aucun pass sémantique n’est déduit d’une migration de catalogue. Les changements d’attente passent par révision autorisée, jamais par choix a posteriori d’une suite favorable.
