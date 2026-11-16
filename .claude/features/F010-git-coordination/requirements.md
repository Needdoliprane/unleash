# F010 — Exigences

Toutes les exigences sont V1. Les conditions de sécurité et de cohérence technique ne deviennent pas des étapes d'audit métier imposées au workflow. Une définition de done mentionnant une cible précise doit être satisfaite sur cette cible.

| ID | Obligation testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-GIT-001 | L'adoption doit identifier dépôt, format d'objet, capacités Git, worktrees, opérations en cours et disponibilité des cibles avant allocation. | AC-F010-001 | T-F010-001 | EV-F010-001 |
| REQ-GIT-002 | Chaque allocation doit relier mission, branche/worktree et propriétaire de mutation ; deux écrivains non coordonnés ne doivent pas être admis dans le même worktree. | AC-F010-002 | T-F010-002 | EV-F010-002 |
| REQ-GIT-003 | Une mutation médiée doit vérifier scope, epoch et préconditions de contenu ; toute extension de scope doit être explicite. | AC-F010-003 | T-F010-003 | EV-F010-003 |
| REQ-GIT-004 | Tout candidat d'intégration doit être préparé dans un espace dédié à partir d'OID épinglés avec stratégie et validations effectives. | AC-F010-004 | T-F010-004 | EV-F010-004 |
| REQ-GIT-005 | Une lease de cible doit porter un epoch contrôlé par le broker ; expiration et processus encore actif doivent déclencher réconciliation avant réattribution. | AC-F010-005 | T-F010-005 | EV-F010-005 |
| REQ-GIT-006 | La publication doit utiliser une comparaison oldOID/newOID, refuser une cible déplacée ou checkout non transférée et conserver le motif. | AC-F010-006 | T-F010-006 | EV-F010-006 |
| REQ-GIT-007 | Un conflit textuel doit créer un état et un mandat d'intégration spécialisés, sans résolution automatique globale `ours/theirs`. | AC-F010-007 | T-F010-007 | EV-F010-007 |
| REQ-GIT-008 | Les validations sémantiques exécutées doivent être celles du workflow effectif ; une fusion sans tests/audit demandés doit rester autorisée et déclarée non vérifiée sur ces aspects. | AC-F010-008 | T-F010-008 | EV-F010-008 |
| REQ-GIT-009 | Toute action Git à effet doit enregistrer son intention puis son résultat observé, avec réconciliation explicite de la fenêtre crash et sans répétition aveugle. | AC-F010-009 | T-F010-009 | EV-F010-009 |
| REQ-GIT-010 | Un rebase ou remplacement de source doit préserver la provenance des anciens OID et invalider dépendances, reçus discovery et preuves concernés. | AC-F010-010 | T-F010-010 | EV-F010-010 |
| REQ-GIT-011 | Le retrait d'un worktree ou d'une branche gérée doit préserver le travail dirty et les artefacts requis, vérifier les processus et appliquer une politique de rétention explicite. | AC-F010-011 | T-F010-011 | EV-F010-011 |
| REQ-GIT-012 | Les observations Git doivent gérer noms de chemins atypiques, liens, sous-modules, dépôts sans commit et formats d'objet qualifiés, puis émettre les événements de réconciliation idempotents. | AC-F010-012 | T-F010-012 | EV-F010-012 |

## Conditions de lecture

REQ-GIT-006 protège la valeur comparée au moment de l'effet ; elle ne transforme pas Git externe en participant à la lease. L'inventaire de checkouts réduit le risque dans le périmètre coopératif sans promettre une transaction atomique avec un processus local hostile. REQ-GIT-009 distingue « état souhaité observé » et « effet attribué avec preuve à cet essai ».

REQ-GIT-008 ne retire pas les préconditions techniques d'une commande : autorisation, intégrité du candidat, comparaison de cible et preuve de résultat restent nécessaires. Il interdit d'ajouter une validation de qualité universelle non demandée sous un nom technique. Le contrat peut demander des tests précis ; dans ce cas leurs résultats doivent porter le candidat effectivement publié.
