# Comparaison technique et versions observées

**Décision R7 du 16 novembre 2026 :** GUI Tauri 2 avec renderer React + TypeScript strict (Vite, types générés depuis les DTO Rust par ts-rs), acceptée dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md) (Q-02 résolue) ; CLI/TUI clap + Ratatui/Crossterm, acceptées dans [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md). Ce document conserve l’analyse comparative qui a précédé ces décisions.

**Révision : 16 novembre 2026, après clarification de Louis sur Kivy, Tauri, Rust et la parité CLI/TUI/GUI. Statut de l’analyse : antérieure à la décision R7, sans prototype exécuté.** « Observée » signifie version affichée par une source primaire consultée, pas dépendance installée, compilée ou gelée. La qualification produira les versions exactes, lockfiles, toolchain et SBOM.

**Compléments R3–R5 :** le périmètre conserve la spécialisation développement et ajoute des usages polyvalents, l’automatisation navigateur réutilisable, la supervision possible par blocs et les modèles locaux. LM Studio, Ollama et « CPP », interprété ici comme llama.cpp, sont demandés dans la V1 ; leur matrice de référence est fixée par Q-19 (versions figées au gel M2, [modèles locaux](../specs/local-models.md)), dialectes et configurations restant à qualifier. L’analyse GUI R2 ci-dessous est conservée ; la décision est celle d’ADR-03.

## Contrat clarifié et provenance

La première analyse privilégiait egui/eframe sous une interprétation « Rust-only strict ». Cette interprétation est remplacée par la clarification du 16/11/2026 : employer Rust là où il est pertinent, réutiliser les bibliothèques et moteurs existants, autoriser HTML/CSS pour le design et fournir une CLI ainsi qu’une TUI soignée strictement isofonctionnelles avec la GUI. L’exemple du moteur MySQL illustre la réutilisation ; il ne demande ni de réécrire un moteur ni de remplacer SQLite par MySQL.

**Confirmé par le brief :** cœur métier Rust, backend commun, interface soignée et parité fonctionnelle complète. La référence précédemment transcrite « kavi » est clarifiée par Kivy puis Tauri ; ce sont deux technologies distinctes, pas deux noms du même framework. L’utilisateur n’avait pas arrêté de framework ni de renderer en R2 ; l’arbitrage délégué R7 les a tranchés (Q-02, ADR-03).

**Proposé en R2, adopté en R7 :** Tauri 2 pour l’enveloppe desktop, avec hôte/adaptateur Rust `unleash-gui` et présentation HTML/CSS. Le renderer retenu est React + TypeScript strict ([ADR-03](../architecture/adr/ADR-03-rust-gui.md)) ; ce code se limite à la vue, la navigation, la saisie et l’adaptation IPC, sans règle métier. Les dépendances natives, WebView et bibliothèques éprouvées sont admissibles ; aucune réécriture du cœur en Python n’est proposée.

**À qualifier en M0 sur la pile retenue :** versions compatibles, qualité réelle du design, accessibilité, IME, charge, packaging et politique de mises à jour des WebViews. Les analyses d’audit antérieures restent une trace de l’hypothèse initiale, pas une décision qui annulerait cette clarification.

## GUI : options et compromis

| Option / version documentaire observée | Atouts pour Unleash | Dette / points à qualifier | Position R7 |
| --- | --- | --- | --- |
| **Tauri 2, documentation v2 ; patch non gelé** | Hôte Rust et ressources web ; HTML/CSS pour la composition ; choix du renderer indépendant [G1–G3] | WebView système, pont IPC et sécurité à borner ; différences de plateforme, composants riches et chaîne de build à qualifier | **Adopté ([ADR-03](../architecture/adr/ADR-03-rust-gui.md))**, qualifié par la tranche verticale M0 et les gates F014 |
| egui + eframe natif, doc examinée 0.36.2 ; release 0.37.0 visible | UI Rust et dessin technique ; cible native documentée [G8] | Rich text, IME, accessibilité, virtualisation et terminal à mesurer ; rendu immédiat à borner | Non retenu en V1 ; option de réexamen d’ADR-03 |
| Iced, doc publiée 0.14.0 | Modèle de messages inspiré d’Elm, widgets et exécution asynchrone [G9] | README expérimental ; ne pas confondre version publiée et documentation de développement | Non retenu en V1 ; option de réexamen d’ADR-03 |
| GPUI, documentation de projet ; version non gelée | Framework Rust accéléré GPU, modes immédiat et retenu ; issu de l’écosystème Zed [G10] | API, dépendances, distribution, accessibilité et maintenance hors Zed à qualifier | Non retenu en V1 ; option de réexamen |
| Slint, documentation courante ; version non gelée | UI déclarative et API Rust [G11] | Langage Slint et choix de licence GPLv3/royalty-free/commercial selon conditions applicables | Non retenu en V1 ; réexamen soumis à la politique de licence (Q-06) |
| Dioxus desktop, guide 0.7.0 | Composants exprimés en Rust et cible desktop [G12] | WebView et intégration de composants à qualifier ; ne pas assimiler desktop à rendu entièrement natif | Non retenu en V1 ; option de réexamen, plus exclue pour une règle Rust-only obsolète |
| Leptos / Rust-WASM | Renderer web écrit en Rust, DOM et HTML/CSS ; guide Tauri disponible [G7] | Glue JS, bindings de composants et coût de build ; le guide Tauri annonce Leptos 0.6, compatibilité actuelle à tester | Option de réexamen du renderer Tauri, pas un shell desktop autonome |
| GTK via bindings Rust | Accès aux bibliothèques GNOME, widgets et intégrations existantes [G13] | Dépendances natives et expérience macOS à qualifier | Non retenu comme toolkit V1 ; option de réexamen |
| **Kivy, documentation à l’URL stable affichant 2.3.2** | Toolkit d’interface Python et langage de présentation Kv [G14] | Python côté UI en plus du daemon Rust ; packaging, pont client, accessibilité et deux environnements à maintenir | Référence UI identifiée par Louis, non retenue en V1 ; sans interdiction de principe |

Les atouts indiquent des pistes, pas des résultats mesurés. Aucun framework ne garantit à lui seul un beau produit ou la couverture des dix espaces. Kivy n’est ni Tauri ni une bibliothèque Rust ; s’il était retenu, il resterait un client du daemon Rust et ne justifierait pas de réécrire le métier en Python.

Les surfaces officielles Kivy ne sont pas alignées lors de cette consultation : la [page d’accueil](https://kivy.org/) annonce 2.3.1, tandis que les pages documentaires `doc/stable` affichent 2.3.2. Ce relevé ne permet pas d’affirmer une « dernière version » distribuée ; une adoption vérifierait release et artefact exacts avant gel des dépendances.

## Tauri : renderer et frontières

| Variante de présentation | Motif pour l’étudier | Coût à mesurer / condition de choix |
| --- | --- | --- |
| Rust/WASM + HTML/CSS, par exemple Leptos — option de réexamen | Continuité Rust pour composants et DTO, réutilisation de la présentation web | Bindings d’éditeur/diff/terminal, glue générée, temps de compilation, accessibilité et CSP WASM ; ne pas réécrire un composant mature pour éviter quelques lignes de glue |
| HTML/CSS + mince JS/TS — **variante retenue : React + TypeScript strict ([ADR-03](../architecture/adr/ADR-03-rust-gui.md))** | Réutilisation directe de composants web établis ; prototypage et outillage visuel | Types TypeScript générés depuis les DTO Rust (ts-rs, CI en échec sur dérive), dépendances et bundle bornés ; ce code se limite à vue, navigation, saisie et adaptation IPC |

Tauri héberge des ressources statiques et ne fournit pas nativement un serveur SSR [G1]. La proposition Unleash ne requiert donc ni serveur web distant ni serveur Node applicatif : les outils de build éventuels sont distincts des dépendances d’exécution. Le renderer retenu est qualifié sur l’écran de référence ; un réexamen au profit d’une autre variante doit démontrer son gain sur le même écran.

**Architecture retenue (ADR-03) :** présentation WebView → commandes Tauri bornées → adaptateur Rust du client commun `unleash-client` → IPC local authentifié → daemon Unleash Rust. Le « core process » Tauri gère l’enveloppe desktop [G2] ; il ne devient pas le propriétaire des jobs, de SQLite ou de la machine workflow. Le daemon F001 demeure un processus/service indépendant. Fermer, redémarrer ou faire crasher l’ensemble de la GUI, hôte compris, ne suspend ni n’annule un job accepté. Un éventuel emballage en sidecar facilite la distribution [G6] ; il ne prouve pas cette indépendance de cycle de vie.

La WebView reçoit uniquement projections autorisées, références et reçus. Elle n’accède ni au socket Unix, ni au credential IPC, ni aux clés fournisseurs, ni directement au SQL, au shell ou aux fichiers arbitraires. Les commandes, capacités de fenêtre, scopes et bornes d’entrée sont explicitement recensés. La documentation Tauri précise que les commandes applicatives enregistrées via `invoke_handler` sont accessibles par défaut à toutes les fenêtres/WebViews ; `AppManifest::commands` permet de les intégrer aux permissions [G3]. Définir des capacités ne dispense donc pas de vérifier la fermeture effective des commandes personnalisées. Le daemon revalide toujours permissions, révisions et fences ; les scopes applicatifs sont aussi vérifiés dans leurs handlers [G4].

La configuration de production active une CSP restrictive, embarque ses ressources et n’accorde aucun accès IPC à des origines distantes. Le contenu d’outil/document est affiché comme texte ou rendu assaini inerte ; pas de HTML actif injecté dans une fenêtre privilégiée. Une iframe n’est pas utilisée comme frontière de confiance suffisante : Tauri documente une limite de distinction iframe/fenêtre sur Linux [G3]. La CSP de production n’ouvre pas l’évaluation de scripts [G5] ; ses directives exactes sont figées et testées en M0 (ADR-03). Les limites de ces mécanismes, les secrets et leurs tests négatifs sont détaillés dans [F014](../features/F014-gui/technical-design.md).

## Arbitrages backend et terminal

| Décision | Option retenue | Alternative crédible | Critère qui ferait changer |
| --- | --- | --- | --- |
| Persistance | SQLite local état + journal + outbox, via bibliothèque existante | Event sourcing complet | Exigence de projections/replay historique qui justifie sa complexité |
| Exécution | Tokio + tâches bornées + processus isolés du daemon | Threads/sync majoritaires | Profiling montrant coût async inutile ; pas de changement du contrat |
| IPC | Socket Unix versionné, commandes et abonnement événements | HTTP local authentifié | Besoin réel navigateur/clients distants ; Tauri seul ne l’impose pas |
| Git | Git CLI externe supervisé, formats machine | gix ou libgit2 sous adaptateur | Couverture précise et licence/FFI qualifiées |
| Code intelligence | Index central de métadonnées + snapshots, analyseurs spécialisés | Un serveur index universel | Exactitude et coût mesurés favorables |
| DSL | AST fini Unleash, compilation typée | SCXML ou moteur durable externe | Primitives supplémentaires justifiées par workflows réels |
| Secrets | Stores OS explicites via façade Rust | Agent de secrets externe | Besoin organisationnel réel, sans dépendance obligatoire générale |
| CLI/TUI | Adoptée ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) : clap + Ratatui/Crossterm ; navigation terminal soignée et parité complète | ncurses ou autre bibliothèque terminal qualifiée | Qualité clavier, maintenabilité et portabilité mesurées ; supprimer la TUI ou des fonctions n’est pas une alternative |
| Modèles locaux | Adaptateurs HTTP Rust vers LM Studio, Ollama et llama.cpp, endpoints gérés par l’utilisateur d’abord | Processus serveur explicitement géré via F011 | Besoin de gestion démontré, propriété et ressources qualifiées ; aucun moteur d’inférence réécrit |
| Navigateur | Procédure typée et moteur existant sous `BrowserPort` qualifié ([exécution navigateur](../architecture/browser-execution.md)) ; pilote de référence `mediated` (Q-18) : WebDriver W3C piloté en Rust (`fantoccini`) sur Chrome for Testing et chromedriver épinglés | Script de projet autorisé (`project_script`) selon garanties réelles ; Firefox optionnel | Capacités, attente, traces, confinement et reprise mesurés ; aucun SDK Rust fournisseur inventé |

Ces options sont arrêtées par les ADR acceptés en R7 ; la dernière colonne donne leurs critères de réexamen.

Les fonctions couvertes par la GUI doivent toutes être accessibles dans chacun des clients CLI et TUI via le même backend, y compris édition/publication, paramètres, rapports, contexte, preuves, connecteurs et interventions. La parité porte sur les actes, les données, les droits, les erreurs et les reçus. Elle ne promet ni mêmes pixels ni rendu audiovisuel identique : graphes ont une représentation structurelle navigable, médias une description/transcription lorsqu’elle existe et une référence/export autorisé, avec les mêmes actes disponibles. Une alternative ne doit ni inventer un contenu absent ni obliger à ouvrir la GUI pour terminer une action. Voir REQ-UI-012 et [F015](../features/F015-cli-tui/README.md).

Sources systèmes dans [systems-sources](systems-sources.md), de workflows dans [workflow-skills-sources](workflow-skills-sources.md), de Git/index dans [git-knowledge-sources](git-knowledge-sources.md). CLI consultée le 16/11/2026 : [clap 4.6.7](https://docs.rs/clap/4.6.7/clap/), [Ratatui 0.30.2](https://docs.rs/ratatui/0.30.2/ratatui/). L’existence d’une bibliothèque n’est pas une preuve de fonctionnalité Unleash.

## Modèles locaux et automatismes : critères de choix R4/R5

Les [sources modèles locaux](local-model-sources.md) distinguent les possibilités documentées et le contrat Unleash. Les trois backends sont inclus ; ils ne sont pas des alternatives dont un seul serait livré. Leurs profils restent distincts des adaptateurs natifs OpenAI/Anthropic et du compatible générique. Le tuple serveur/build, modèle, template, quantification, contexte et dialecte doit satisfaire les capacités demandées. Ni une marque ni une route ressemblant à OpenAI ne prouvent cette conformité.

Une SecretRef est facultative lorsqu’un backend loopback qualifié n’exige pas de secret. L’exception HTTP se limite à ce périmètre ; une cible distante suit authentification et TLS. `data_locality=local_only` exige un placement qualifié et interdit repli distant ou inconnu, même après OOM. Le choix d’une URL locale ne suffit pas, comme le montrent les relais décrits dans le registre des sources.

La voie HTTP réutilise les moteurs existants sans imposer leur SDK ou leur CLI à la logique métier. Elle réduit la responsabilité d’Unleash à ses contrats de sélection, données, ressources et traces. Une gestion optionnelle de processus ajoute du contrôle de cycle de vie et du coût de maintenance ; elle ne justifie pas d’installer ou charger des poids automatiquement. Aucun runtime, modèle ou benchmark local n’a été exécuté dans cette étude.

Le [contrat local](../specs/local-models.md) et les [tests prévus](../testing/local-model-tests.md) départagent les configurations au moyen des capacités effectives, de l’authentification, du placement, des cas froid/chaud, de l’OOM et de l’annulation incertaine. Le plafond 5/50 totalise les unités `agent_invocation`, `replay_worker` et `provider_probe` ([scheduling](../specs/scheduling.md)), sans promettre une capacité GPU équivalente. Une limite locale initiale d’un appel simultané est une proposition à mesurer.

Pour le navigateur, la [recherche dédiée](browser-automation-sources.md) compare réutilisation de moteur, scripts de projet et clients WebDriver. Elle ne déclare aucun parcours Carrefour opérationnel ni support général du bureau. Les modes sans modèle et supervisé partagent les mêmes contrats ; `forbidden` interdit aussi modèles locaux, embeddings et résumés, tandis que `allowed_bounded` permet des checkpoints explicites.

Les choix techniques doivent servir le [périmètre polyvalent](../brief/general-purpose-scope.md) et le [parcours guidé](../design/guided-automation-experience.md) : démarrage sans agent préalable, procédure recommandée lisible, accès aux détails et mêmes fonctions TUI. La facilité d’un premier résultat et la capacité de comprendre une exception doivent être mesurées avec la performance ; aucun résultat d’utilisabilité n’est présumé.

## Tranche verticale de qualification M0

La tranche verticale M0 qualifie la pile adoptée (Tauri 2 + React/TypeScript, ADR-03) ; elle ne départage plus de candidats et n’impose plus de comparaison avec un candidat natif. Utiliser l’écran de mission de référence (charge d’AC-F014-011) : 1 000 agents logiques, 100 000 événements paginés/virtualisés, diff de 10 000 lignes, saisie multiligne IME, lecteur d’écran, navigation clavier, contexte et terminal sous flux borné. Les bibliothèques de terminal/éditeur existantes sont préférées à une réécriture, sous qualification.

Mesurer lisibilité, cohérence visuelle, latence, mémoire, accessibilité, coût de build, packaging et dette d’adaptateurs sur la machine M0. Rejouer les parcours via CLI/TUI, puis fermeture/crash de la GUI, reconnexion, conflit de révision et contenu hostile. AC-F014-009 à 014 couvrent design, accessibilité, charge, parité, durabilité et frontière WebView. Une défaillance éliminatoire se corrige d’abord en remplaçant le composant fautif ; si elle est intrinsèque à la WebView ou à Tauri, ADR-03 est réexaminé par supersession selon ses critères, jamais par retrait d’une capacité V1.

Cet essai est prévu au plan. Aucun écran, test de sécurité, benchmark ou mesure d’accessibilité n’a été exécuté pendant cette révision documentaire.

## Sources primaires de la révision

Toutes les pages ci-dessous ont été consultées le **16/11/2026**. Les pages non versionnées et les guides « v2 » ne figent aucun patch de bibliothèque. Les URLs sont les sources ; les choix et frontières Unleash ci-dessus relèvent des ADR acceptés en R7, pas de ces sources.

| Repère | Source primaire / version documentaire | Point vérifié et limite |
| --- | --- | --- |
| G1 | [Tauri Frontend Configuration](https://v2.tauri.app/start/frontend/), Tauri v2 | Ressources HTML/CSS/JS/WASM, indépendance du framework, pas de support SSR natif |
| G2 | [Tauri Process Model](https://v2.tauri.app/concept/process-model/), v2 ; [WebView Versions](https://v2.tauri.app/reference/webview-versions/) | Hôte Rust/WebView système ; ne garantit pas la durabilité du daemon Unleash |
| G3 | [Tauri Capabilities](https://v2.tauri.app/security/capabilities/), v2 | Permissions par fenêtre/WebView, défaut des commandes personnalisées, limite des iframes |
| G4 | [Tauri Command Scopes](https://v2.tauri.app/security/scope/), v2 | Application des scopes dans les handlers, à auditer |
| G5 | [Tauri CSP](https://v2.tauri.app/security/csp/), v2 | Activation explicite et contrainte de compilation WASM ; CSP seule insuffisante |
| G6 | [Embedding External Binaries](https://v2.tauri.app/develop/sidecar/), v2 | Packaging d’exécutables, pas contrat de service durable |
| G7 | [Tauri / Leptos](https://v2.tauri.app/start/frontend/leptos/), guide annonçant Leptos 0.6 ; [Leptos : intégration JavaScript](https://book.leptos.dev/web_sys.html), livre courant | Chemin Rust/WASM documenté ; versions et glue à requalifier ensemble |
| G8 | [eframe 0.36.2](https://docs.rs/eframe/0.36.2/eframe/), [releases egui](https://github.com/emilk/egui/releases) | Documentation 0.36.2 examinée ; release 0.37.0 également visible le 16/11/2026, sans migration ni benchmark exécutés |
| G9 | [Iced 0.14.0 README](https://docs.rs/crate/iced/0.14.0/source/README.md), [releases Iced](https://github.com/iced-rs/iced/releases) | Document publié distinct du développement ; qualification Unleash future |
| G10 | [GPUI](https://gpui.rs/), [exemples](https://gpui.rs/examples/), documentation courante | Framework Rust GPU, origine Zed ; pas preuve de maturité de tous les composants |
| G11 | [Slint FAQ](https://slint.dev/faqs), [licences](https://slint.dev/pricing), pages courantes | Langage déclaratif, API Rust, options de licence à qualifier |
| G12 | [Dioxus desktop](https://dioxuslabs.com/learn/0.7/guides/platforms/desktop/), guide 0.7.0 | Cible desktop ; pas qualification de l’intégration Unleash |
| G13 | [gtk-rs](https://gtk-rs.org/), documentation de projet | Bindings Rust vers GNOME/GTK |
| G14 | [Kivy stable](https://kivy.org/doc/stable/), [installation affichant 2.3.2](https://kivy.org/doc/stable/gettingstarted/installation.html), [accueil affichant 2.3.1](https://kivy.org/) | Toolkit Python ; support annoncé CPython 3.8–3.14 dans la doc 2.3.2 ; divergence de versions documentée, pas de version distribuée gelée |
| G15 | [Tauri WebDriver](https://v2.tauri.app/develop/tests/webdriver/), v2, page mise à jour le 29/06/2026 | Service embarqué documenté notamment sur macOS ; driver natif direct limité à Windows/Linux ; chaque voie reste à qualifier |
