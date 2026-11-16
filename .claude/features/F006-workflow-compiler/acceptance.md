# F006 — Acceptation

| AC | Given / When / Then |
|---|---|
| AC-F006-001 | Étant donné une intention partiellement ambiguë, quand le modèle produit le draft, alors hypothèses et champs non résolus restent visibles et le contexte envoyé est inspectable ; aucun champ critique inventé ne permet le démarrage. |
| AC-F006-002 | Étant donné entrée trop profonde, duplicate key ou champ inconnu, quand elle est compilée, alors un diagnostic localisé apparaît dans la borne de ressources et aucune publication n'est créée. |
| AC-F006-003 | Étant donné références absentes, dupliquées ou slots tardifs incomplets, quand la validation s'exécute, alors chaque problème est attribué au bon nœud ; un slot tardif contraint valide reste autorisé. |
| AC-F006-004 | Étant donné lecture d'une branche concurrente ou nombre comparé à une chaîne, quand le typage s'applique, alors la définition est refusée ; une lecture après jointure avec type compatible passe. |
| AC-F006-005 | Étant donné boucles imbriquées proches des limites, quand la borne est calculée, alors overflow et dépassement sont refusés ; le rapport donne le calcul et les budgets distincts. |
| AC-F006-006 | Étant donné script, référence distante ou capacité excessive, quand le draft est validé, alors aucun code ni réseau n'est exécuté et un refus précis est retourné. |
| AC-F006-007 | Étant donné le même draft et catalogue snapshot, quand la compilation est répétée, alors bytes/digest/diagnostics sont identiques ; Dev → Merge ne reçoit aucune étape de test/audit. |
| AC-F006-008 | Étant donné branches alternatives, quand la simulation explore un sous-ensemble borné, alors rapport mentionne simulation, fixtures et branches non couvertes ; aucun outil réel n'est invoqué. |
| AC-F006-009 | Étant donné deux publications concurrentes et un run v1 actif, quand v2 est publiée, alors aucune version n'est écrasée et le run reste v1 ; le doublon identique reçoit le même receipt. |
| AC-F006-010 | Étant donné un diagnostic corrigeable, quand l'utilisateur ou N+1 modifie le draft, alors intention et historique restent conservés et seules les erreurs résolues disparaissent du nouveau rapport. |

L'acceptation combine tests de structure, de sémantique, de sécurité et de concurrence. Elle ne se limite pas à la génération d'un JSON syntaxiquement valide. Le scénario B du mandat requiert une version réellement publiée et inspectable dans l'implémentation future ; un objet généré dans une conversation ne suffira pas.
