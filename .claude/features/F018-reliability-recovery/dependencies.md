# Dépendances F018

| Dépendance | Contrat nécessaire |
|---|---|
| [F001](../F001-core-daemon/README.md) | Instance exclusive, lifecycle dont le mode `read_only_recovery` qui porte l'exclusion de maintenance d'une restauration, statut diagnostic et aucune admission prématurée. |
| [F002](../F002-persistence-events/README.md) | État/journal/receipt atomiques avec lignée/tombstones, pins et suppressions exclusives, backups protégés, capsule de restore, réservations et erreurs précises. |
| [F003](../F003-agent-runtime/README.md) | Checkpoints et bundles durables, tentative/fence distincts, capacité de continuation révocable. |
| [F011](../F011-terminal-pool/README.md) | Identité OS, manifests de hosts, logs durables et protocole de signalisation. |
| Adaptateurs Git/API/documents | Stratégie de réconciliation par action, receipt/identifiant distant et limites de répétition. |
| Sécurité/AI-Guard frontière locale | Identité du décideur, autorisation actuelle, révocation et audit ; frontière Unleash décidée par Q-04 ([ADR-10](../../architecture/adr/ADR-10-ai-guard-boundary.md)), protocole réel sous gate EXT-GUARD. |

La sécurité et la fiabilité se complètent : un résultat ancien peut être conservé comme preuve sans être autorisé à muter l'état actuel. Le coordinateur ne délivre pas de nouvelles permissions simplement parce qu'une reprise est urgente.

Les [sources primaires](../../research/systems-sources.md) documentent SQLite, processus et outils de test ; elles ne certifient pas le plan de recovery d'Unleash. Les comportements fournisseur de recherche de requête, facturation et idempotence doivent être vérifiés dans leur domaine. En leur absence, le résultat reste unknown avec disposition guidée V1.

Les backups exigent une politique d'objets et de worktrees. Une sauvegarde complète peut coûter plus qu'un snapshot DB ; le manifeste doit préciser les exclusions sans les transformer en couverture supposée. La validation macOS/Linux nécessite infrastructure de panne isolée, pas des essais destructifs sur les projets de l'utilisateur.

Les clients GUI/CLI/TUI conservent authority_generation et command_id d'origine pendant les retries ; le handshake ne remplace aucune enveloppe persistée. La reprise doit pouvoir lire la file refusée et son lien à l'inspection historique. L'adaptateur filesystem fournit des synchronisations qualifiées et des observations d'incarnation, sans prétendre qu'une transaction SQLite englobe suppression/copie. Un delta post-snapshot indisponible exige une disposition de couverture inconnue, pas une dépendance distante supposée capable de retrouver tous les effets.
