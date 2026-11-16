# ADR-05 — Sessions durables et incarnations éphémères

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : autorité de continuité, identités, workers, process-host et ancrage de signal fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

La RAM d’un fork n’est pas une sauvegarde. La reprise doit reconstruire contexte et droits sans usurper un processus.

Exigences : REQ-AGENT REQ-TERM REQ-REC. Risques/hypothèses/questions : R-02 R-17 R-24 ; H-03 H-04 H-13.

## Options et compromis

Option recommandée : **Session logique + bundles immuables par invocation + attempts identifiés**.

Alternatives considérées : Une session fournisseur comme autorité ; snapshot mémoire complet des workers.

Le modèle local permet portabilité et reprise ; session fournisseur lie conservation/capacités ; snapshot RAM coûte et ne rend pas les effets externes atomiques.

## Décision

Historique et artefacts durables, identités séparées, process_run_uuid et fence. Contexte sélectionné puis payload envoyé traçables ; compactions versionnées. Process-host PTY pour continuité limitée au même boot.

## Amendement R7

1. L’autorité de continuité est la session logique Unleash persistée dans SQLite, avec `context_mode=explicit_unleash_history`. Un identifiant de réponse, de conversation ou de cache fournisseur n’est qu’une corrélation, jamais une source de reprise.
2. Chaque invocation persiste avant dispatch un InvocationBundle immuable ; `bundle_hash` est le SHA-256 de sa sérialisation RFC 8785. Chaque appel API possède son action, son `model_call_id` et son ModelCallManifest.
3. Agent, session, invocation, tentative, action et `process_run_uuid` ont des identités séparées. Au plus une tentative active par invocation et une invocation active par agent ; chaque tentative porte `authority_generation`, `daemon_epoch` et un fence monotone.
4. Les workers d’invocation sont des processus enfants Rust du daemon, sans clé fournisseur ni connexion d’écriture. Leur continuation ne survit pas au daemon : à la reprise, l’invocation passe par `reconciling`, puis une nouvelle tentative.
5. PTY et services longs sont tenus par un `unleash-process-host` par ressource, parent direct de l’enfant. La continuité est limitée au même boot ; après reboot, rien n’est ressuscité.
6. Le host ne signale que son enfant direct non réapé et le groupe dont cet enfant est leader, tant qu’il n’est pas réapé ; signaux et réapage sont sérialisés par le seul handle propriétaire (pidfd sous Linux lorsqu’il est qualifié). Le daemon ne signale jamais un PID observé ; après perte du host : IdentityUnverified et réconciliation par observation. Crates de référence `portable-pty` et `rustix`, versions verrouillées au M0.
7. La compaction installe une nouvelle tête de session par compare-and-swap de révision ; l’`initial_context_manifest_id` d’un bundle n’est jamais réécrit.

H-03 reste une hypothèse réfutable, amendée en R7 ([hypothèses](../../brief/assumptions.md)). Détail : [session-model](../session-model.md).

## Conséquences, complexité et impact V1

Stockage et rétention à gérer ; pas de reprise bit à bit du modèle ni résurrection PTY après reboot. Ancrage de signal par host parent non réapé, qualifié en M0. Coût modéré, nécessaire F003/F011/F018.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Worker/daemon crash, PID reuse, compaction et lecture après disparition.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../session-model.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

