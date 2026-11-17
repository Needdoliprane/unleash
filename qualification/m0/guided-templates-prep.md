# Préparation Q-20 — Catalogue des 6 templates guidés et protocole de mesure (M0)

**Date : 17 novembre 2026 — Statut : fiche préparatoire M0 (`not_run`, aucun essai utilisateur ni exécution UI réalisé).**
Références : `.claude/design/guided-automation-experience.md`, `.claude/architecture/client-operation-catalog.md`, `.claude/features/F004-hierarchical-orchestration/interfaces.md`, `.claude/examples/*.json`.

---

## 1. Inventaire des 6 templates intégrés Q-20

Les six templates intégrés sont livrés en lecture seule dans le scope global (avec ID, version et digest), exposés par `ListGuidedTemplates`, et personnalisables par copie dans un scope projet. La recommandation initiale par `ListGuidedTemplates` est purement déterministe et n'appelle aucun modèle :

1. **`template.reconcile-csv` est recommandé par défaut sur un profil vierge** (aucun dépôt Git, aucun agent préexistant, aucune clé fournisseur requis).
2. Sans fournisseur modèle actif, les templates `forbidden` sont classés en tête et les templates nécessitant un fournisseur affichent explicitement leur prérequis manquant.
3. Sans dépôt Git enregistré, `template.dev-merge-only` et `template.dev-feature` restent accessibles mais ne sont pas recommandés par défaut.

| # | ID du template | Usage | Référence workflow / opération | Profil DSL & `model_policy` | Prérequis déterministes | Statut par défaut (profil vierge) |
|---|---|---|---|---|---|---|
| 1 | `template.reconcile-csv` | Rapprocher deux exports CSV (`GEN-03`) | `demo.reconcile.csv` (v1) | `unleash.workflow.v1.1`, `forbidden` (`max_llm_calls=0`, `max_agent_executions=0`) | Aucun dépôt, aucun agent, aucun fournisseur ; owner humain | **Recommandé par défaut** |
| 2 | `template.replay-rows` | Rejouer une procédure publiée ligne par ligne avec un nouveau CSV (`GEN-02/06`) | `fixture.rows-replay` (v1) | `unleash.workflow.v1.2` (`for_each`, `max_items=10000`, `batch_size=1000`), `forbidden` | Procédure publiée éligible ; pour une procédure navigateur, runtime navigateur préparé | Disponible si procédure publiée présente |
| 3 | `template.supervised-report` | Générer un rapport avec revue bornée par bloc (`GEN-05`) | `demo.supervised.report` (v1) | `unleash.workflow.v1.1` + binding `supervisor` (`role.automation.supervisor`), `allowed_bounded` | Fournisseur modèle actif vérifié (`max_llm_calls=1`, `max_output_tokens=2048`, `max_cost_microusd=1_000_000`) | Prérequis fournisseur affiché si absent |
| 4 | `template.learn-browser` | Préparer une procédure navigateur depuis une démonstration ou description (`GEN-01/02`) | Job de conception `LearnAutomation` sous mandat | Draft candidat (pas un `AutomationRun` publié) | Fournisseur modèle actif + runtime navigateur préparé (`Chrome for Testing` + `chromedriver`), budget conception `2 USD` (`2_000_000` microusd) | Prérequis fournisseur + runtime affichés |
| 5 | `template.dev-merge-only` | Développer une modification puis intégrer sans test/audit imposé | `workflow.merge-only` (v1) | `unleash.workflow.v1`, bindings `developer` + `integrator` | Dépôt Git enregistré + fournisseur actif | Non recommandé sans dépôt enregistré |
| 6 | `template.dev-feature` | Développer deux branches en parallèle, revue N+1, puis intégrer | `workflow.feature` (v1) | `unleash.workflow.v1`, bindings `developer` + `leader` + `integrator` (`late`) | Dépôt Git enregistré + fournisseur actif | Non recommandé sans dépôt enregistré |

---

## 2. Schémas d'entrée et fixtures synthétiques prévues par template

### 2.1 `template.reconcile-csv` (défaut profil vierge)
- **Entrées attendues** :
  - `source_left_csv` (`resource_ref | project_file | client_upload`, UTF-8, séparateur `,` ou `;` explicite)
  - `source_right_csv` (`resource_ref | project_file | client_upload`)
  - `data_revision_id` (résolu par `ImportAutomationData` avant effet)
  - `join_key` (`string`, défaut `"invoice_id"`, préservant les zéros initiaux)
  - `amount_unit` (`enum`: `"minor_currency_unit"` par défaut — unités mineures entières + devise)
  - `duplicate_policy` (`enum`: `"block"` par défaut)
- **Fixture synthétique prévue** : deux exports CSV synthétiques de factures (identifiants avec zéros initiaux type `"0012"`, montants entiers en centimes `EUR`, variante négative à dernière ligne invalide et cellule commençant par `=` vérifiant la non-exécution de formules).
- **Complétion** : `/results/compare/outputs/report_complete == true` (mécanique, zéro appel modèle).

### 2.2 `template.replay-rows`
- **Entrées attendues** :
  - `automation_version_ref` (`artifact_id` + `version` publiée immuable + digest)
  - `data_revision_ref` (CSV validé par `ValidateAutomationData`, tableau `/input/rows` borné `≤ 10 000`)
  - `batch_size` (`integer`, défaut `1000`, `1..=10000`)
  - `browser_runtime_ref` (optionnel, obligatoire si profil navigateur)
- **Fixture synthétique prévue** : jeux de `0`, `1`, `100`, `101` et `300` lignes synthétiques (ex. panier synthétique `SYNTHETIC-APPLE-1KG`, `SYNTHETIC-RICE-1KG`) sur serveur marchand local + coupure C4/C5 vérifiant qu'une ligne `outcome_unknown` n'avance jamais le curseur ni `after_batch`.

### 2.3 `template.supervised-report`
- **Entrées attendues** :
  - `data_revision_id` (`string`, référence vers révision immuable)
  - `provider_profile_ref` (fournisseur vérifié par `TestProviderProfile`)
  - `max_llm_calls` (`1`), `max_cost_microusd` (`1_000_000`), `max_output_tokens` (`2048`)
- **Fixture synthétique prévue** : observation d'export synthétique (`demo.data.observe`) suivie d'un unique appel de revue `leader_decision` renvoyant tour à tour `continue` (avec `safe_to_continue=true, effect_state="none"`), `continue` contredisant `safe_to_continue=false` (refusé par le choice/broker) et `escalate`.

### 2.4 `template.learn-browser`
- **Entrées attendues** :
  - `intent_description` (`string`, objectif utilisateur sans jargon workflow)
  - `allowed_origins` (`array<origin>`, liste fermée pour `EgressProxy`)
  - `sample_data_revision_ref` (données synthétiques d'exemple optionnelles)
  - `browser_runtime_ref` (`Chrome for Testing` + `chromedriver` préparés)
  - `design_budget_microusd` (`integer`, défaut `2_000_000`, soit `2 USD`)
  - `provider_profile_ref` (fournisseur actif pour `LearnAutomation`)
- **Fixture synthétique prévue** : parcours sur site marchand synthétique local produisant un `AutomationDraft` candidat non publié avec locators `RoleName`/`TestId` et assertions déterministes (aucune publication implicite, aucune transmission de droits d'apprentissage au replay).

### 2.5 `template.dev-merge-only`
- **Entrées attendues** :
  - `repo_ref` (dépôt Git enregistré)
  - `change_request` (`string`, description du changement)
  - `base_snapshot` (référence de base)
  - `target_ref` (`string`, ex. branche gérée non checkout selon `Q-10`)
  - `provider_profile_ref` (fournisseur actif)
- **Fixture synthétique prévue** : dépôt Git temporaire local (`$TMPDIR`) avec branche cible non checkout et variante `blocked_target_checkout` (branche checkout dans un worktree refusée selon `Q-10`).

### 2.6 `template.dev-feature`
- **Entrées attendues** :
  - `repo_ref` (dépôt Git enregistré)
  - `api_change_request` (`string`), `ui_change_request` (`string`)
  - `base_snapshot` (`string`)
  - `target_ref` (`string`, branche cible non checkout)
  - `provider_profile_ref` (fournisseur actif)
- **Fixture synthétique prévue** : dépôt Git temporaire local exerçant les deux tâches parallèles (`develop-api`, `develop-ui`), l'arbitrage `leader-check` (`accept` vs rejet `LEADER_REJECTED`) et l'intégrateur tardif borné (`git.inspect`, `git.integrate`, `registry.search`).

---

## 3. Protocole de mesure du premier parcours (`GATE-GUIDED` — prévu M5)

- **Cohorte** : 5 participants non développeurs en GUI et 3 participants en TUI, sur macOS de référence, depuis un profil vierge, avec modération et export local consenti (zéro télémétrie produit).
- **Scénario évalué** : démarrage avec `template.reconcile-csv` puis 6 tâches : (1) choisir les bonnes sources, (2) corriger une erreur CSV via copie de travail client (`ImportAutomationData`), (3) distinguer préparation (`CreateGuidedSetupDraft`/`ApplyGuidedSetup`) et exécution (`StartRun`), (4) repérer une ligne « Résultat à confirmer » (`OutcomeUnknown`), (5) retrouver la preuve d'une action, (6) adapter une procédure sans écraser l'historique.
- **Critères de sortie bloquants (`GATE-GUIDED`)** :
  - `≥ 4/5` (GUI) et `≥ 2/3` (TUI) achèvent le rapprochement sans aide.
  - Médiane `≤ 10 min` (GUI) et `≤ 15 min` (TUI) du profil vierge au résultat affiché.
  - `100 %` identifient correctement l'absence/usage de modèle et repèrent la ligne « Résultat à confirmer ».
  - `≥ 4/5` corrigent le CSV invalide et retrouvent la preuve d'une action.
  - `0` effet externe, probe ou chargement modèle lors du simple aperçu (`GetSetupPreview`).
  - Parité exacte des objets, reçus et événements backend entre GUI et TUI (SUS cible indicatif `≥ 70`).

**Statut global Q-20 à la sortie de `t09` : `not_run` (fiche préparatoire prête pour M5).**
