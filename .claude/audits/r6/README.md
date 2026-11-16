# R6 — Quatorze expertises déléguées et corrections

**Revue documentaire réellement déléguée à quatorze instances d’agents spécialisés.** Date : 16 novembre 2026. Chaque instance a reçu un mandat ciblé, a lu les sources et a produit son rapport distinct. Il s’agit de revues par agents IA, pas de quatorze professionnels humains ni de modèles dont on prétendrait des identités indépendantes. Le coordinateur consolide ; les propositions restent soumises aux statuts du mandat utilisateur.

Les douze premières revues ont précédé leurs corrections ; des relectures ciblées ont ensuite contrôlé les raccords. E13 conduit la contradiction après rédaction et E14 vérifie couverture/arbitrages de produit. Les rapports gardent l’objection initiale ; ce registre indique la correction de conception actuelle. Les tests de produit restent not_run.

## Rapports attribués

| Expert | Rapport |
| --- | --- |
| E01 — Chief Software Architect | [expert-01-chief.md](expert-01-chief.md) |
| E02 — Rust Systems Architect | [expert-02-rust.md](expert-02-rust.md) |
| E03 — Workflow & Deterministic Execution | [expert-03-workflow.md](expert-03-workflow.md) |
| E04 — Multi-Agent Systems | [expert-04-multi-agent.md](expert-04-multi-agent.md) |
| E05 — LLM API & Context | [expert-05-llm.md](expert-05-llm.md) |
| E06 — Git & Integration | [expert-06-git.md](expert-06-git.md) |
| E07 — Knowledge & Code Intelligence | [expert-07-knowledge.md](expert-07-knowledge.md) |
| E08 — Skill & Contract Systems | [expert-08-skills.md](expert-08-skills.md) |
| E09 — Reliability & Persistence | [expert-09-reliability.md](expert-09-reliability.md) |
| E10 — Security / AI-Guard | [expert-10-security.md](expert-10-security.md) |
| E11 — UX / GUI / TUI | [expert-11-ux.md](expert-11-ux.md) |
| E12 — QA & Verification | [expert-12-qa.md](expert-12-qa.md) |
| E13 — Independent Adversarial Reviewer | [expert-13-adversarial.md](expert-13-adversarial.md) |
| E14 — Technical Product Architect | [expert-14-product.md](expert-14-product.md) |

## Registre des corrections

Les identifiants et relations sont aussi disponibles dans [findings.json](findings.json). « Corrigé en conception » signifie que le contrat et sa recette ont été précisés, pas que le mécanisme Rust ou sa sécurité ont été prouvés. Les preuves de qualification restent celles des features.

| Finding | Priorité | Problème | Correction et référence | Vérification attendue |
| --- | --- | --- | --- | --- |
| R6-E01-01 | Élevée | Probe fournisseur impossible sans invocation agente | ModelOrigin et ProviderProbeContext ; même pool 5/50, données synthétiques et mandat authentifié. [Contrat](../../features/F012-model-api-router/interfaces.md) | T-F012-013/014/018 |
| R6-E01-02 | Moyenne | Noms d’opérations et contrats non structurés | Registre de 196 opérations, propriétaires/parité ; GetStateSnapshot distinct, enveloppe et payload distingués. [Contrat](../../catalog/operations.json) | DOC-ING-005 |
| R6-E02-01 | Bloquant invariant | Deux racines peuvent créer deux schedulers | Autorité utilisateur OS unique et rendez-vous indépendant de data_root ; changement après drainage. [Contrat](../../architecture/runtime-lifecycle.md) | T-F001-001 |
| R6-E02-02 | Bloquant invariant | Relecture PID puis kill conserve la course | Cible non réutilisable qualifiée ou IdentityUnverified ; groupe et handle individuel qualifiés séparément. [Contrat](../../architecture/process-supervision.md) | T-F011-002/006 |
| R6-E03-01 | Élevée | Curseur CSV et choix de ligne non définis | for_each borné v1.2, portée /items, cursor/result atomiques, after_batch explicite ; v1/v1.1 conservés. [Contrat](../../architecture/workflow-data-iteration.md) | T-F019-003/005/012 |
| R6-E03-02 | Élevée | Diagnostic publié inaccessible sous blocked | Microsteps et route diagnostique réduite admis sans lever la barrière ; reprise après résolution prouvée. [Contrat](../../architecture/state-machines.md) | T-F005-003/006 ; T-F019-001/011 |
| R6-E03-03 | Moyenne | F006 interdit un candidat de qualification permis par F019 | Exception qualification_only/purpose=evaluation explicite, refus du même snapshot en opérationnel. [Contrat](../../features/F006-workflow-compiler/dependencies.md) | T-F019-002/004/008 |
| R6-E04-01 | Élevée | Décision de clôture human/owner_agent sans commande | RecordMissionDecision/RecordGoalDecision, critères/owner/epoch/bundle/révision ; mechanical sans LLM. [Contrat](../../features/F004-hierarchical-orchestration/interfaces.md) | T-F004-008/011 |
| R6-E04-02 | Moyenne | Compteur sans progrès réinitialisable entre réveils | Chaîne persistante, réservations de tours, défaut huit, progrès observables et réarmement attribué. [Contrat](../../specs/agent-hierarchy.md) | T-F004-006 |
| R6-E05-01 | Élevée | Résumé perd les restrictions de sa source | Politique transitive des dérivés, provenance et déclassification autorisée ; context v2. [Contrat](../../specs/model-routing.md) | T-F012-009/015 |
| R6-E05-02 | Élevée | Contexte distant implicite contredit les exclusions locales | Historique explicite, continuation distante refusée ; opaque qualifié et métadonnées de rétention. [Contrat](../../architecture/session-model.md) | T-F012-010/011 |
| R6-E05-03 | Moyenne | Préférence modèle skill non représentée | ModelSelectionBinding externe versionné exact, résolution par scope et tombstone. [Contrat](../../features/F012-model-api-router/interfaces.md) | T-F012-002 |
| R6-E06-01 | Élevée | Symref peut déplacer une cible non mandatée | Refs directes requises, mutations sans déréférencement et cas extérieur réconcilié. [Contrat](../../architecture/git-coordination.md) | T-F010-001/006/011 |
| R6-E06-02 | Élevée | refs/replace change une lecture sous OID inchangé | Profil brut commun sans replacements, version/digest conservés et preuves antérieures revalidées. [Contrat](../../specs/git-integration.md) | T-F010-004/008/009 |
| R6-E07-01 | Élevée | DiscoveryReceipt exige une invocation pour un humain | Origines invocation/replay/control_plane exclusives et authentifiées. [Contrat](../../schemas/discovery-receipt.schema.json) | T-F009-005/008 |
| R6-E07-02 | Moyenne | Discovery sans Git n’a pas de provenance commune | TypedSourceRef Git, document, fichier et registry, cohérence/ACL/révisions explicites. [Contrat](../../architecture/knowledge-indexing.md) | T-F009-001/011 |
| R6-E07-03 | Moyenne | La propre création périme sa preuve préalable | DiscoveryUseObservation historique séparée de current_freshness et completion_revalidation. [Contrat](../../specs/shared-knowledge.md) | T-F009-005/008 |
| R6-E08-01 | Élevée | Lien claim/preuve sans prédicat vérifiable | Vocabulaire fermé versionné et claim_predicate_matches ; prose et inconnu ne deviennent pas vrais. [Contrat](../../schemas/contract-claim.schema.json) | T-F008-004/007 |
| R6-E08-02 | Élevée | Un test favorable peut remplacer la commande attendue | Attente autoritaire et binding scellés avant effet, argv/cwd/env/suite/parser/snapshot vérifiés. [Contrat](../../schemas/expected-execution-binding.schema.json) | T-F008-005 |
| R6-E09-01 | Bloquant invariant | Retry client réexécutable après rollback | IPC v2 avec authority_generation, refus avant idempotence, tombstones et capsule des effets/coûts. [Contrat](../../architecture/failure-recovery.md) | T-F018-002/007 ; T-F002-004 |
| R6-E09-02 | Élevée | Attachement concurrent pendant GC/unlink | Pins et writer, deleting/tombstone, incarnations physiques distinctes et barrière de backup. [Contrat](../../architecture/storage-model.md) | T-F002-006/010 ; T-F018-002 |
| R6-E10-01 | Élevée | Allow conserve des contraintes sans les imposer | Obligations typées imposées avant secret/dispatch ; inconnue ou impossible refuse. [Contrat](../../specs/ai-guard.md) | T-F017-005/008/010 ; T-SEC-10 |
| R6-E10-02 | Moyenne | Réauthentification peut changer le principal du compte | ProviderPrincipalBinding et auth_generation, mismatch suspend accès/Watch et conserve provenance. [Contrat](../../specs/connectors.md) | T-F013-007/009 |
| R6-E11-01 | Moyenne | Draft guidé sauvegardé introuvable dans un autre client | GetOverview expose les préparations y compris avant projet, sous ACL et pagination. [Contrat](../../features/F004-hierarchical-orchestration/interfaces.md) | T-F014-015 ; T-F015-017 |
| R6-E11-02 | Moyenne | Deux Apply de même draft peuvent dupliquer le travail | Application unique par draft, résultat réutilisé, draft appliqué figé et clone explicite. [Contrat](../../features/F004-hierarchical-orchestration/interfaces.md) | T-F004-012 ; T-F015-003/017 |
| R6-E11-03 | Moyenne | Correction d’une valeur CSV sans interaction intégrée | Formulaire/copie locale, diff, nouvelle révision puis validation intégrale ; mêmes fonctions TUI. [Contrat](../../design/guided-automation-experience.md) | T-F019-005 ; T-F014-015 ; T-F015-017 |
| R6-E12-01 | Élevée | Validation accepte omissions/doublons/fausses preuves | Comparaison bidirectionnelle sources/projections, arêtes exactes, statuts et code d’échec ; formats sondés. [Contrat](../../testing/document-validation.md) | DOC-ING-002/003/004 |
| R6-E12-02 | Moyenne | Sous-preuves E2E H/N omettent la reprise | Dépendances obligatoires F018/F003/F002 explicites ; preuves partielles ne clôturent pas la recette. [Contrat](../../testing/integration-tests.md) | AC-E2E-H/N |
| R6-E12-03 | Moyenne | Pression WAL sans oracle discriminant | Lecteur retenu + écritures + export, quotas et pression visibles, reprise sans faux receipt. [Contrat](../../features/F002-persistence-events/tests.md) | T-F002-002/007 |
| R6-E13-01 | Élevée | Validation du package peut résoudre un $ref distant | Scan des références avant validation, registre URN local et retrieve refusant, sentinelle sans réseau. [Contrat](../../specs/document-package-ingestion.md) | DOC-ING-006/007 |

## Arbitrages et limites préservés

Le dossier conserve les désaccords sur la frontière de migration, le choix GUI, l’isolation macOS et AI-Guard. Les contrats d’itération, d’origine sans agent, de provenance et de reprise précisent les garanties atteignables sans ajouter un juge LLM obligatoire ni une architecture distribuée. Le calendrier automatique Q-21 reste une option non confirmée.

> **Note R7 (16 novembre 2026) :** ces arbitrages sont rendus : Q-02, Q-03, Q-04 et Q-05 sont tranchées et Q-21 est écartée de la V1 ([révision R7](../../brief/revision-2026-11-16-r7.md)). Les renvois de recette de R6-E05-01, R6-E05-02, R6-E05-03, R6-E08-01, R6-E09-02 et R6-E12-03 ont été recoupés avec les sections R6 des features et corrigés dans ce registre et dans [findings.json](findings.json). Les résidus de ces corrections sont repris dans les contrats canoniques : la décision de déclassification de R6-E05-01 est soumise par `RecordDeclassificationDecision` ([catalogue client](../../architecture/client-operation-catalog.md)) ; GetStateSnapshot appartient à F001/F002, distinct de GetSnapshot (F009), et le décompte de 196 opérations de R6-E01-02 est celui de R6 ([registre courant](../../catalog/operations.json)) ; AC-F017-005/008/010 portent les compléments de R6-E10-01 ([acceptation F017](../../features/F017-ai-guard-integration/acceptance.md)) ; DiscoveryReceipt v1 désigne la seule forme corrigée de R6-E07-01 ([schémas](../../schemas/README.md)) ; `provider_probe` (R6-E01-01), le profil v1.2 (R6-E03-01) et le pipeline IPC v2 (R6-E09-01, même classe que R6-REL-01) sont propagés dans les résumés qui les omettaient.

Le contrôle documentaire constate schémas, fixtures, références, relations et contre-exemples ; il ne mesure ni latence runtime, ni capacité GPU, ni compatibilité réelle OAuth/AI-Guard, ni qualité d’une automatisation sur un compte réel. Ces gates ne disparaissent pas dans le verdict. La revue finale et [la validation](../../testing/document-validation.md) donnent les résultats effectivement observés.

## Contrelecture de remise

Une instance distincte a ensuite relu les raccords prioritaires et l’ingestion. [R6-REL-01](release-readthrough.md) corrige l’ordre de contrôle lignée/idempotence dans le résumé du moteur de workflows, puis a été relu. Il complète les trente constats des quatorze expertises sans être compté comme un nouveau test produit.
