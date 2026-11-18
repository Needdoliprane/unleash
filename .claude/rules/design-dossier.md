---
paths:
  - ".claude/brief/**"
  - ".claude/architecture/**"
  - ".claude/specs/**"
  - ".claude/features/**"
  - ".claude/design/**"
  - ".claude/schemas/**"
  - ".claude/examples/**"
  - ".claude/catalog/**"
  - ".claude/testing/**"
  - ".claude/plans/**"
---

# Règles de modification du dossier de conception

Le dossier `.claude/` est la référence de réalisation. Le code peut révéler une erreur de contrat : on corrige alors le dossier, explicitement, dans le même goal.

- **Autorité.** Les invariants (`brief/invariants.md`) ne changent que sur décision de Louis/OVPT. Les ADR acceptés, schémas et contrats canoniques (voir `README.md`, « Autorités canoniques ») priment sur les résumés, features et audits.
- **Propagation complète.** Un changement de contrat met à jour, dans le même changement : ADR ou contrat canonique, schéma, fixtures, feature(s) concernée(s), tests attendus, catalogue (`catalog/*.json`, `testing/traceability.*`) et résumés qui le citent. Les audits restent historiques : on n'y réécrit pas l'histoire, on ajoute une révision.
- **Projections.** `catalog/design-catalog.json`, `catalog/operations.json` et `testing/traceability.json` doivent rester identiques à leurs sources Markdown. Une exigence ajoutée a son REQ, AC, T et EV, dans la feature et dans les deux projections.
- **Régénération et contrôle.** Après toute modification du dossier :

  ```sh
  python3 tools/design-package/regen.py .
  python3 tools/design-package/validate.py .
  ```

  `regen.py` réécrit le manifest et `CHECKSUMS.sha256` ; `validate.py` doit finir sur `0 errors`. Si une exigence, une question, un ADR, une hypothèse ou un risque a changé, lancer d'abord `python3 tools/design-package/sync.py .`, qui reprojette les sources Markdown dans `catalog/design-catalog.json` et `testing/traceability.*`. Ne jamais éditer le manifest ou les checksums à la main.
- **Statuts de preuve.** `product_test_status` reste `not_run` dans le dossier ; les preuves réelles vont dans `qualification/evidence/` à la racine du dépôt (voir [règles de test](testing.md)).
- Les journaux de travail temporaires sous `.claude/evidence/` sont exclus de l’export de conception et de ses checksums.
- **Style.** Français, phrases normatives courtes, identifiants stables. Un identifiant supprimé n'est jamais réutilisé.
