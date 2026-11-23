# F006 — Tests prévus

| Test | AC | Jeu de tests | Preuve attendue |
|---|---|---|---|
| T-F006-001 | AC-F006-001 | Intention merge-only, ambiguïté de cible, contexte interdit. | EV-F006-001 : drafts et manifests, hypothèses conservées. |
| T-F006-002 | AC-F006-002 | Corpus JSON malformé, doublons, limites taille/profondeur. | EV-F006-002 : diagnostics et bornes de consommation. |
| T-F006-003 | AC-F006-003 | Références manquantes, IDs répétés, slots fixes/tardifs. | EV-F006-003 : table de symboles et erreurs attendues. |
| T-F006-004 | AC-F006-004 | Matrice types/portées, branches et frames de boucle. | EV-F006-004 : acceptations/refus sémantiques. |
| T-F006-005 | AC-F006-005 | Multiplication de bornes et valeurs frontières. | EV-F006-005 : calcul exact ou BoundOverflow. |
| T-F006-006 | AC-F006-006 | Injection eval, réseau, path escape, grants augmentés. | EV-F006-006 : aucune exécution et refus ciblés. |
| T-F006-007 | AC-F006-007 | Ordre de clés différent, même contenu, fixture merge-only. | EV-F006-007 : octets canoniques et diff explicite. |
| T-F006-008 | AC-F006-008 | Simulations succès/échec/inconnu et budget d'exploration. | EV-F006-008 : couverture déclarée, aucun effet réel. |
| T-F006-009 | AC-F006-009 | Courses publication/édition/validation et doublons. | EV-F006-009 : versions immuables, conflit, receipt réutilisé. |
| T-F006-010 | AC-F006-010 | Correction incrémentale avec ancien rapport conservé. | EV-F006-010 : filiation des drafts et diagnostics stables. |

Les tests de fuzzing doivent cibler le parseur borné, les références et les calculs de profondeur sans déclencher de ressource réseau. Les tests de propriétés comparent compilation répétée à catalogue figé. Une future campagne de qualité NL évaluera séparément fidélité d'intention et taux de clarification ; elle ne pourra déclarer l'intention toujours correctement comprise. Les contrôles JSON de ce dossier ne constituent que validation documentaire préalable.

Pour T-F006-009, envoyer la commande de [publication](../../examples/ipc-publish-workflow-version.json) après validation du brouillon. Vérifier le refus sans rapport, sans empreinte ou avec un rapport lié à une autre révision. Refaire l’essai avec un document dont l’identité ou la version diverge, puis avec un dernier numéro qui épuise la borne de `CreateRun`. Vérifier l’absence de version ajoutée après chaque refus.

À version précédente identique, deux commandes concurrentes produisent une seule nouvelle version et un conflit pour l’autre commande. Le rejeu de la commande gagnante rend le même reçu. Relire la version exacte, ses octets et son digest, puis l’utiliser pour un nouveau run. Le run déjà engagé doit conserver sa version. La cohérence des fixtures et le test de la transaction IPC ont des preuves distinctes.
