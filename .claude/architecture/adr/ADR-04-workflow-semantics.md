# ADR-04 — Moteur de workflows et frontière de migration

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : la frontière de migration est le Run (Q-03 résolue) ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Un appel API, une invocation, une étape et un run ont des durées différentes. Publier une version ne doit pas changer une exécution engagée.

Exigences : REQ-WF REQ-COMP. Risques/hypothèses/questions : R-05 ; H-05 ; Q-03.

## Options et compromis

Option recommandée : **AST fini normalisé, run épinglé, adoption sur nouveau run quiescent**.

Alternatives considérées : Moteur SCXML restreint ; migration au checkpoint interne ; état en prompt.

L’AST fermé permet bornes/types ; SCXML apporte sémantique riche mais reste à restreindre ; migration interne exige mapping et autorité ; prompts ne garantissent pas les transitions.

## Décision

Un run conserve AST/hash et bindings engagés. Slots tardifs déclarés peuvent recevoir nouveaux bindings exacts avant première activation. Le cycle d’adoption est le Run (Q-03, amendement R7). Les boucles gardent versions.

## Amendement R7

1. Le moteur est un AST fini, fermé et normalisé (profils `unleash.workflow.v1`, `v1.1` et `v1.2`), interprété par un reducer Rust unique ; un profil inconnu est refusé. SCXML, état porté par le prompt et expressions programmables sont écartés. Les bornes du profil publié ([workflow-dsl](../workflow-dsl.md)) sont les valeurs V1 ; elles ne changent que par une nouvelle version de profil.
2. Frontière de migration (Q-03) : définition, version d’AutomationArtifact et bindings engagés par `CreateRun` restent ceux du Run jusqu’à son état terminal (`succeeded`, `failed`, `cancelled`). Aucune migration en place en V1, à aucun grain (appel, invocation, étape, itération, lot, checkpoint, pause ou reprise) ; aucune commande de migration de Run.
3. Une nouvelle version n’est adoptée que par un nouveau Run à référence exacte (identifiant, version, digest), qui déclare `predecessor_run_id`. `CreateRun` refuse un successeur tant que le prédécesseur n’est pas terminal (`ResourceBusy/PREDECESSOR_NOT_TERMINAL`) et un second successeur direct (`InvalidArgument/SUCCESSOR_EXISTS`) : la lignée reste linéaire.
4. Les effets `outcome_unknown` du prédécesseur n’empêchent pas la création du successeur ; ils maintiennent des barrières qui refusent les seules actions concernées jusqu’à réconciliation ou résolution. Pour adopter v2 sans attendre la fin naturelle, l’auteur autorisé annule v1 puis crée le successeur.
5. La liaison tardive d’un slot vierge, avant sa première activation et dans son plafond, reste la seule évolution permise dans un Run ; ce n’est pas une migration. Une révocation de sécurité s’applique immédiatement aux actions futures sans changer de version. Les budgets de mission restent cumulés sur toute la succession.

## Conséquences, complexité et impact V1

Expressivité limitée aux primitives des profils publiés ; nouvelles primitives évoluent par version. La frontière de cycle est fixée par Q-03 (amendement R7).

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Scénarios B/C/K/L, guards inconnus, joins et effets résiduels.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../workflow-dsl.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

