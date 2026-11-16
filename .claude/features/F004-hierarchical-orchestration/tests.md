# F004 — Plan de tests et preuves

Tests prévus pour l'implémentation Rust ; aucun résultat produit n'est revendiqué. Les fixtures utilisent un fournisseur déterministe fictif et un journal contrôlé. Les appels réels éventuels servent une campagne distincte et ne remplacent pas les tests de domaine.

| Test | AC couvert | Procédure essentielle | Preuve attendue |
|---|---|---|---|
| T-F004-001 | AC-F004-001 | Générer arbres valides puis mutations cycle/profondeur/double owner. | EV-F004-001 : états avant/après et erreurs sans mutation. |
| T-F004-002 | AC-F004-002 | Créer N direct ; retirer chaque précondition à tour de rôle. | EV-F004-002 : receipt et relations durables, refus atomiques. |
| T-F004-003 | AC-F004-003 | Barrer deux clients au même expected_revision et varier bundle. | EV-F004-003 : ordre du journal, un gagnant, conflit ou StaleEvidence. |
| T-F004-004 | AC-F004-004 | Vingt leaders, cinq N, urgence N+2 ; config 1/5/50/51. | EV-F004-004 : séries d'occupation, files et refus du plafond dépassé. |
| T-F004-005 | AC-F004-005 | Injecter crash avant/pendant/après commit et pièce non autorisée. | EV-F004-005 : atomicité mailbox/message, trace ACL expurgée. |
| T-F004-006 | AC-F004-006 | Rafale de logs, urgence, leader qui répète sans nouvelle preuve. | EV-F004-006 : coalescence, budget et arrêt NoProgressLimit. |
| T-F004-007 | AC-F004-007 | Réaffecter pendant action puis envoyer résultat ancien. | EV-F004-007 : epoch, refus nouveau side effect, incident de réconciliation. |
| T-F004-008 | AC-F004-008 | Terminer tâches/runs sans satisfaire tout le goal. | EV-F004-008 : états distincts et refus de clôture injustifiée. |
| T-F004-009 | AC-F004-009 | Discovery puis trois agents même rôle, scopes séparés ; slot tardif. | EV-F004-009 : reçus, bindings, manifests et absence de mélange de contexte. |
| T-F004-010 | AC-F004-010 | Simuler conflit Git puis mission d'intégration créée par N+2. | EV-F004-010 : chaîne arbitrage → mission → observation, auteurs attribués. |

La campagne de concurrence rejoue les événements dans plusieurs ordres légaux et vérifie l'invariant d'owner. La campagne de sécurité essaie faux acteur, scope étranger et epoch périmé. La campagne de panne conserve le journal minimal nécessaire à la reproduction sans secret. L'acceptation nécessite toutes les preuves de la table, la version du binaire et le snapshot du jeu de fixtures ; un résumé de modèle ne constitue pas un rapport de test.

## Tests supplémentaires R3–R5 — à exécuter

| Test | Critère / exigence | Méthode et oracle | Preuve attendue |
| --- | --- | --- | --- |
| T-F004-011 | AC-F004-011 / REQ-ORCH-011 | Fixture sans repository, serveur modèle piège, clôture mécanique puis humaine ; confronter journal et appels au router. | EV-F004-011 : fixtures, journal causal, compteurs, résultats et limites |
| T-F004-012 | AC-F004-012 / REQ-ORCH-012 | Doublon de setup, scope insuffisant, incident et modèle proposant retry interdit ; vérifier identité, grants, nombre d’appels et refus déterministe. | EV-F004-012 : fixtures, journal causal, compteurs, résultats et limites |

### Compléments de T-F004-006/008/011 — R6

Injecter un ping-pong de deux leaders avec nouveaux IDs, puis crash au tour 7 et redelivery : EV-F004-006 montre huit réservations distinctes, compteur conservé et zéro neuvième appel/escalade récursive. Injecter progression validée et réarmement humain explicite, vérifier les compteurs et budgets cumulés.

T-F004-008/011 exercent les deux commandes de clôture en GUI/TUI/CLI : succès humain sans agent, verdict agent mandaté, refus ancien epoch, critères manquants, effet inconnu et retransmission après reçu perdu. EV-F004-008/011 contient décisions, transaction état/événement/reçu et compteur modèle zéro pour human/forbidden. Ces preuves restent attendues (not_run).

### Parcours croisés R6 — T-F004-012

Sauvegarder une préparation avant création projet/goal, fermer GUI et retrouver en TUI sans cache/ID. Deux clients appliquent la même révision avec command_id distincts : un seul application_id et mêmes objets, zéro StartRun/modèle implicite. Draft appliqué en lecture seule ; copie explicite pour un autre travail. Corriger une quantité invalide avec formulaire intégré, annuler puis importer/valider : nouvelle révision partagée, original et run engagé inchangés, zéro effet avant validation. Preuves de produit attendues, tests not_run.
