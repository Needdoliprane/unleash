# Retours d’expérience Unleash

Le [skill `retex`](../skills/retex/SKILL.md) utilise cette base pour retrouver les corrections qui s’appliquent au cas rencontré. À [G7](goal-protocol.md#7-clôture-et-rapport), `retro` reprend les mêmes entrées pour comparer leurs effets. Le journal du goal conserve les observations et la fiche donne l’état courant de la livraison.

## RETEX-001 — Raccorder le parcours avant de compter un lot clos

Sur M2, les intégrations ont ajouté le démarrage du run, la création d’agent, des commandes de profils et la lecture de la préférence du skill. Le premier `AssignTask` vérifié consommait encore un bundle publié par le banc. Le suivi répétait `13/16` et détaillait les sous-lots sans identifier assez tôt le premier raccord qui permettrait d’accepter l’un des trois lots ouverts.

Le [protocole de goal](goal-protocol.md#6-boucle-et-conditions-darrêt) demande désormais un résultat d’acceptation, une action et un responsable pour chaque lot ouvert. Dans la [fiche M2](../state/goals/g0003-m2/goal.md#actions-de-clôture-ouvertes), le premier contrôle F003 suit `StartRun`, `CreateAgent` et `AssignTask` sur l’IPC public. L’assertion vérifie que le daemon produit le bundle à partir de la politique autorisée et des sources de contexte, puis observe l’admissibilité. Le rattachement de ce contrôle aux lots est appliqué au suivi. La preuve produit reste à obtenir.

## RETEX-002 — Rejouer le bon binaire

Pendant la qualification de Settings, une réextraction a conservé les dates Git et Cargo a réutilisé le binaire de référence. Le journal attribué au candidat contenait encore les deux échecs de la référence.

Les [règles de test](../rules/testing.md) demandent d’actualiser les dates, de comparer les empreintes et d’établir la provenance du binaire. Le banc IPC a déjà appliqué cette correction. Après recompilation attestée des deux révisions, il a observé deux échecs attendus sur `fcf5db4` et deux succès sur `0350f90`. Voir la [preuve RETEX-002](../state/goals/g0003-m2/journal.md#preuve-retex-002).

## RETEX-003 — Vérifier l’identité réellement engagée

La première revue de la préférence du skill a trouvé deux cas absents du banc. Un rôle de la même portée provenait d’une autre affectation. La dépréciation d’un skill encore épinglé était traitée comme sa révocation.

Pour une lecture issue d’un plan engagé, reprendre le lecteur de l’affectation effective et ajouter les contre-cas d’identité et de cycle de vie au test du contrat. Les deux cas ont échoué sur `9699167`, puis réussi sur `8acdc07`. Le candidat corrigé a été intégré dans `f0c0a20` après deux nouvelles revues. Voir la [preuve RETEX-003](../state/goals/g0003-m2/journal.md#preuve-retex-003).
