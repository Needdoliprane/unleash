# Tests transversaux des contrats

## Statut et objet

Ce plan vise la future implémentation Rust. Les contrôles documentaires des JSON joints ne prouvent ni comportement du daemon ni autorisation effective. Le plan relie [F007](../features/F007-skill-studio-registry/tests.md) et [F008](../features/F008-contract-engine/tests.md), avec F004/F005 pour décisions et transitions. Chaque campagne conserve version de binaire, contrat, validateur, fixture, snapshot et environnement expurgé.

## Corpus minimal

| Cas | Données préparées | Résultat exigé |
|---|---|---|
| CT-01 Forme vraie, contenu faux | Claim observable `test.expectation_satisfied`=true lié au binding scellé, observation exit 1 ; variante en prose qualitative | Structure pass ; `claim_predicate_matches` fail/ClaimContradicted pour le claim typé, unknown pour la prose ; clôture refusée si règle obligatoire |
| CT-02 Zéro cas | Processus exit 0, parseur connu, zéro test alors que minimum 1 | Test observation fail, sans changer exit observé |
| CT-03 Snapshot périmé | Test valide commit A, sujet commit B ou dirty snapshot différent | EvidenceSnapshotMismatch ou unknown explicite |
| CT-04 Preuve inventée | Texte de log produit par modèle sans action collectée | Artefact déclaré, jamais observation authentifiée |
| CT-05 Révocation | Binding épinglé puis grant retiré | Prochain effet refusé, bundle ancien intact |
| CT-06 Auto-escalade | N+2 modifie outils/ACL déclarée au-delà de ses grants | Publication ou action refusée selon phase ; aucun grant nouveau |
| CT-07 Discovery absent | Création identifiable et contrat exigeant recherche | Opération structurée refusée avant effet (ContractViolation/DISCOVERY_REQUIRED) ; création constatée à la clôture : ContractViolation/DISCOVERY_EVIDENCE_MISSING ; aucune clôture normale |
| CT-08 Create justifié | Reçu antérieur, similaire nonmergé, domaine incompatible documenté | Contrôle de trace pass, jugement qualitatif distinct, pas mutualisation forcée |
| CT-09 Reçu hors scope | Recherche dans autre repo ou après création | Refus ou dérogation distincte si politique l'autorise ; jamais rétrodatage |
| CT-10 Import hostile | `$ref` distant, chemin sortant, script et prose injonctive | Aucune I/O de validation, aucun script exécuté, diagnostics bornés |
| CT-11 Publication sans juge | Fixtures positives/négatives, politiques compatibles | Publication possible sans second modèle ; rapport limité à ce qui est vérifié |
| CT-12 Preuve purgée | Verdict ancien puis purge autorisée de sa preuve | Tombstone, impossibilité de nouveau pass fondé sur contenu absent |
| CT-13 Race révision | Bundle assemblé à r, sujet modifié à r+1 | StaleEvidence ou RevisionConflict à l'acceptation |
| CT-14 Slot tardif | Nouvelle compétence compatible pour slot vierge, autre slot engagé | Nouveau binding append-only permis ; substitution engagée refusée |
| CT-15 Input incompatible | Même rôle affiché mais digest input schema modifié | Binding refusé InputSchemaMismatch, aucune inclusion implicite |
| CT-16 Output incompatible | Booléen devenu chaîne ou schéma canonique différent | Binding refusé OutputSchemaMismatch ; descriptions similaires sans effet |
| CT-17 Règle retirée | Règle obligatoire supprimée, changée ou devenue informative | Binding refusé RequiredRuleChanged ; ajout conjonctif restrictif seul autorisable |
| CT-18 Entrée ambiguë | Deux skills, entry_skill absente ou non membre | Validation refusée AmbiguousEntrySkill ; aucune sélection au hasard |
| CT-19 Évaluateur remplacé | Crash puis nouveau catalogue/evaluator installé | Reprise avec versions épinglées ; blocage si anciennes indisponibles, jamais upgrade implicite |

## Isolation des garanties

Les doubles d'adaptateur produisent observations authentifiées fictives dans un environnement de test. Le rapport doit identifier cette simulation. Une campagne d'intégration ultérieure utilisera vrais processus et repositories temporaires pour vérifier collecte, snapshot et réconciliation. Les fixtures multilangages peuvent appeler les outils des projets ; leur harness interne est Rust. Les tests de sécurité ne doivent jamais porter sur des ressources utilisateur sans scope de test explicite.

Le corpus négatif doit varier séparément acteur, epoch, repo, commit, worktree, argv, cwd, outil, parseur et date d'observation. Cela vérifie la correspondance réelle des preuves au sujet. Des tests de propriété s'assurent qu'une preuve inaccessible ne fait pas apparaître un pass plus favorable, et qu'ajouter un claim ne transforme pas une observation d'échec en réussite.

## Critères de campagne

Chaque résultat identifie la règle et les preuves consultées. La collecte d'un secret dans une sortie doit déclencher redaction avant log ou contexte modèle selon politique. Les rapports de refus restent assez précis pour corriger le problème sans exposer des données hors ACL. Une panne du validateur ou un dépassement de budget produit unknown ou erreur, jamais pass par défaut.

Le contrôle de publication ne comprend aucun juge LLM obligatoire universel. Une évaluation sémantique ajoutée est déclarée comme jugement et testée séparément. Les assertions « aucun bug » ou « aucun doublon possible » ne figurent pas dans les résultats acceptables. Le niveau de preuve doit rester proportionné à la propriété effectivement vérifiée.
