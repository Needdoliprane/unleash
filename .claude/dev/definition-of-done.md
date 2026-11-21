# Définition de terminé

**Statut : règle adoptée en R7.** Une tâche ou un goal n'est « terminé » que si chaque point applicable est observé, avec la sortie réelle des commandes consignée dans le journal du goal.

## Tâche de code

1. Le comportement demandé est implémenté dans le périmètre de fichiers de la tâche, sans modification hors périmètre non justifiée.
2. Les tests `T-…` des exigences visées existent, portent leur identifiant, couvrent le cas positif et les rejets significatifs, et passent.
3. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features` et `cargo doc --workspace --no-deps` passent sur la composition contenant les tâches dont les diffs ont été revus. Le journal relie chaque lot au SHA contrôlé.
4. Les [règles Rust](../rules/rust.md) sont respectées : pas d'`unwrap`/`panic` hors tests, pas d'`unsafe` hors adaptateurs recensés, erreurs typées, pas de logique métier dans les clients.
5. Si les dépendances changent : `cargo deny check` passe et la justification figure dans le commit.
6. Si un contrat change (DTO, schéma, état, erreur, opération, migration) : le dossier `.claude/` est mis à jour selon les [règles du dossier](../rules/design-dossier.md) et `python3 tools/design-package/validate.py .` termine sans erreur.
7. Un `verifier` distinct de l'implémenteur a accepté le diff et vérifié les contrôles ciblés du lot, en examinant les sorties rattachées aux mêmes entrées ou en exécutant les contrôles autorisés nécessaires. Les résultats communs des gates sont rattachés à la composition selon le [protocole](goal-protocol.md#4-vérification-indépendante).
8. Le commit est sur la branche de la tâche, avec un message Conventional Commits qui cite les `REQ-…` couverts.

## Goal

1. Toutes les tâches sont intégrées ou explicitement `blocked` avec diagnostic.
2. Les critères mesurables du goal sont observés sur la branche du goal puis, après fusion, sur `dev`.
3. Le journal contient les commandes, sorties, décisions, escalades et modèles utilisés.
4. Les preuves réellement exécutées sont enregistrées dans `qualification/evidence/` ; aucune EV n'est marquée acquise sans elles.
5. Les branches locales et worktrees intégrés sont retirés et vérifiés selon le [cycle de vie](worktrees.md). Le journal donne le résultat pour chaque ressource et pour les caches privés arrivés en fin d’usage.
6. Le rapport final dit ce qui reste, les risques et le prochain goal recommandé.
7. La vue des travaux correspond aux fiches canoniques, vérifiée par `python3 -B tools/design-package/current_state.py . --check`.

## Ce qui ne compte pas comme preuve

Une affirmation d'agent, un test ignoré ou affaibli, un mock qui recopie l'implémentation, une sortie de commande non relue, un build réussi sur une autre branche, une capture d'écran de documentation, un appel réseau simulé présenté comme qualification réelle.
