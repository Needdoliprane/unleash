# F015 — tests et erreurs

Statut : plan de tests futur, aucune preuve produit exécutée dans ce dossier. Les tests spécifiques à Unleash et les doubles de contrat sont en Rust ; la recette de terminal réel complète les assertions mécaniques. Les tests ne se limitent pas aux captures de widgets.

| Test | Critère et exigence | Scénario / injection | Preuve attendue |
| --- | --- | --- | --- |
| T-F015-001 | AC-F015-001 ; REQ-CLI-001 | Les 58 groupes PAR de la matrice, succès et refus/conflit sur mêmes fixtures/identités ; GUI/TUI/CLI reliés au même backend. | EV-F015-001 : résultat par PAR, événements/effets/erreurs comparés, parcours réels. |
| T-F015-002 | AC-F015-002 ; REQ-CLI-002 | Accents, retours ligne, gros payload paginé, watch interrompu, erreur fournisseur et pipeline. | EV-F015-002 : schémas, stdout/stderr capturés, aucun ANSI ou prompt. |
| T-F015-003 | AC-F015-003 ; REQ-CLI-003 | Timeout avant/après commit, replay même clé, payload modifié avec même clé, inspection depuis autre client ; restauration entre la première tentative et le replay → StaleFence/STALE_AUTHORITY_GENERATION, zéro effet, intention conservée avec son refus, aucune commande réémise avec la lignée annoncée par Hello. | EV-F015-003 : un effet, reçu original, OutcomeUnknown explicite. |
| T-F015-004 | AC-F015-004 ; REQ-CLI-004 | Major incompatible, champ réponse inconnu, expected_revision périmée, acteur falsifié, scope refusé. | EV-F015-004 : refus cohérents et aucune écriture hors façade. |
| T-F015-005 | AC-F015-005 ; REQ-CLI-005 | Quitter/détacher, fermeture fenêtre terminal, reconnexion, mission/processus toujours observables. | EV-F015-005 : identités, états durables, aucune mutation d’arrêt implicite. |
| T-F015-006 | AC-F015-006 ; REQ-CLI-006 | Tout clavier, resize au milieu d’un formulaire/chat, recherche globale, retour inspecteur et souris absente. | EV-F015-006 : parcours, focus et brouillons préservés, cible d’action stable. |
| T-F015-007 | AC-F015-007 ; REQ-CLI-007 | ANSI/OSC hostiles, stdout binaire, lecteur lent, PID réutilisé, lease perdu et révocation pendant saisie. | EV-F015-007 : sortie assainie, autorisation/lease, génération exacte, aucun raccourci métier involontaire. |
| T-F015-008 | AC-F015-008 ; REQ-CLI-008 | Daemon absent, quota/denied, stdin/pipe fermé, non-TTY, wait/timeout et annulation métier explicite. | EV-F015-008 : codes documentés, aucune attente interactive cachée, état final inspectable. |
| T-F015-009 | AC-F015-009 ; REQ-CLI-009 | Dix espaces en états vide/chargement/partiel/erreur, palette, panneaux, texte long et navigation causale. | EV-F015-009 : snapshots de widgets et captures terminal 120×40/80×24, revue des parcours. |
| T-F015-010 | AC-F015-010 ; REQ-CLI-010 | Sans éditeur externe : SKILL multiligne, rôle composé, WORKFLOW avec branches/bornes/bindings ; invalidité, diff et publication concurrente. | EV-F015-010 : texte/AST, diagnostics, simulation/évaluation, versions immuables et reçus. |
| T-F015-011 | AC-F015-011 ; REQ-CLI-011 | Contexte multi-appels/fallback/compaction ; doc inaccessible, rétention, branche non intégrée et média sans rendu. | EV-F015-011 : manifests et provenance, contenu/export, absence de secret et limites annoncées. |
| T-F015-012 | AC-F015-012 ; REQ-CLI-012 | Fournisseur invalide, scopes insuffisants, navigateur indisponible, consentement refusé/expiré, coffre verrouillé, guard offline. | EV-F015-012 : parcours de configuration et services réels, états/erreurs et canaris secrets. |
| T-F015-013 | AC-F015-013 ; REQ-CLI-013 | Matrice de terminaux de la [conception terminal](../../design/cli-tui-design.md#matrice-terminal-qualifiée-v1) (ADR-14, amendement R7), tmux ≥ 3.4 local/SSH, lecteurs VoiceOver/Orca, sans couleur/souris, ASCII, Unicode large/combiné, clavier international et lecteur d’écran. | EV-F015-013 : matrice de capacités et recette d’assistance avec limites explicites. |
| T-F015-014 | AC-F015-014 ; REQ-CLI-014 | Sortie normale, init partielle, erreur, panique, SIGINT/SIGTERM interceptables, Ctrl+Z (suspension pilotée), SIGSTOP/SIGCONT externes (réinitialisation seule) et éditeur facultatif échoué. | EV-F015-014 : modes terminal avant/après et shell réutilisable ; frontière SIGKILL consignée. |
| T-F015-015 | AC-F015-015 ; REQ-CLI-015 | Charge de référence ci-dessous ; rafale d’événements, énorme log, scroll/filtre/resize et backend lent. | EV-F015-015 : p95, pages chargées, caches/files bornés, backpressure et usage CPU au repos observé. |
| T-F015-016 | AC-F015-016 ; REQ-CLI-016 | Gap de rétention, nouveau stream_id, coupure après message accepté, modification GUI concurrente et brouillon hors ligne. | EV-F015-016 : reçu retrouvé, snapshot/cursor, déduplication, conflit et brouillon intact. |

## Charge et budgets proposés pour T-F015-015

La recette fixe avant exécution machine, versions OS/terminal, backend, jeu de données et mesure. Cible de conception : 20 leaders, 100 000 objets historiques navigables, un million de lignes de logs côté service et un artefact de 10 MiB accessible par fragments. Ces volumes ne sont pas tous chargés dans le client.

Budgets proposés : pages de listes de 200 éléments au plus ; buffer d’affichage des logs de 10 000 lignes et 16 MiB au maximum (première borne atteinte) ; cache de contenu client limité à 64 MiB ; file de notifications de présentation limitée à 1 024 entrées avec resynchronisation explicite en débordement. Le journal backend reste la vérité : coalescer des rafraîchissements n’autorise pas à perdre les événements métier.

Cible locale : retour visuel d’une frappe/navigation p95 ≤ 100 ms sous la charge nominale ; l’attente de donnée distante affiche immédiatement un état de chargement et se mesure séparément. Mesurer RSS et CPU au repos, sans boucle de redraw permanente. Ces valeurs sont des critères proposés, pas des performances mesurées ; une modification exige une justification tracée et ne peut rendre la virtualisation facultative.

## Procédure de validation

1. Verrouiller contrats, build, version des bibliothèques et fixtures ; appliquer la matrice de terminaux de la [conception terminal](../../design/cli-tui-design.md#matrice-terminal-qualifiée-v1) (ADR-14, amendement R7).
2. Vérifier état initial et identité commune ; jouer chaque ligne PAR en GUI puis TUI et CLI, en réinitialisant la fixture pour comparer les effets, pas leurs IDs de transport aléatoires.
3. Injecter panne/refus/conflit au point nommé ; observer commande, reçu, événements, état durable, contenu et preuve. Comparer les identités métier, révisions et invariants.
4. Capturer parcours clavier, rendu, états dégradés et restauration terminal sans secrets. TestBackend peut couvrir le rendu déterministe ; il ne prouve pas comportement d’un émulateur, lecteur d’écran ou fournisseur.
5. Mesurer charge et bornes ; vérifier aucune mutation silencieuse pendant reconnexion et aucun arrêt mission en quittant.
6. Publier les EV avec résultats, environnement et limites. Une capacité requise manquante reste un échec de parité ; un résultat de mock reste une preuve de contrat uniquement.

OAuth et connecteurs se testent sur comptes de qualification autorisés ; leur réussite réelle est distincte du test déterministe de présentation. Les tests sécurité ciblent mécanisme, périmètre et canaris : autorité backend, isolation de projet, secret absent des arguments/logs, contrôle de lease et neutralisation de séquences actives.


## Tests supplémentaires R3–R5 — à exécuter

| Test | Critère / exigence | Méthode et oracle | Preuve attendue |
| --- | --- | --- | --- |
| T-F015-017 | AC-F015-017 / REQ-CLI-017 | Fixtures de setup, fichiers de paramètres invalides, terminal compact et nonTTY ; comparer commandes, reçus, effets et refus aux parcours GUI. | EV-F015-017 : fixtures, journal causal, compteurs, résultats et limites |

### Parcours croisés R6 — T-F015-003/017

Sauvegarder une préparation avant création projet/goal, fermer GUI et retrouver en TUI sans cache/ID. Deux clients appliquent la même révision avec command_id distincts : un seul application_id et mêmes objets, zéro StartRun/modèle implicite. Draft appliqué en lecture seule ; copie explicite pour un autre travail. Corriger une quantité invalide avec formulaire intégré, annuler puis importer/valider : nouvelle révision partagée, original et run engagé inchangés, zéro effet avant validation. Preuves de produit attendues, tests not_run.
