# ADR-03 — GUI Tauri 2 + React/TypeScript et parité des interfaces

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : Q-02 est tranchée ; shell Tauri 2, hôte Rust `unleash-gui` client du daemon et renderer React + TypeScript strict construit par Vite et embarqué ; voir la section « Amendement R7 ». Remplace la décision proposée R2, révisée le même jour après clarification utilisateur. Réexamen seulement sur défaut éliminatoire mesuré en M0. Accepté n’est pas confirmé : cet ADR se révise par un ADR de supersession.

## Contexte et provenance

Louis demande un produit beau, une navigation terminal soignée de type ncurses ou meilleure et une CLI/TUI strictement isofonctionnelle avec la GUI. Sa clarification autorise HTML/CSS, l’emploi raisonné de Rust et la réutilisation de bibliothèques existantes. Elle remplace l’interprétation antérieure « chaque composant applicatif et dépendance doit être Rust ». L’exemple d’un moteur de base de données ne change pas le choix de persistance et n’impose aucune réécriture.

Le cœur métier, la planification, la persistance et les clients système restent en Rust selon la contrainte utilisateur confirmée ; le moteur de présentation est un choix technique, arrêté en R7. Kivy et Tauri ont été identifiés ; l’ambiguïté « kavi » est levée. Le fait que Kivy soit un toolkit Python n’en fait ni un synonyme de Tauri ni une demande de réécrire le cœur en Python. Les versions et faits vérifiés sont conservés dans la [comparaison technique](../../research/technology-comparison.md).

Exigences : REQ-UI-001 à 014 et REQ-CLI ; risques R-14/R-15, hypothèse H-06. Q-01 et Q-17 sont résolues par la clarification R2 ; Q-02, choix technique du framework GUI et du moteur de présentation, est résolue par cet ADR en R7. La vérification des dépendances et des frontières ne rouvre pas une demande de permission sur HTML/CSS ou sur la réutilisation de bibliothèques.

## Options et compromis

| Option | Position et justification |
| --- | --- |
| **Tauri 2 + adaptateur Rust + HTML/CSS** | Retenue (R7), avec renderer React + TypeScript strict : répond directement au besoin de présentation soignée et permet de réutiliser des composants web, en conservant le métier dans le daemon Rust |
| Renderer Rust/WASM, par exemple Leptos | Option de réexamen, non retenue pour la V1 : préférence Rust, avec coût de bindings/glue et versions compatibles à mesurer |
| Renderer mince JS/TS | Retenue sous la forme React + TypeScript strict (R7) pour réutiliser éditeur/diff/terminal et outillage ; limité à la vue, la navigation, la saisie et l’adaptation IPC |
| egui/eframe, Iced, GPUI | Options de réexamen ; egui/eframe ou Iced sont les candidats natifs d’une supersession (amendement R7, point 11) ; charge, accessibilité, IME, composants riches et maintenance décident, pas une préférence esthétique présumée |
| Slint, Dioxus desktop, GTK/Rust | Options de réexamen ; licences, dépendances et intégration à qualifier ; aucune exclusion fondée sur une interdiction générale de dépendances non Rust |
| Kivy | Référence UI non retenue : runtime Python côté client ; coût supplémentaire de packaging et de contrats face au daemon Rust, sans bénéfice démontré ici |

Ces positions sont des jugements de conception, pas des performances observées. Les faits de framework et leurs sources officielles datées figurent dans la comparaison, repères G1 à G15. Le guide Tauri/Leptos consulté annonce Leptos 0.6 ; il ne prouve pas une combinaison actuelle de versions prête à compiler.

## Décision

1. Shell Tauri 2 avec ressources de présentation embarquées et hôte/adaptateur Rust ; renderer React + TypeScript strict (amendement R7). Composants réutilisés, lisibilité, clavier, accessibilité, IME, charge, sécurité et coût de maintenance restent les critères de qualification. Aucun serveur web distant ni moteur métier JavaScript.
2. Garder le daemon Rust indépendant de tout frontend. La GUI passe par l’adaptateur Tauri puis le même client IPC que CLI/TUI (`unleash-client`, [ADR-15](ADR-15-interface-parity-and-pragmatic-rust.md)) ; elle ne lit pas directement SQLite. Le processus « core » de Tauri est l’hôte desktop, pas le core métier durable Unleash. Fermer ou faire crasher la GUI, hôte compris, laisse les jobs acceptés continuer. Un sidecar de packaging ne suffit pas à prouver cette propriété.
3. Exposer à la WebView une liste bornée de commandes et projections typées. Aucun accès brut au socket Unix, aux credentials IPC, aux clés fournisseurs, au shell ou au SQL. Le daemon conserve les contrôles d’autorité, de révision, d’idempotence et de fence. Les [capabilities Tauri](https://v2.tauri.app/security/capabilities/), v2 consultées le 16/11/2026, exigent une configuration effective des commandes applicatives ; ne pas déduire une fermeture par défaut de la seule présence d’un fichier de capabilities.
4. Activer une CSP de production adaptée, embarquer les ressources et traiter documents, sorties d’outils, logs et Markdown comme non fiables. Rendu inerte, sans scripts ni privilèges IPC distants. Qualifier la frontière sous injection ; ne pas faire d’une iframe une preuve d’isolation. Voir [F014 technique](../../features/F014-gui/technical-design.md).
5. Garantir la parité fonctionnelle complète GUI/CLI/TUI : mêmes capacités de lecture et d’action, données, permissions, états, erreurs et reçus. Les gestes et rendus s’adaptent au support. Pour un graphe ou média, offrir structure navigable, métadonnées/contenu textuel disponible et accès autorisé à la ressource, avec tous les actes accessibles sans GUI. Ne promettre ni mêmes pixels ni transcription inventée.

## Amendement R7

Q-02 est résolue par l’arbitrage délégué de Louis du 16 novembre 2026 ([révision R7](../../brief/revision-2026-11-16-r7.md)).

1. **Shell.** Tauri 2, branche 2.x stable, patch figé dans Cargo.lock au M0. L’hôte Rust `unleash-gui` est un client du daemon via `unleash-client` ; le processus core Tauri n’est jamais l’autorité métier.
2. **Renderer.** React + TypeScript strict, bundle Vite embarqué depuis `apps/gui-renderer/` ([règles du renderer](../../rules/gui-renderer.md)). Node.js LTS et pnpm servent au seul build, versions figées par `pnpm-lock.yaml` ; à l’exécution, ni serveur Node, ni SSR, ni CDN, ni code distant. Le TypeScript ne porte ni autorité, ni transition, ni budget, ni validation métier ; sa prévalidation de forme n’est qu’une aide.
3. **Contrat.** Les DTO Rust sont canoniques. Les types TypeScript sont générés par ts-rs dans `apps/gui-renderer/src/generated/` ; la CI échoue sur tout diff de régénération. L’hôte désérialise strictement (`deny_unknown_fields`).
4. **Commandes.** Une commande Tauri par opération du catalogue exposée au renderer, générée depuis l’énumération Rust des opérations. Les opérations exposées sont exactement celles dont `parity_group_ids` n’est pas vide dans le [registre](../../catalog/operations.json) ; `Hello` est conduit par l’hôte via `unleash-client` et le flux de `SubscribeEvents` est servi au renderer sur un `tauri::ipc::Channel` borné. `RecordClaim`, `RecordObservation` et `RecordJudgment`, commandes du canal worker/harness sans groupe PAR, ne sont jamais exposées au renderer. Aucune commande générique de relais. Les commandes sont déclarées dans le manifeste de permissions Tauri et accordées à la seule fenêtre `main` ; aucun plugin shell, fs ou http dans le build livré.
5. **Contenus non fiables.** Le Markdown est analysé côté Rust en AST borné puis rendu en éléments React ; `dangerouslySetInnerHTML`, `innerHTML` et `eval` sont interdits par un lint bloquant. xterm.js désactive par défaut les liens OSC 8 et le presse-papiers OSC 52. Un lien externe ne s’ouvre que sur action de l’utilisateur, après validation par l’hôte.
6. **CSP de production.** Ressources `self` uniquement ; ni `unsafe-eval`, ni `wasm-unsafe-eval`, ni script inline non haché, ni origine distante. Les directives exactes sont figées et testées en M0.
7. **Secrets.** Saisie dans une fenêtre native créée par l’hôte hors WebView (`unleash-platform`) ; la valeur suit le canal privé BeginSecretImport et la WebView ne reçoit que SecretRef et statut.
8. **Composants.** CodeMirror 6 et @codemirror/merge, xterm.js, TanStack Virtual, React Aria Components, @xyflow/react (vue graphe toujours doublée de l’arbre navigable). Chaque dépendance du renderer passe la revue licence, maintenance et taille des crates et figure au SBOM.
9. **Tests.** Logique de présentation sous Vitest ; parcours de bout en bout par la voie Tauri WebDriver, qualifiée sur chaque plateforme de la matrice d’[ADR-14](ADR-14-local-data-oss.md). Capacités et plugins de test ne sont compilés que sous une feature exclue du build livré.
10. **Cycle de vie.** La GUI ne lance jamais le daemon comme enfant ; elle passe par le service utilisateur du [cycle de vie](../runtime-lifecycle.md). Fermer ou faire crasher la fenêtre et l’hôte laisse les jobs acceptés continuer.
11. **Réexamen.** Un défaut éliminatoire mesuré (IME, lecteur d’écran, charge AC-F014-011, fermeture des commandes) se corrige d’abord en remplaçant le composant fautif. S’il est intrinsèque à la WebView ou à Tauri, cet ADR est rouvert par supersession avec egui/eframe ou Iced comme candidat natif, sans retrait de capacité V1.

## Conséquences et impact V1

Le shell desktop et sa présentation peuvent évoluer sans déplacer les règles métier ou le journal durable. Rust reste prioritaire quand il sert la robustesse et le partage de contrats ; une bibliothèque spécialisée est préférée à une réécriture injustifiée. HTML/CSS, WebView, composants externes et le TypeScript de présentation introduisent des dépendances et frontières à maintenir explicitement.

La tranche verticale M0 qualifie la pile retenue ; la comparaison préalable à un candidat natif n’est plus un gate de sélection. Le coût de cette qualification est assumé ; aucun écran V1, inspecteur terminal, éditeur, contenu lisible ou action terminal n’est supprimé pour faciliter le framework. Une interface agréable ne remplace ni les tests de parité ni les tests du daemon réel.

## Vérification, sortie et réexamen

Les chaînes REQ-UI-001 à 011 restent stables. Ajouts : **REQ-UI-012** parité GUI/CLI/TUI, **REQ-UI-013** indépendance de cycle de vie, **REQ-UI-014** frontière WebView. Chaque ajout possède AC-F014/T-F014/EV-F014 de même numéro. Les critères AC-F014-009 à 011 continuent de couvrir design, accessibilité et charge.

La qualification M0 de la pile retenue exige captures et parcours, résultats sur builds identifiés, versions et dépendances figées, fichier de capacités Tauri effectif et refus prouvé d’une commande depuis une fenêtre non autorisée, CSP observée, injections HTML, Markdown, SVG, URL et logs inertes, canaris de secrets absents du DOM, du stockage, des événements et des diagnostics, diff ts-rs nul, preuves du daemon survivant à la fermeture/crash, erreurs/reçus communs et essais négatifs de l’IPC WebView. Ces preuves ne sont pas un arbitrage en attente. Revoir cet ADR selon le point 11 de l’amendement R7, si un composant essentiel manque ou si la contrainte utilisateur évolue. Une mise à jour de bibliothèque ne peut modifier silencieusement une exécution engagée.

**État des preuves :** revue documentaire seulement ; aucun prototype, benchmark, test produit ou configuration de sécurité n’a été exécuté/validé dans cette révision. Détail canonique : [comparaison et sources primaires datées](../../research/technology-comparison.md), [F014](../../features/F014-gui/README.md), [IPC commun](../ipc-and-apis.md).
