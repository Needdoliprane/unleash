# ADR-02 — Événements et dispatch durable

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : dispatcher interne, ordre de livraison, retry, dead_letter, purge de l’inbox et fenêtre de reprise fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Les managers doivent être réveillés sur décisions utiles, et le daemon peut tomber après engagement d’un événement.

Exigences : REQ-PERSIST REQ-ORCH REQ-OBS. Risques/hypothèses/questions : R-01 R-03 ; H-02.

## Options et compromis

Option recommandée : **Journal transactionnel + outbox/inbox dédupliquées**.

Alternatives considérées : Bus mémoire seul ; broker distribué ; replay event sourcing complet.

La mémoire seule perd des réveils ; broker ajoute un deuxième état durable ; journal local conserve causalité sans exiger de rejouer tout le monde.

## Décision

Engager état, événements, intents et reçu ensemble. Dispatch au moins une fois ; inbox et command_id empêchent double application interne. Coalescer les causes, sans supprimer messages humains.

## Amendement R7

1. Journal transactionnel, outbox et inbox dans la base d’autorité d’[ADR-01](ADR-01-sqlite-persistence.md) ; ni broker, ni bus distribué. État, événements, outbox, intents et receipt sont engagés dans la même transaction ; le dispatch suit le commit.
2. Dispatcher interne au daemon (tâches Tokio), réveillé après chaque commit et par un balayage de sûreté chaque seconde. `outbox.destination` est l’`inbox.consumer_id`. Livraison au moins une fois, dans l’ordre de `sequence` par destination ; une seule livraison en vol par couple (destination, agrégat) ; parallélisme entre agrégats borné à 32.
3. Une entrée `leased` porte `lease_epoch = daemon_epoch` ; au démarrage, toute lease d’un epoch antérieur redevient `pending`.
4. Une erreur transitoire est rejouée en backoff exponentiel (200 ms × 2, plafond 60 s, gigue ±20 %). Une erreur non transitoire d’un handler passe en `dead_letter` après 8 tentatives.
5. Un `dead_letter` arrête les retries du couple (événement, destination), apparaît dans GetHealth et GetDaemonStatus et, si l’événement conditionne une transition, met l’agrégat en `blocked` (DELIVERY_DEAD_LETTER). Il est réarmé une seule fois, au démarrage d’un binaire dont `consumer_version` diffère. Aucune nouvelle opération publique.
6. Une ligne inbox n’est purgeable que si l’entrée outbox correspondante est `acked` depuis au moins 7 jours.
7. SubscribeEvents garantit la reprise d’un curseur de moins de 7 jours dans le même `stream_id` ; au-delà (curseur antérieur au plus ancien payload conservé) ou après restore : ResyncRequired ; PurgeHistory n’atteint jamais un événement de moins de 7 jours. La rétention longue du journal relève de Q-12 et ne descend jamais sous cette fenêtre.
8. Coalescence des réveils managériaux sur une fenêtre de 2 s par clé ; un message humain n’est jamais fusionné.
9. Wire : ResyncRequired est une classe commune d’erreur ; Backpressure n’est pas une classe et se sérialise `ResourceBusy/BACKPRESSURE`.

Détail : [event-model](../event-model.md).

## Conséquences, complexité et impact V1

Réconciliation de handlers, rétention de déduplication et watermark nécessaires ; aucune garantie exactly-once externe. Complexité faible à moyenne, requise V1.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Redelivery x10, arrêt après commit, N+2 dormant puis réveil utile unique.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../event-model.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

