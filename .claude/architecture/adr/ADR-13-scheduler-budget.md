# ADR-13 — Concurrence, fairness et budgets

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : paramètres de Q-15 : pool unique, classes `control` et `work`, réserve souple par défaut ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Cinq slots peuvent être tous occupés par des exécutants ; un manager doit pouvoir décider sans surallocation.

Exigences : REQ-AGENT REQ-MODEL. Risques/hypothèses/questions : R-03 R-04 ; H-04 H-14 ; Q-15.

## Options et compromis

Option recommandée : **Admission globale 5 défaut/50 dur, waiting durable et réserve configurable**.

Alternatives considérées : Réserve dure contrôle ; réserve souple prêtée ; préemption arbitraire.

Dure réduit débit mais disponibilité plus claire ; souple dépend délais/checkpoints ; préemption arbitraire ne prouve pas arrêt distant.

## Décision

Compter N/N+1/N+2 identiquement pendant invocation active. Processus auxiliaires bornés séparément. Réservation souple par défaut, deadlines et aging ; option dure (Q-15). Budget/quota peut toujours bloquer admission.

## Amendement R7

1. Admission globale par utilisateur OS : un seul pool C = `effective_limit` (défaut 5, 1..50, borne dure 50). Il compte `agent_invocation` N/N+1/N+2, `replay_worker` et `provider_probe`, sans exemption. Plusieurs racines, projets ou backends locaux ne multiplient pas ce plafond.
2. `admitted`, `running` et `checkpointing` consomment une unité, de même que `reconciling` tant que l’ancien exécuteur peut agir ; `waiting_durable` ne rend l’unité qu’après un checkpoint sûr et la neutralisation de la continuation. Une baisse sous l’occupation inscrit `requested_limit` en attente et gèle les admissions jusqu’à son activation atomique.
3. Classes (Q-15) : `control` (invocation N+1/N+2 déclenchée par message humain, escalade, blocage, décision de mission ou de goal, route diagnostique sous blocage) et `work` (tout le reste). Réserve souple d’une unité pour `control` par défaut, prêtée au travail tant qu’aucun contrôle admissible n’attend ; option dure seulement si C ≥ 2. Une combinaison incohérente est refusée (`InvalidArgument/CONTROL_RESERVE_INCOHERENT`), sans ajustement silencieux.
4. Aucune surallocation : aucune unité n’est rendue au seul envoi d’une demande de checkpoint ; un contrôle ne préempte jamais un autre contrôle ; aucun arrêt forcé ne sert à accélérer un manager. Anti-famine par `control_burst_limit=2` ; équité entre projets par déficit pondéré.
5. Processus auxiliaires, contextes navigateur et générations locales sont des ressources additionnelles ([ADR-16](ADR-16-reusable-automation-and-supervision.md), [ADR-17](ADR-17-local-model-providers.md)), jamais des unités supplémentaires. Budget, quota et sécurité peuvent toujours bloquer une admission. Seules les valeurs par défaut sont mesurées en recette ([scheduling](../../specs/scheduling.md)).

## Conséquences, complexité et impact V1

Ne pas promettre délai absolu face panne OS/API. Aucun slot libéré tant que l’ancienne exécution locale peut encore agir. Complexité moyenne, indispensable dès F003.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Vingt leaders et cinq workers, attente durable de raison `waiting_owner` (valeur de `invocations.wait_reason`, pas un état), appels lents, dépassement 51 rejeté.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/scheduling.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.


## Précision R6

Le même pool utilisateur compte aussi replay_worker et provider_probe, sans augmenter 5/50. Le verrou d’autorité est par utilisateur OS indépendamment des racines/profils. La qualification inclut deux racines concurrentes et un probe avant création d’agent. Les attentes ne libèrent pas artificiellement la capacité tant qu’un exécuteur peut agir. Voir [runtime-lifecycle](../runtime-lifecycle.md) et [scheduling](../../specs/scheduling.md).
