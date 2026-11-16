# F014 — tests et erreurs

**Révision : 16 novembre 2026. Tous ces tests sont spécifiés, non exécutés.** Les tests du domaine et des contrats Rust emploient des fixtures déterministes. Des outils d’automatisation de présentation/desktop existants sont admis, quelle que soit leur langue ; leur usage ne déplace pas le métier. Les tests de plateforme complètent les doubles sans se substituer aux assertions mécaniques. Aucune preuve produit n’est revendiquée dans ce dossier.

| Test | Critère et exigence | Cas limite / injection | Evidence attendue |
| --- | --- | --- | --- |
| T-F014-001 | AC-F014-001 ; REQ-UI-001 | Jeu complet et cas vide par écran. | EV-F014-001 : assertions, journal causal, versions et artefacts |
| T-F014-002 | AC-F014-002 ; REQ-UI-002 | Commandes concurrentes et expected_revision périmée. | EV-F014-002 : assertions, journal causal, versions et artefacts |
| T-F014-003 | AC-F014-003 ; REQ-UI-003 | Historique volumineux et agents archivés. | EV-F014-003 : assertions, journal causal, versions et artefacts |
| T-F014-004 | AC-F014-004 ; REQ-UI-004 | Perte connexion après acceptation du message. | EV-F014-004 : assertions, journal causal, versions et artefacts |
| T-F014-005 | AC-F014-005 ; REQ-UI-005 | Rétention, permissions et payload natif opaque. | EV-F014-005 : assertions, journal causal, versions et artefacts |
| T-F014-006 | AC-F014-006 ; REQ-UI-006 | Process non coopératif et effet OutcomeUnknown. | EV-F014-006 : assertions, journal causal, versions et artefacts |
| T-F014-007 | AC-F014-007 ; REQ-UI-007 | Contrat invalide, permissions excessives et révision périmée. | EV-F014-007 : assertions, journal causal, versions et artefacts |
| T-F014-008 | AC-F014-008 ; REQ-UI-008 | Gap de rétention et restauration backup, dont une restauration avec commande en vol : refus StaleFence/STALE_AUTHORITY_GENERATION, zéro effet, enveloppe d’origine conservée et aucune réémission avec la nouvelle lignée. | EV-F014-008 : assertions, journal causal, versions et artefacts |
| T-F014-009 | AC-F014-009 ; REQ-UI-009 | Variantes claire/sombre, erreurs, textes longs et petite fenêtre. | EV-F014-009 : assertions, journal causal, versions et artefacts |
| T-F014-010 | AC-F014-010 ; REQ-UI-010 | VoiceOver et Orca (matrice d’ADR-14, amendement R7), zoom 200 %, IME japonais/chinois, caractères accentués/RTL. | EV-F014-010 : assertions, journal causal, versions et artefacts |
| T-F014-011 | AC-F014-011 ; REQ-UI-011 | Backpressure, gros artefact et terminal rapide. | EV-F014-011 : assertions, journal causal, versions et artefacts |
| T-F014-012 | AC-F014-012 ; REQ-UI-012 | Parcourir toute la matrice GUI/CLI/TUI, dont éditeurs, Settings, connecteurs, contexte et médias ; droits refusés, révision périmée, contenu non textuel et terminal sans médias. | EV-F014-012 : matrice complète, traces normalisées par opération, états/reçus/erreurs comparés et parcours sans GUI |
| T-F014-013 | AC-F014-013 ; REQ-UI-013 | Fermer dernière fenêtre, quitter l’hôte puis tuer séparément WebView/hôte pendant jobs et après commit avant reçu ; daemon lancé par service utilisateur réel. | EV-F014-013 : identités de service/hosts, chronologie d’événements, preuve de progression sans GUI et de reprise sans effet supplémentaire |
| T-F014-014 | AC-F014-014 ; REQ-UI-014 | Fenêtre non autorisée, commande inconnue, scope usurpé, faux actor_ref, taille/débit excessifs, HTML/Markdown/SVG/URL/log hostile et secret canari. | EV-F014-014 : inventaire de capacités, CSP effective, refus corrélés, absence de secret/exécution/exfiltration et contrôle du build de production |

## Procédure de validation

Préparer une configuration reproduisible et une graine pour les scénarios concurrents. Vérifier l’état initial ; déclencher l’action et la panne au point nommé ; observer reçu, effets et état durable ; redémarrer si nécessaire ; vérifier absence de duplication et présence de limites explicites. Une réussite doit être liée au commit réellement testé.

Pour les interfaces, enregistrer les parcours clavier et observations visuelles avec captures sans secrets. Pour la sécurité, utiliser canaris et environnements de test dédiés. Pour les systèmes externes, conserver la version de contrat et les capacités réellement négociées. Ne jamais déclarer le gate externe passé sur la seule base d’une réponse de mock.


## Recette croisée de parité — T-F014-012

Partir de fixtures réinitialisées au même état et avec la même identité. Exécuter chaque opération de la matrice depuis GUI, CLI puis TUI ; comparer données métier, permissions, transition, erreurs, révision et contenu sémantique des reçus. Normaliser seulement les identifiants générés et horodatages propres à chaque exécution, sans masquer un événement ou un effet supplémentaire. Compléter par des sessions simultanées : initier dans un client, observer/intervenir dans un autre, vérifier conflits et idempotence. Le test refuse une ligne manquante, une commande réservée à la GUI ou une alternative qui nécessite d’ouvrir celle-ci.

Pour graphe/document/média, vérifier l’accès à la structure, au texte disponible, à la provenance et aux actes avec terminal sans rendu média. Une description/transcription absente reste explicitement absente. L’égalité attendue est fonctionnelle et informationnelle ; aucune assertion de pixels identiques n’est utilisée.

## Durabilité et erreurs — T-F014-013

Tester fermeture normale puis crash de l’hôte complet, pas seulement de la WebView. Le daemon doit rester joignable par CLI/TUI ; les jobs acceptés continuent selon leur contrat et aucune commande d’arrêt n’est créée. Après réouverture, retrouver l’état depuis snapshot/cursor et résoudre les command_id incertains sans réémettre avec un nouvel ID. Distinguer arrêt explicite du daemon, panne du daemon et perte du frontend dans les assertions. Conserver les versions d’OS, configuration de service et processus réellement testés.

## Sécurité du frontend — T-F014-014

Avec des comptes et canaris de test, inspecter l’inventaire des commandes/capabilities effectivement livré, pas seulement un fichier source. Depuis une fenêtre sans droit, tenter une commande applicative enregistrée ; vérifier rejet avant effet et contrôle serveur même si le renderer forge ses champs. Tester séparément autorité et scope, débits, tailles, références de ressources, identités et protocoles incompatibles. Les payloads invalides doivent conserver les erreurs communes et ne produire aucun faux reçu accepté.

Injecter les contenus hostiles dans conversations, preuves, titres, logs et documents. Vérifier qu’ils demeurent inertes, ne chargent pas de ressource distante et n’appellent pas l’IPC. Examiner DOM, stockage WebView, événements et diagnostics pour prouver l’absence de canaris de secret. Tester le parcours de saisie native/authentification contrôlée et sa parité CLI/TUI ; une simple redaction visuelle après passage du secret dans le DOM ne satisfait pas le critère. Vérifier la CSP effective du build livré et l’absence de serveurs/plugins de test et de permissions inutiles. Vérifier l’absence de `unsafe-eval`, de `wasm-unsafe-eval` et de toute origine distante dans la CSP effective.

## Outillage et limites de plateforme

La documentation [Tauri WebDriver](https://v2.tauri.app/develop/tests/webdriver/), v2 consultée le 16/11/2026, décrit un service WebDriver embarqué pour macOS/Linux/Windows et une voie `tauri-driver` native directe limitée à Linux/Windows. Qualifier la voie adaptée à chaque plateforme de la matrice d’ADR-14 ; ne pas recopier une ancienne affirmation d’impossibilité globale sur macOS. Vitest couvre la logique de présentation ; les parcours utilisent la voie WebDriver Tauri qualifiée par plateforme ([ADR-03](../../architecture/adr/ADR-03-rust-gui.md)). Un test de renderer avec IPC simulé couvre la présentation ; il ne prouve ni le packaging Tauri, ni les capabilities effectives, ni le daemon. Les parcours clavier/lecteur d’écran et la revue visuelle réelle restent requis.

## Tests supplémentaires R3–R5 — à exécuter

| Test | Critère / exigence | Méthode et oracle | Preuve attendue |
| --- | --- | --- | --- |
| T-F014-015 | AC-F014-015 / REQ-UI-015 | Recette avec utilisateurs novices selon le protocole et les seuils R7 du [parcours guidé](../../design/guided-automation-experience.md) (Q-20), interruptions chaque étape, absence de modèle en replay, permissions refusées ; captures et erreurs observées. | EV-F014-015 : fixtures, journal causal, compteurs, résultats et limites |
| T-F014-016 | AC-F014-016 / REQ-UI-016 | Comparaison des modes/couts/manifests ; fixture clic réussi mais produit incorrect ; local proxy classifié inconnu ; matrice de parité et preuves. | EV-F014-016 : fixtures, journal causal, compteurs, résultats et limites |

### Parcours croisés R6 — T-F014-015

Sauvegarder une préparation avant création projet/goal, fermer GUI et retrouver en TUI sans cache/ID. Deux clients appliquent la même révision avec command_id distincts : un seul application_id et mêmes objets, zéro StartRun/modèle implicite. Draft appliqué en lecture seule ; copie explicite pour un autre travail. Corriger une quantité invalide avec formulaire intégré, annuler puis importer/valider : nouvelle révision partagée, original et run engagé inchangés, zéro effet avant validation. Preuves de produit attendues, tests not_run.
