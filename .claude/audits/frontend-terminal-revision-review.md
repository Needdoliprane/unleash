# Revue contradictoire ciblée R2 — Frontend et terminal

**Date : 16 novembre 2026. Auteur : sous-agent distinct `r2_independent_review`. Nature : revue documentaire indépendante des auteurs de la révision GUI, de la révision TUI et de la consolidation.** Cette attribution désigne un processus de relecture distinct, pas un expert humain supplémentaire. Aucun code produit n’a été écrit ou modifié par cette revue.

> **Note R7 (16 novembre 2026) :** les noms SpawnProcess, WriteTerminal et ReadProcessLog (R2-REV-04) ne sont des alias publics dans aucune version du protocole, y compris `unleash.ipc.v2` ([IPC](../architecture/ipc-and-apis.md)). Tauri 2 avec renderer React/TypeScript et Ratatui/Crossterm, présentés ci-dessous comme non adoptés, sont acceptés en R7 par arbitrage délégué ([ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)).

## Verdict et portée

**Avis favorable sur la cohérence documentaire du périmètre R2 relu, après corrections vérifiées. Aucun défaut bloquant non résolu n’a été identifié dans ce périmètre à la clôture.** La politique actuelle autorise la réutilisation de bibliothèques et HTML/CSS, conserve le cœur Rust et impose une TUI complète. Le dossier ne présente ni Tauri ni Ratatui/Crossterm comme un choix utilisateur définitivement adopté. Les opérations de création, édition, configuration, récupération et contrôle de terminal ne sont pas réservées à la GUI.

Ce verdict autorise à considérer la révision documentaire ciblée comme cohérente ; il ne vaut pas qualification du produit, preuve de faisabilité complète ou adoption des ADR. **Aucun build, test produit, benchmark, essai de sécurité, test OAuth ou parcours d’accessibilité n’a été exécuté par ce relecteur.** La validation globale des liens, de l’index, des matrices et de l’archive relève du rapport [de validation documentaire](../testing/document-validation.md), distinct de la présente revue.

La dernière clarification de Louis et la [politique d’implémentation](../brief/implementation-policy.md) sont l’autorité de la révision. Les sources primaires servent à vérifier les propriétés des bibliothèques ; elles ne peuvent adopter un choix à la place de l’utilisateur ni prouver leur intégration dans Unleash.

## Objections soulevées et corrections relues

Les constats ci-dessous ont été transmis au consolidateur, corrigés par lui, puis relus. La relecture n’a pas modifié les documents canoniques. Certaines divergences de noms existaient avant R2 ; le raccordement des interfaces les a rendues visibles et a permis de les résoudre avant implémentation.

| Repère | Objection et conséquence | Correction vérifiée | État |
| --- | --- | --- | --- |
| R2-REV-01 | L’ADR-03 qualifiait encore le cœur, la planification et la persistance Rust de simples propositions, alors que la politique et INV-001 les présentent comme contrainte confirmée. | [ADR-03](../architecture/adr/ADR-03-rust-gui.md), contexte : cœur Rust confirmé ; choix du moteur de présentation proposé. | Corrigé et relu |
| R2-REV-02 | Plusieurs formulations interdisaient seulement une « fonction clé » exclusivement GUI. Une lecture isolée pouvait laisser croire qu’une fonction secondaire échappait à la parité. | Qualification « clé » supprimée de la matrice, de F015 et du design terminal ; les critères continuent d’exiger toutes les opérations. | Corrigé et relu |
| R2-REV-03 | ExportBundle figurait parmi les requêtes tout en promettant un reçu et une préparation longue. Les clients pouvaient en déduire deux contrats incompatibles. | [Catalogue commun](../architecture/client-operation-catalog.md) : ExportBundle et ExportReport sont des commandes avec reçu ; ReadArtifact lit ensuite l’artefact. F016 décrit la même séparation et une destination client contrôlée. | Corrigé et relu |
| R2-REV-04 | Les noms terminal, rapports et récupération divergeaient entre catalogue et interfaces propriétaires, alors que le protocole annonçait une seule nomenclature publique. | [IPC](../architecture/ipc-and-apis.md), F011, F016 et F018 raccordés : AcquireTerminal/SendTerminalInput/ReadTerminalLog, CreateReportSnapshot/GetReportSnapshot, RestoreBackup/InspectRecovery. Les anciens noms sont explicitement des raccourcis historiques non acceptés sur le wire. | Corrigé et relu |
| R2-REV-05 | Le [parcours terminal](../design/interaction-flows.md) exigeait la création d’un projet, mais la matrice et la grammaire terminal ne proposaient que sa lecture/sélection. | PAR-OVERVIEW-01, F015/interfaces et catalogue incluent project create/revise et CreateProject/ReviseProject. | Corrigé et relu |
| R2-REV-06 | Le parcours après crash permettait une décision humaine de répétition ou d’abandon ; l’inventaire terminal ne nommait que l’inspection de la récupération. | PAR-SETTINGS-05 et F015 exposent recovery show/reconcile/resolve/resume, raccordés à InspectRecovery, ReconcileAction, ResolveActionOutcome et ResumeRecoveredInvocation. La disposition ne fabrique pas une preuve de succès antérieur. | Corrigé et relu |
| R2-REV-07 | Le resize PTY était exigé par les écrans et tests mais sans opération publique et sans préconditions explicites dans l’interface F011. | ResizeTerminal est nommé dans F011, IPC, catalogue et parcours terminal. Sa cible porte terminal_id/process_run_uuid, lease/fence et dimensions positives bornées. AcquireTerminal distingue allocation et acquisition d’une génération enregistrée sans relancement. | Corrigé et relu |
| R2-REV-08 | Après harmonisation du nom SubscribeEvents, F016 conservait un payload after_cursor tandis que l’IPC définissait stream_id et after_sequence. | F016 utilise maintenant les mêmes champs stream_id/after_sequence que l’IPC ; GetActivitySnapshot reste une projection de reprise distincte. | Corrigé et relu |
| R2-REV-09 | La sauvegarde de workflow exigeait un « document normalisé », ce qui pouvait empêcher de conserver le draft invalide pourtant exigé par les éditeurs. | Catalogue : SaveWorkflowDraft conserve la source exacte même invalide, sans exécution ; normalisation et validation restent explicites avant publication. La même conservation de draft s’applique aux skills et rôles. | Corrigé et relu |

Ces corrections portent sur des contrats et leur couverture. Elles ne constituent pas neuf tests du produit passés.

## Constats confirmés dans les documents

### Rust pragmatique et statut des technologies

La [politique](../brief/implementation-policy.md), les [clarifications](../brief/clarifications.md), [INV-001](../brief/invariants.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md), le workspace et F001 concordent : cœur métier et système Rust, bibliothèques existantes réutilisées, présentation HTML/CSS admissible. Le dossier ne demande plus de réécrire SQLite, un moteur graphique ou une dépendance système pour obtenir une chaîne transitive exclusivement Rust. La mention MySQL reste un exemple de réutilisation, sans changement silencieux du stockage retenu.

Kivy et Tauri sont distingués ; aucun texte relu ne présente Kivy comme une bibliothèque Rust ou comme un autre nom de Tauri. Tauri est candidat de référence, renderer Rust/WASM ou mince JS/TS à départager. Q-01 et Q-17 sont résolues dans le brief ; Q-02 reste le choix technique final. Ratatui/Crossterm est une recommandation à qualifier. Les versions documentaires observées ne sont pas décrites comme un lockfile compilé.

### Parité réelle dans le contrat de conception

La [matrice](../design/gui-tui-parity.md) couvre dix espaces et cinquante groupes d’opérations. Chaque verbe d’un groupe exige son propre cas de recette. Les sept documents de F015 imposent éditeurs intégrés de workflow, skills et rôles, formulaires, diff, publication, conversation, contexte, connecteurs, Settings, processus et récupération. L’installation d’un éditeur externe n’est pas nécessaire pour terminer les éditions requises.

Les adaptations de rendu sont explicites : graphe navigable sous forme d’arbre ou de relations, diff intégral dans des panneaux alternés, document structuré et média accompagné de ses métadonnées/contenus disponibles. Une transcription absente n’est pas inventée. L’accès à une source externe complète l’inspection ; il ne remplace pas les données et opérations que le produit doit exposer. Le navigateur de consentement OAuth peut être lancé depuis chaque interface sans imposer la GUI Unleash.

Les critères de F014 et F015 comparent les permissions, données, révisions, commandes, effets, reçus, erreurs et preuves sur un même backend. Les modes compact, ASCII, sans couleur et lecture linéaire ne dispensent pas de conserver les actions. Aucune réduction de la TUI à la supervision ou aux seules opérations essentielles n’a été trouvée dans le périmètre courant relu.

### Autorité, secrets et cycle de vie

La chaîne proposée est cohérente : WebView de présentation, adaptateur Rust restreint, client IPC commun, daemon indépendant. Aucun handler client n’est autorisé à devenir un second moteur de workflow ou à écrire directement dans SQLite. Le daemon revalide identité, mandat, révision et fencing.

La frontière des secrets est décrite comme un travail à réaliser : la WebView reçoit références et statuts, la saisie GUI utilise une surface de confiance à qualifier, et le transfert vers le coffre passe par un canal privé hors enveloppes journalisées. La TUI utilise une saisie masquée ou un canal protégé. Le catalogue nomme les erreurs et la destruction des tampons temporaires selon les capacités du système. Aucune API Tauri de saisie sécurisée prétendument existante n’est inventée.

La fermeture ou le crash de l’hôte GUI complet est distingué de l’arrêt explicite du daemon. F014 exige de vérifier la survie et la progression des jobs sur un service utilisateur réel ; le packaging sidecar seul n’est pas présenté comme preuve. F015 distingue détachement, libération du contrôle PTY, arrêt du processus et restauration des modes du terminal. La limite d’un SIGKILL ou d’une perte du terminal est explicite.

## Points qui restent à qualifier

Ces éléments sont des inconnues ou preuves futures déjà exposées par la conception, pas des défauts masqués par un verdict favorable.

| Sujet | Statut actuel | Preuve ou décision nécessaire |
| --- | --- | --- |
| Shell et renderer GUI | Tauri proposé ; alternative native conservée | Prototypes comparables, choix motivé, versions compatibles, coûts d’intégration et packaging |
| Composants terminal | Ratatui/Crossterm proposés ; éditeur et émulation PTY à qualifier | Bibliothèques réutilisables, Unicode, clavier, restauration, mémoire et charge réelles |
| Accessibilité et esthétique | Exigences et profils de recette définis | Parcours sur écrans et terminaux réels, lecteurs d’écran, IME et configurations supportées |
| Secrets et commandes WebView | Frontières et tests négatifs décrits | Implémentation du canal/saisie de confiance, inventaire effectif des commandes et permissions, CSP, canaris et injections |
| Contrats publics | Noms, propriétaires, arguments structurants et règles communes documentés | Schémas détaillés figés, handlers, fixtures de compatibilité et preuve de parité de chaque sous-opération |
| Daemon et processus | Durabilité requise ; limites OS déclarées | Service utilisateur macOS/Linux, fermeture/crash, reprise, génération/lease, politiques de restart et conditions de session |
| Services externes | Gates maintenus | Contrat et dépôt AI-Guard, comptes fournisseurs, OAuth réel, politiques de droits et révocation |

Le [plan M0–M7](../plans/v1-implementation-plan.md) et la [readiness checklist](../plans/readiness-checklist.md) maintiennent les décisions et preuves dans leurs jalons. Ils distinguent la permission utilisateur déjà acquise de réutiliser les bibliothèques et les vérifications techniques encore nécessaires. La revue ne fixe aucun délai ni résultat de performance absent.

## Vérifications documentaires effectivement exécutées

- Lecture croisée des documents listés ci-dessous, comparaison de leurs opérations et examen des corrections dans les fichiers après leur application.
- Recherche textuelle des interdictions Rust-only, questions Q-01/Q-17 présentées comme ouvertes, réductions de parité et anciens noms d’opérations. Les formulations historiques explicitement supersédées ne sont pas traitées comme des règles actuelles.
- Contrôle mécanique local : F014 possède les suffixes 001–014 dans REQ/AC/T/EV ; F015 les suffixes 001–016 ; aucun suffixe attendu absent. La matrice contient cinquante repères PAR distincts. Ce contrôle ne démontre pas la réussite d’un scénario.
- Contrôle ciblé des noms publics après harmonisation et des formes SubscribeEvents/export/draft invalide. Les anciens noms encore cités dans l’IPC y sont marqués comme historiques et non comme alias acceptés.
- Reconsultation de quatre sources primaires pour les points de bibliothèque les plus sensibles, détaillée ci-dessous. Les autres versions et options de la comparaison n’ont pas fait l’objet d’une nouvelle campagne exhaustive par ce relecteur.

Les audits initiaux conservent leur valeur d’historique. Leurs addenda R2 et la présente revue remplacent leurs anciennes conclusions de langage ou de parité. Le libellé SubscribeActivity rencontré dans l’ancienne revue finale est également historique : l’opération courante est SubscribeEvents, avec le contrat IPC harmonisé. Aucune ancienne conclusion ne sert ici de preuve produit.

## Périmètre effectivement relu

Lecture de fond, puis relecture ciblée des passages modifiés après objection :

- **Brief :** implementation-policy.md, clarifications.md, invariants.md, open-questions.md et revision-2026-11-16-r2.md.
- **ADR :** ADR-03-rust-gui.md et ADR-15-interface-parity-and-pragmatic-rust.md.
- **Architecture :** system-overview.md, rust-workspace.md, security-boundaries.md, ipc-and-apis.md et client-operation-catalog.md.
- **Spécifications :** gui-cli-tui.md et product-requirements.md.
- **F014 et F015 :** leurs quatorze documents, soit pour chacune README.md, requirements.md, interfaces.md, technical-design.md, acceptance.md, tests.md et dependencies.md.
- **Contrats adjacents :** F001/requirements.md, technical-design.md et acceptance.md ; interfaces.md de F006, F011, F013, F016 et F018 ; schéma ipc-command.schema.json.
- **Design :** gui-tui-parity.md, cli-tui-design.md, gui-screen-specifications.md, gui-information-architecture.md, interaction-flows.md, terminal-inspector.md et conversation-ux.md.
- **Plans et recherche :** readiness-checklist.md, v1-implementation-plan.md et technology-comparison.md.

Inspection limitée aux passages trouvés par recherche ou aux en-têtes, sans revendication de relecture exhaustive : workflow-editor.md, F006/technical-design.md, interfaces F001/F002, les autres résultats de recherche transversale et les addenda/en-têtes de rust-feasibility.md, gui-ux-audit.md, consolidated-verdict.md et final-independent-review.md. Les autres features, matrices globales, schémas métier et audits ne sont pas recertifiés dans cette revue ciblée.

## Sources primaires revérifiées

Consultation indépendante du **16 novembre 2026**. Les sources établissent les mécanismes documentés, pas leur configuration ni leur efficacité dans Unleash.

| Source | Point revérifié |
| --- | --- |
| [Tauri v2 — Capabilities](https://v2.tauri.app/security/capabilities/) | Les commandes applicatives enregistrées sont accessibles par défaut aux fenêtres/WebViews ; leur intégration aux permissions doit être explicite. La limite de distinction des iframes sur Linux et les limites des contrôles vis-à-vis du Rust et des WebViews compromis sont documentées. |
| [Tauri v2 — WebDriver](https://v2.tauri.app/develop/tests/webdriver/) | La voie de service embarqué couvre notamment macOS ; la voie tauri-driver native directe reste Windows/Linux. La conception ne répète pas une interdiction générale obsolète de test WebDriver macOS. |
| [Ratatui — Installation](https://ratatui.rs/installation/) | La page consultée annonce Ratatui 0.30.2, Rust au moins 1.88 et Crossterm 0.29 par défaut, avec attention aux versions incompatibles du backend. |
| [Crossterm 0.29.0 — Events](https://docs.rs/crossterm/0.29.0/crossterm/event/index.html) | Clavier/souris/redimensionnement ; choix exclusif entre read/poll sur un même thread et EventStream. Le design TUI ne prévoit pas plusieurs lecteurs concurrents. |

**Statut final : revue de cohérence R2 clôturée après corrections ; qualification et tests produit non exécutés.**
