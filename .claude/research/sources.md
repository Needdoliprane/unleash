# Registre des sources et niveau de vérification

Recherche effectuée le **16 novembre 2026** dans les sources primaires : documentations officielles, spécifications et dépôts mainteneurs. Les pages évolutives ont une date de consultation ; une version affichée par la documentation n'est pas une preuve que le projet compile avec cette version.

## Index thématique

| Registre | Contenu | Conclusions à ne pas dépasser |
| --- | --- | --- |
| [Capacités API vérifiées](verified-api-capabilities.md) | OpenAI, Anthropic, profils compatibles et locaux, outils, sorties structurées, contexte et usage | Documentation publique ; aucun accès de compte, appel payant ou inférence locale testé |
| [Sources modèles locaux](local-model-sources.md) | LM Studio, Ollama, serveur llama.cpp ; HTTP, authentification, placement, capacités et métriques | Trois backends dans la V1 ; aucune version/build, configuration, ressource matérielle ou inférence qualifiée par cette seule recherche |
| [Sources automatisation navigateur](browser-automation-sources.md) | Moteurs réutilisés, Playwright/MCP, WebDriver, locators, traces et frontières de sécurité | Possibilités documentées ; aucun parcours Carrefour, compte marchand ou contrôle général du bureau testé |
| [Sources systèmes](systems-sources.md) | Rust, Tokio, SQLite, WAL, PTY, launchd, Linux, tests de concurrence | Interfaces publiées ; dépendances à verrouiller et comportement OS à qualifier |
| [Sources workflow/skills](workflow-skills-sources.md) | SCXML, JSON Schema, JCS, Temporal, Agent Skills et MCP | Précédents et formats ; Unleash n'implémente pas implicitement ces moteurs |
| [Sources Git/knowledge](git-knowledge-sources.md) | Git, LSP, analyseurs, snapshots, traçabilité | Capacités d'outils ; aucune équivalence sémantique universelle garantie |
| [Sources sécurité/connecteurs](security-sources.md) | OAuth, secrets, OS, Drive/Graph/Notion, MCP, open source | Contraintes officielles ; contrat AI-Guard absent |
| [Comparaison technologique](technology-comparison.md) | GUI native, Tauri/WebView, Rust/WASM, exécution navigateur et modes d’intégration des modèles locaux | Choix GUI adopté par [ADR-03](../architecture/adr/ADR-03-rust-gui.md) ; autres choix à qualifier sur prototype et mesures ; les trois backends locaux demandés ne sont pas réduits à un seul gagnant |

## Références GUI complémentaires

- [egui/eframe, crate 0.36.2](https://docs.rs/eframe/0.36.2/eframe/) : fenêtre native et support web ; distinguer le code applicatif Rust de la glue web.
- [Iced releases](https://github.com/iced-rs/iced/releases) : 0.14.0 stable observée ; des pages de développement indiquent 0.15.0-dev. Ne pas mélanger leurs APIs.
- [GPUI](https://www.gpui.rs/) et [exemples](https://www.gpui.rs/examples/) : non retenu en V1, option de réexamen d'[ADR-03](../architecture/adr/ADR-03-rust-gui.md) ; aucune version stable supposée.
- [Slint FAQ](https://slint.dev/faqs) et [licences](https://slint.dev/pricing) : DSL d’UI, packaging et licences doivent être examinés selon la politique pragmatique R2 et la destination open source.
- [Dioxus desktop 0.7](https://dioxuslabs.com/learn/0.7/guides/platforms/desktop/) : architecture desktop WebView ; Rust applicatif n'implique pas chaîne d'exécution exclusivement Rust.
- [Tauri frontend](https://tauri.app/start/frontend/) : plusieurs frontends, dont Rust/WASM ; ce n'est pas une GUI native pure par la seule présence d'un backend Rust.
- [Kivy](https://kivy.org/) : framework Python officiel. La clarification R2 identifie explicitement Kivy et Tauri ; **Q-17 est résolue**. Kivy reste une alternative de présentation à coût Python, pas une recommandation retenue.
- [GTK4 Rust](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/) : bindings Rust autour de GTK ; dépendances natives et intégration macOS à qualifier.
- [WCAG 2.2](https://www.w3.org/TR/WCAG22/) : recommandation W3C ; les critères de contraste/focus inspirent les tests UX, sans annoncer une certification automatique d'une application native.

## Capacités vérifiées et capacités non vérifiées

Les IDs **gpt-6.1-sol** et **claude-fable-5-2** sont documentés sur leurs pages officielles consultées. La sélection reste configurable et l'accès doit être contrôlé avec la clé et le compte réellement utilisés. OpenAI tool calling pour Sol utilise Responses ; Fable documente des restrictions de tool_choice forcé et des paramètres de sampling. Ces différences justifient des adaptateurs natifs distincts.

Les versions de bibliothèques notées dans les registres sont des observations. Aucun Cargo.lock, binaire Unleash, benchmark, test hardware ou essai API réel n'a été produit par cette mission de conception. Aucune API AI-Guard n'a été confirmée. Les mécanismes proposés côté Unleash sont nommés comme des contrats internes à développer.

LM Studio, Ollama et « CPP », interprété explicitement comme le serveur llama.cpp, appartiennent au périmètre V1. Leurs [sources primaires dédiées](local-model-sources.md) documentent les différences ; les modes d’authentification et capacités doivent être vérifiés sur les installations réelles. Une SecretRef peut être absente selon le backend ; une adresse loopback ne prouve pas une inférence locale. La qualification `local_only`, le refus d’un fallback distant et l’interdiction modèle `forbidden`, embeddings compris, sont des contrats Unleash (ADR-17, accepté en R7), pas des effets automatiques de ces logiciels.

Les motivations de produit se trouvent dans le [périmètre polyvalent](../brief/general-purpose-scope.md) et le [parcours guidé](../design/guided-automation-experience.md). Le développement reste spécialisé ; les cas personnels et professionnels illustrent des usages à qualifier. Une source technique ne confirme ni l’efficacité métier d’une procédure, ni une conformité comptable, ni l’intégration d’un marchand.

## Politique de mise à jour

Une modification de source qui affecte un contrat doit produire une fiche : ancienne/nouvelle référence, date, capacité modifiée, tests de compatibilité, risques et profils touchés. Les catalogues de prix/capacités sont versionnés et gardent leur provenance. Une page non versionnée ne devient pas un pin reproductible ; la réalisation conservera les versions de dépendance et des fixtures protocolaires autorisées.

Les résumés du dossier paraphrasent les sources pour justifier une décision précise. Ils ne remplacent ni leur licence ni la qualification de l'implémentation. Les ADR, acceptés en R7, indiquent les hypothèses et les tests dont la réfutation conduirait à leur réexamen.

## Sources de la révision interfaces R2

La [comparaison technologique](technology-comparison.md) conserve les URLs primaires, dates de consultation et versions documentaires de Tauri, Kivy et des alternatives. La [conception CLI/TUI](../design/cli-tui-design.md) et F015 documentent Ratatui/Crossterm et les alternatives curses. Les numéros observés servent à préparer la qualification, pas à prétendre qu’un assemblage a été compilé. Q-01 et Q-17 sont résolues par la demande utilisateur, non par une source externe ; les autres questions sont tranchées par l’arbitrage délégué de la [révision R7](../brief/revision-2026-11-16-r7.md).

## Compléments R3–R5

Le registre [navigateur](browser-automation-sources.md) et le registre [modèles locaux](local-model-sources.md) conservent les sources de ces extensions et leurs dates. Le [contrat d’automatisation](../specs/reusable-automations.md) sépare préparation, rejeu sans modèle et supervision bornée ; le [contrat local](../specs/local-models.md) sépare politique d’appel et politique de données. Les tests attendus ne sont pas des résultats exécutés. Les questions Q-18 à Q-20 du [registre](../brief/open-questions.md) sont résolues en R7 ; restent les qualifications et mesures, sans rouvrir les choix de périmètre confirmés.
