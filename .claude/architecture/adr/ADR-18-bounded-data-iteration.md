# ADR-18 — Itération bornée sur données et diagnostic bloqué

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : concurrence par élément exclue de la V1 ; la frontière de migration est fixée par Q-03, résolue ([ADR-04](ADR-04-workflow-semantics.md)) ; voir la section « Amendement R7 ». Proposé en R6 par le responsable architecture F005/F006/F019, dans les invariants confirmés. Sources d’arbitrage : [E03](../../audits/r6/expert-03-workflow.md), INV-015/018/059/064, H-05/H-18/H-21, risques R-01/R-05.

## Problème

La procédure publiée doit parcourir des données variables et pouvoir demander une supervision entre lots. v1.1 ne définit pas de liaison de ligne ni de curseur ; répéter un chemin fixe ou générer 10 000 feuilles ne satisfait pas le contrat. Un effet inconnu peut aussi bloquer le run avant le checkpoint censé demander une réconciliation.

## Options

| Option | Avantages | Coût et risques |
| --- | --- | --- |
| Bulk opaque dans chaque driver | Peu de primitives centrales | Curseur/reprises/supervision redéfinis par adaptateur ; preuves et bornes difficiles à comparer. |
| Opération bulk spécialisée contractualisée | Adaptée à une API distante réellement atomique | Contrat spécifique encore nécessaire ; ne fournit pas l’itération générique de toutes les procédures. |
| for_each fermé dans profil v1.2 | Même reducer, frames, budgets, preuves et contrôle de portée | Nouvelle primitive bornée à qualifier, comptage et projection lexicale explicites. |
| Langage de scripts libre dans le moteur | Expressivité élevée | Orchestration cachée, analyse des bornes et déterminisme mécanique perdus. |

## Décision

Adopter [for_each v1.2](../workflow-data-iteration.md) : tableau immuable, nombre maximum, body, batch_size et after_batch facultatif. Les lignes sont séquentielles dans ce profil ; le parallélisme explicite du workflow reste disponible. Le curseur avance atomiquement avec le résultat d’une ligne quiescente, jamais sur la seule annonce d’un worker.

Sous EffectUnknown, garder Run blocked. Autoriser uniquement microsteps purs et une route diagnostique publiée, qualifiée et à capacités réduites. Une observation ou disposition autorisée lève les barrières ; un avis continue ne le fait pas. Une route après une frontière de lot encore inaccessible n’est pas une solution à un blocage dans la ligne.

## Amendement R7

1. Le profil `unleash.workflow.v1.2` et son bloc `for_each` fermé sont retenus : tableau immuable et épinglé (digest, nombre, ordre), `max_items` 1..10 000, `batch_size` 1..1 000, `body` séquentiel, `after_batch` facultatif. Le curseur durable avance dans la transaction ItemCompleted du writer.
2. Borne : `B(for_each) = 1 + n×B(body) + ⌈n/b⌉×B(after_batch)`, avec refus au-delà de 100 000 activations ([itération de données](../workflow-data-iteration.md)).
3. La concurrence entre éléments d’un même `for_each` est exclue de la V1 ; le parallélisme passe par des branches ou partitions explicites, sous le scheduler commun.
4. Sous EffectUnknown, seule une résolution d’effet autorisée lève les barrières. Une opération bulk spécialisée reste possible seulement comme opération contractualisée distincte, jamais comme mécanisme implicite des CSV.
5. La frontière de migration est celle de Q-03 ; v1 et v1.1 restent inchangés.

## Conséquences et validation

v1 et v1.1 restent inchangés ; une définition engagée n’est pas migrée. Les profils ont des schémas/fixtures distincts et le même moteur. Le compilateur calcule les bornes et scopes, le writer possède les curseurs, les drivers observent/exécutent sous mandat. T-F005-003/006 et T-F019-001/003/005/011/012 qualifient cardinalités, budgets, supervision, reprises et effets inconnus. Le schéma seul ne prouve aucun de ces résultats runtime.

Réexaminer si les cas V1 exigent une concurrence par ligne non représentable par partitions explicites, ou si une API bulk démontre un meilleur contrat d’atomicité. Ne pas supprimer la fonctionnalité V1 pour éviter cette qualification.
