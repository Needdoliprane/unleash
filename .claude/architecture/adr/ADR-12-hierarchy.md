# ADR-12 — Frontières N+2 / N+1 / N

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : trois niveaux exacts, commandes typées, mailbox et borne de non-progression fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Autonomie organisationnelle et technique ne doivent pas créer plusieurs moteurs concurrents ni de microgestion.

Exigences : REQ-ORCH REQ-AGENT. Risques/hypothèses/questions : R-03 R-19 R-23 ; H-03 H-04.

## Options et compromis

Option recommandée : **N+2 staffing/rapports, N+1 décision technique, N exécution ; harness seul contrôle mécanique**.

Alternatives considérées : Manager unique pour chaque microtâche ; équipes sans hiérarchie.

Le partage réduit coût de contexte ; manager unique devient goulot ; équipes autonomes non bornées compliquent arbitrage et autorité.

## Décision

Mandats versionnés, scope/budgets et événements causaux. N+2 direct N autorisé sous mission spécialisée ; N ne redélègue pas librement. Messages humains N+1 visibles N+2 sans wake systématique.

## Amendement R7

1. Trois niveaux exactement. N+2 : goals, organisation, arbitrages entre missions, registre de skills, création de N+1 et de N directs sous mission spécialisée. N+1 : mission de bout en bout, expertise du domaine, jugement OK/KO/correction/escalade. N : tâche sous rôle et contrat. Aucun quatrième niveau ; un N ne crée ni agent ni mission et adresse une demande typée à son owner.
2. Les choix des agents passent uniquement par des commandes typées (AssignMission, ReviseMission, CreateAgent, BindRole, AssignTask, AcceptStep, RequestCorrection, RequestAgentReassignment, RecordMissionDecision, RecordGoalDecision). Le harness valide et exécute seul les mutations et le travail mécanique.
3. Une mission peut avoir un OwnerRef utilisateur et s’exécuter sans agent actif, sans ajouter de niveau. Les leaders dormants sont réveillés par une mailbox durable ; un message humain au N+1 est visible au N+2 (visible, à traiter, traité, ignoré avec motif) sans réveil systématique.
4. DecisionProgressPolicy versionnée : `max_no_progress_turns=8` par défaut, configurable de 1 à 100, compteur durable par `decision_chain_id`. Au seuil : DecisionStalled et notification humaine déterministe, sans LLM.
5. Les invocations N+1/N+2 occupent le même pool que N ([ADR-13](ADR-13-scheduler-budget.md)) ; sous `forbidden`, aucune n’est admise.

## Conséquences, complexité et impact V1

Qualité sémantique du leader dépend du modèle/skill. Coûts de management mesurés ; boucle bornée par DecisionProgressPolicy (8 tours sans progrès par défaut). Responsabilité skills N+2 peut déléguer conception technique.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

A/C/I/J, boucle staffing, mailbox et résultats périmés.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/agent-hierarchy.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

