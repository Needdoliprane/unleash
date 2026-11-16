# F014 — exigences

Statut : règles proposées, révisées le 16 novembre 2026 après clarification de la stack et de la parité complète. Identifiants REQ-UI-001 à 011 conservés ; compléments 012 à 014 (R2) puis 015/016 (R3–R5) ajoutés. Chaque ID a une correspondance directe avec acceptation, test et preuve.

| Exigence | Règle | Critère |
| --- | --- | --- |
| REQ-UI-001 | Fournir les dix espaces demandés et navigation depuis projet/goal/mission. | AC-F014-001 |
| REQ-UI-002 | Consommer exclusivement le backend commun pour toute règle métier. | AC-F014-002 |
| REQ-UI-003 | Séparer agents logiques, invocations actives, dormance, queue et attente. | AC-F014-003 |
| REQ-UI-004 | Afficher message N+1 et sa visibilité N+2 sans prétendre à une lecture LLM. | AC-F014-004 |
| REQ-UI-005 | Montrer le contexte effectivement assemblé/transmis et sa provenance. | AC-F014-005 |
| REQ-UI-006 | Différencier pause, reprise, interruption, réaffectation et arrêt de processus. | AC-F014-006 |
| REQ-UI-007 | Lire intégralement, éditer, comparer et publier skills/rôles/workflows via backend. | AC-F014-007 |
| REQ-UI-008 | Rattraper sans doublon et informer de l’état périmé. | AC-F014-008 |
| REQ-UI-009 | Présenter une GUI de produit fini selon le design system et critères visuels. | AC-F014-009 |
| REQ-UI-010 | Permettre clavier, focus visible, labels, contraste et saisie internationale. | AC-F014-010 |
| REQ-UI-011 | Borner mémoire et latences avec grandes listes et logs. | AC-F014-011 |
| REQ-UI-012 | Garantir la parité fonctionnelle complète GUI/CLI/TUI pour tous les espaces, actes, données, droits, erreurs et reçus ; fournir des représentations adaptées et accessibles aux contenus riches sans rendre un acte exclusivement graphique. | AC-F014-012 |
| REQ-UI-013 | Garder daemon et jobs indépendants de la GUI, y compris de son hôte Rust ; fermeture, crash et reconnexion ne déclenchent ni annulation ni répétition d’effets acceptés. | AC-F014-013 |
| REQ-UI-014 | Avec Tauri, passer par un adaptateur Rust au backend commun ; borner les commandes/capabilities/scopes, activer la CSP et neutraliser le contenu non fiable ; aucun socket, credential IPC ou secret fournisseur exposé à la WebView. | AC-F014-014 |

Les limites décrites dans les audits sont des frontières à exposer, pas des exceptions silencieuses aux critères. Une action acceptée, une action accomplie et une preuve disponible sont des états différents.


La [matrice de parité](../../design/gui-tui-parity.md) rend REQ-UI-012 vérifiable opération par opération. Elle comprend éditions, workflows, connecteurs, paramètres, contexte, preuves et interventions. La différence de rendu est permise ; l’omission d’un acte ou d’une information métier ne l’est pas. Une fonction indisponible chez un fournisseur est signalée avec la même raison par les trois clients.

Tauri 2 avec renderer React/TypeScript est la décision technique adoptée par [ADR-03](../../architecture/adr/ADR-03-rust-gui.md), pas un invariant utilisateur. Rust reste le cœur métier, HTML/CSS est autorisé et les dépendances spécialisées sont réutilisées. Un autre framework n’entre que par réexamen de l’ADR ; il doit satisfaire les mêmes exigences de résultat et des frontières de sécurité équivalentes, sans supprimer le gate REQ-UI-014 par changement d’étiquette.

## Compléments R3–R5

| Exigence | Règle vérifiable | Critère |
| --- | --- | --- |
| REQ-UI-015 | Fournir un démarrage guidé sans agent préconfiguré ni vocabulaire workflow obligatoire : objectif ou procédure recommandée, sources, paramètres, mode, modèle si nécessaire, périmètre, aperçu puis résultat. | AC-F014-015 |
| REQ-UI-016 | Présenter les automations, versions, données, replay/supervised, points de revue et modèles locaux/distants ; afficher séparément préparation, exécution et réparation avec compte réel d’appels et preuves. | AC-F014-016 |
