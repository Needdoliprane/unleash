# Conditions de préparation et de clôture

Cette checklist distingue la **complétude du dossier** de la **qualification du produit**. À la rédaction, aucun build d’Unleash n’est revendiqué. Les statuts de recette produit restent « à exécuter ». Depuis R7, la réalisation commence en M0 ; une preuve réelle est consignée dans `qualification/evidence/` à la racine du dépôt, jamais dans `.claude/`.

## Dossier de conception

| Gate | Preuve attendue | Responsable |
| --- | --- | --- |
| DOC-01 Couverture | Chaque invariant et chaque section/annexe reliés à un document et feature | Product + QA |
| DOC-02 Interfaces | Identités, états, erreurs, commandes et schémas cohérents | Chief + propriétaires |
| DOC-03 Traçabilité | REQ → AC → T → EV sans exigence orpheline | QA |
| DOC-04 Contradiction | Première revue par domaines puis revue indépendante des documents rédigés | Adversarial |
| DOC-05 Sources | Références primaires datées, version observée, limites explicites | Chaque expert |
| DOC-06 Navigation | Arborescence complète, liens locaux résolus, exemples parsables | QA documentaire |
| DOC-07 Décisions | Registre Q-01 à Q-21 entièrement tranché : Q-01/Q-17 résolues en R2, les autres résolues en R7 par arbitrage délégué ; navigateur, modèles locaux, templates guidés, budgets, checkout et analyseurs décidés, chacun suivi par une preuve nommée : navigateur (Q-18, EXT-BROWSER), modèles locaux (Q-19, GATE-LOCAL et EXT-MODEL-LOCAL), templates guidés (Q-20, GATE-GUIDED), budgets (Q-07, EXT-MODEL), checkout (Q-10 : T-F010-001/006 au gate M4 du [plan](v1-implementation-plan.md)) et analyseurs (Q-13 : scénario T, rejoué en M7 sur la matrice EXT-OS) ; récurrence calendaire exclue de la V1 par Q-21 ; sans fait inventé | Product |
| DOC-08 Consolidation R3–R5 | F001–F018 conservées + F019, H-17 à H-21 et INV-058 à INV-065 reliés ; replay/supervised distincts, localité séparée de l'adresse, un seul moteur/DB/admission | Architecture + QA |

Le résultat réel de ces contrôles est consigné dans [document-validation](../testing/document-validation.md).

## Avant construction parallèle

GATE-ARCH : les dix-neuf ADR structurants sont acceptés en R7 ([révision R7](../brief/revision-2026-11-16-r7.md)), dont ADR-10 et ADR-14 ; la sémantique run est figée par Q-03 ; restent à figer en M0 les DTO et types du protocole. GATE-RUST : vérifier la frontière cœur Rust / présentation autorisée, inventorier les dépendances, qualifier la toolchain épinglée par `rust-toolchain.toml` et créer Cargo.lock. GATE-UI : tranche verticale M0 sur les piles acceptées ADR-03 (Tauri 2 + React/TypeScript) et ADR-15 (Ratatui/Crossterm), et matrice de parité complète ; les permissions de réutiliser les bibliothèques et HTML/CSS sont déjà acquises. GATE-AUTH : modèle de menace et modes d’isolation décidés (Q-05) ; qualifier le mode `strict` (backends Seatbelt sur macOS et bubblewrap avec seccomp sur Linux, egress par le proxy local) et son refus sur les OS de la matrice ADR-14. Aucune décision ne reste ouverte : seules les preuves de ces gates restent à produire.

GATE-AUTOMATION : qualifier BrowserPort, le pilote de référence WebDriver de Q-18 et les profils mediated et project_script en M0, puis profils v1.1/v1.2, données/artefact versionnés, modèle interdit et supervision publiée en M3. GATE-LOCAL : verrouiller au gel M2 la matrice de Q-19 ([modèles locaux](../specs/local-models.md)) — LM Studio, Ollama et llama.cpp, modèle texte Qwen3-8B instruct GGUF Q4_K_M et un modèle d’embedding — par tuple serveur/build/modèle/template/quantification/configuration, capacité, localité et ressources. GATE-GUIDED : tester en M5 les six templates de Q-20 et un premier travail sans agents configurés ni Git obligatoire, avec aperçu/autorité, opérations équivalentes en TUI et les seuils de mesure du [parcours guidé](../design/guided-automation-experience.md). Ces gates décrivent les preuves à produire, pas des validations acquises par rédaction.

## Avant qualification d’une feature

Chaque requirement a au moins un cas positif, ses rejets significatifs et sa preuve. Les tests ne doivent pas se réduire à vérifier un mock qui reproduit l’implémentation. Les effets de bord ont des points de panne, un état ambigu et une procédure de réconciliation. Les read models ont une politique de fraîcheur. Les opérations sensibles ont une autorité déterministe et une trace expurgée.

La feature fournit : version du code, environnement, commande de test exacte, seed/corpus, statut, références d’artefacts et limites. Une preuve n’est réutilisable que si ses entrées et son périmètre restent valides.

## Gates de dépendances et plateformes obligatoires de V1

| Gate | Bloque | Pièces et preuve |
| --- | --- | --- |
| EXT-GUARD | Recette F017 | Code/protocole réel AI-Guard, version, auth, refus, révocation, panne et audit corrélé |
| EXT-MODEL | Recette native F012 | Comptes de test OVPT dédiés OpenAI/Anthropic, plafond de dépense chez le fournisseur, clés au seul coffre de CI ; profils `gpt-6.1-sol` et `claude-fable-5-2`, capacité réelle, stream/tool/schema/erreurs et coûts observés ; catalogue de prix de la release confronté aux coûts facturés |
| EXT-MODEL-LOCAL | Recette locale F012 | LM Studio, Ollama et llama.cpp réels sur la matrice Q-19 ; tuples/versionnements et capacités, local_only/placement, transport/auth/TLS, cold/warm/OOM, annulation et inconnues ; aucun test HTTP simple présenté comme qualification totale |
| EXT-BROWSER | Recette F019/F011/F017 | Chrome for Testing, chromedriver du même build et adaptateur `fantoccini` réels, épinglés par version et SHA-256 (Q-18), proxy de sortie, credentials de fixture, contrôle du contexte, DOM hostile, effets même domaine, modèle interdit, reprise et limites du script opaque |
| EXT-DOC | Recette F013 | Comptes de test Drive, OneDrive, Notion ; droits, suppression, pagination et resync ; clients officiels `ovpt_official` enregistrés et vérifiés, sinon release en `bring_your_own` seul (Q-08) ; claims id_token, consentements réels et token d’intégration Notion (Q-09) |
| EXT-OS | Release locale | Builds et tests sur la matrice qualifiée d’[ADR-14](../architecture/adr/ADR-14-local-data-oss.md) : macOS 15, 26 et 27 sur Apple Silicon (minimum 15.0), Ubuntu 24.04 et 26.04 LTS x86_64 ; daemon user, secrets, processus, terminaux et accessibilité WCAG 2.2 AA avec VoiceOver et Orca |
| EXT-OSS | Publication open source | LICENSE, NOTICE et en-têtes SPDX Apache-2.0 appliqués (Q-06), `cargo deny` vert, SBOM et notices tierces par artefact, politique de marque, SECURITY et CONTRIBUTING (DCO), recherche d’antériorité du nom |
| EXT-DIST | Distribution utilisateur | Developer ID OVPT actif, Hardened Runtime et notarisation agrafée, DMG/archives et `.deb` vérifiables, SBOM et SHA-256 publiés ; mise à jour signée appliquée sur commande de l’utilisateur par l’installeur ou l’updater de l’application, hors protocole daemon ; restauration et désinstallation prouvées sur la matrice ADR-14 (Q-16) |

Un blocage externe ne disparaît pas dans une moyenne de couverture. Il est visible dans le rapport de recette ; aucune simulation ne doit être présentée comme validation fournisseur réelle. Un serveur local absent ou une frontière d'isolation indisponible conserve un statut blocked pour le profil concerné ; cela ne transforme pas le besoin en permission de cloud fallback ou de script libre sous garantie médiée. Les tests utilisent données/identités synthétiques, sans achat ni paiement réel requis.

## Clôture V1

La clôture exige que les scénarios A–O et leurs compléments, dont W/X/Y/Z de la [recette R3–R5](../testing/acceptance-scenarios.md), passent sur la matrice qualifiée d’ADR-14. Les 19 features doivent être acceptées, sans retirer les 18 initiales. Les états « OutcomeUnknown », « Stale », « Blocked » et « Unsupported » sont testés et compréhensibles dans toutes les interfaces. Les objectifs de performance sont mesurés sur une configuration publiée. Les sauvegardes sont restaurées, les migrations interrompues testées et les données sensibles expurgées selon la politique.

Les preuves déterminantes supplémentaires sont : replay sans invocation agente/Git/clé modèle nécessaires ; zéro dispatch modèle mesuré jusque dans les descendants et rapports ; supervision bornée par checkpoints et corroborée par assertions/snapshots ; CSS valide au résultat métier faux ; incident d'effet conservé après nouvelle version ; plafond mixte 5/50 sans script autonome hors unité ; qualification des trois profils locaux et absence de cloud fallback sous local_only ; premier parcours guidé et réglages complets en GUI/TUI. Les contrôles de produit ne deviennent pas des étapes de tests ou de qualité imposées à tous les workflows utilisateur.

Les comptes finaux REQ/AC/T/EV et les résultats effectivement exécutés appartiennent à la [traçabilité](../testing/traceability.md) et au rapport de qualification. Cette checklist ne leur attribue aucun passed par anticipation ; la validation documentaire des plans reste distincte des [tests d'automatisation](../testing/automation-replay-tests.md) et des [tests locaux](../testing/local-model-tests.md).

Toute dérogation à un invariant utilisateur requiert une décision de son autorité, avec portée et conséquences ; elle ne peut être déduite d’un commentaire d’expert. Les arbitrages techniques acceptés en R7 ne sont pas des dérogations : ils respectent les invariants et restent révisables par un ADR de supersession.

## Contrôle de remise R6

DOC-09 exige les quatorze rapports attribués, les corrections reliées et la contrelecture de remise. DOC-10 exige la concordance des sources avec les catalogues, opérations, groupes de parité et chaînes REQ→AC→T→EV ; l’importeur refuse les données étrangères, références externes actives et statuts produit sans preuve. Les [exigences DOC-ING-001 à 008](../specs/document-package-ingestion.md) et le [rapport final](../testing/document-validation.md) constituent les oracles documentaires, sans remplacer un gate produit. Depuis R7, `tools/design-package/validate.py` est le validateur de référence du paquet ; il doit finir sans erreur après régénération.

M0 inclut les DTO Rust/payloads complets du protocole IPC v2 et le refus des anciennes lignées ; la qualification de la cible de signal non réutilisable ; les limites de la connaissance multi-snapshots ; le profil v1.2 pour l’itération de données ; les attentes de tests scellées et les contrôles de provenance de contexte. Les catalogues `contract_stage=documented` ne sont pas présentés comme une API runtime déjà générable ou testée.
