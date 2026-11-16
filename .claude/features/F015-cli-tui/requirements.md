# F015 — exigences

Statut : exigences de conception de la V1 complète, non implémentées. Identifiants 001 à 008 conservés ; compléments 009 à 016 ajoutés pour la clarification de parité complète (R2), puis 017 (R3–R5). Chaque ID possède un critère, un test et une preuve de même suffixe.

| Exigence | Règle | Critère |
| --- | --- | --- |
| REQ-CLI-001 | Assurer la parité fonctionnelle complète GUI/TUI des dix espaces de la matrice ; exposer les opérations métier en CLI non interactive. Même backend, identité, permissions, révisions, résultats et preuves ; aucun écran ou fonction exclusivement GUI. | AC-F015-001 |
| REQ-CLI-002 | Produire JSON/NDJSON versionné et stable sans messages humains ni séquences terminal mélangés ; stdout pour données, stderr pour diagnostics. | AC-F015-002 |
| REQ-CLI-003 | Distinguer commande acceptée, travail terminé et issue incertaine ; retrouver le reçu par command_id sans dupliquer l’effet. | AC-F015-003 |
| REQ-CLI-004 | Utiliser expected_revision, négocier le protocole et appliquer les mêmes décisions d’autorité que la GUI ; aucune écriture métier directe dans la base. | AC-F015-004 |
| REQ-CLI-005 | Quitter, détacher ou fermer CLI/TUI sans arrêter les missions, processus ou daemon ; une interruption métier exige une commande explicite à cible visible. | AC-F015-005 |
| REQ-CLI-006 | Permettre au clavier tous les parcours, la recherche, les conversations et formulaires ; préserver focus, sélection, brouillons et contexte au redimensionnement. Souris facultative, aucun geste exclusivement souris. | AC-F015-006 |
| REQ-CLI-007 | Inspecter logs et attacher PTY selon permissions, lease d’écriture et process_run_uuid ; isoler saisie PTY, chat et raccourcis ; neutraliser séquences hostiles dans les vues textuelles. | AC-F015-007 |
| REQ-CLI-008 | Documenter codes de sortie, attente, annulation, pagination, stdin fermé, absence de TTY et erreurs ; aucun prompt caché dans un script. | AC-F015-008 |
| REQ-CLI-009 | Fournir une application plein écran cohérente : dix espaces, arborescences/tableaux, panneaux, inspecteur, fil d’Ariane, palette de commandes, aides contextuelles, formulaires et états vide/chargement/erreur. | AC-F015-009 |
| REQ-CLI-010 | Lire et éditer intégralement SKILL, WORKFLOW et rôles dans la TUI : texte, champs structurés, étapes/branches/bindings, diff, validation, simulation/évaluation applicable et publication ; déprécier/révoquer les skills et rôles selon leur contrat. L’éditeur externe est une préférence facultative. | AC-F015-010 |
| REQ-CLI-011 | Explorer contexte autorisé/sélectionné/transmis, appels et compactions, snapshots, documents, preuves et provenance ; proposer preview textuelle/tableaux/arborescence et accès au contenu intégral selon droits, avec limites de rendu explicites. | AC-F015-011 |
| REQ-CLI-012 | Gérer fournisseurs, modèles, secrets référencés, connecteurs, scopes, limites, politiques et AI-Guard depuis Settings en TUI ; initier/suivre/annuler/reprendre l’authentification externe supportée sans lancer la GUI. | AC-F015-012 |
| REQ-CLI-013 | Détecter capacités terminal et fournir modes compact, sans couleur, ASCII et lecture linéaire ; qualifier clavier, Unicode et technologies d’assistance sans supposer une accessibilité automatique de Ratatui. | AC-F015-013 |
| REQ-CLI-014 | Maîtriser le cycle de vie terminal : entrée/sortie raw/alternate screen, curseur, souris, paste, focus ; restauration sur sortie normale, erreur, panique et signaux interceptables, puis réinitialisation après suspension/reprise. | AC-F015-014 |
| REQ-CLI-015 | Virtualiser listes/arbres, paginer contenu et logs, borner files et caches, appliquer backpressure et maintenir la réactivité sous charge ; l’I/O métier ne bloque pas le rendu. | AC-F015-015 |
| REQ-CLI-016 | Rétablir la connexion par snapshot/cursor, dédupliquer et traiter ResyncRequired ; afficher fraîcheur et statut des commandes, préserver les brouillons sans soumettre silencieusement des mutations. | AC-F015-016 |

La [matrice de parité](../../design/gui-tui-parity.md) est normative pour REQ-CLI-001. Une adaptation de rendu ne supprime aucune opération. Une impossibilité technique observée devient un écart à résoudre et interdit de déclarer la parité complète ; elle n’est pas renommée « fonction avancée GUI ».

Une action acceptée, une action accomplie et une preuve disponible restent trois états distincts. Une capacité fournisseur indisponible est représentée de façon identique par les trois clients ; aucun ne peut la fabriquer ni étendre l’autorité.


## Compléments R3–R5

| Exigence | Règle vérifiable | Critère |
| --- | --- | --- |
| REQ-CLI-017 | Offrir en TUI le même démarrage guidé, les templates, données d’automatisation, modes replay/supervised, supervision et configuration locale/cloud ; exposer les opérations en CLI structurée sans prompts cachés. | AC-F015-017 |
