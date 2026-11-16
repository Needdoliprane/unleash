# Modèle de composants et propriété des décisions

| Composant | Possède | Entrées / sorties | Ne possède pas |
| --- | --- | --- | --- |
| Domain | Types, invariants, transitions pures | Commande + état + faits → décision/événements/intents | Réseau, filesystem, SQL |
| Application | Cas d’usage, ports et transactions métier | API locale, décisions agents | Un deuxième jugement technique obligatoire |
| Store | État engagé, revisions, journal, outbox/inbox | Transactions courtes, snapshots | Effets externes en transaction |
| Scheduler | Admission globale 5/50 des agent_invocation, replay_worker et provider_probe, quotas, fairness et wakeups | Jobs persistants, leases, bundles et réservations additionnelles | Pool supplémentaire d'actions, ou réveil LLM sous forbidden |
| Session/Context | Historique, manifest, compactions | Sélection et provenance | Pensée privée du modèle |
| Workflow/Compiler | AST v1/v1.1/v1.2 normalisé (v1.2 requis pour for_each), analyse sémantique, actions bornées et task de supervision déclarées | Draft → version ; événements → transitions du même WorkflowReducer | Code arbitraire dans guards ou second orchestrateur de scripts |
| Automation, module du moteur | AutomationArtifact, données révisionnées, compilation, qualification et projection de Run | Candidat et CSV → plan épinglé ; résultats par ligne et incidents | Autorité indépendante, faux agent obligatoire ou modification en cours de run |
| Registries | Skills, rôles et automatisations dans leurs namespaces, versions/publication | Discovery, drafts, evaluations | Auto-élévation d’autorité |
| Contract | Validation structurale et preuves | Outputs, receipts, règles | Qualité sémantique générale |
| Model Router | Protocoles API et profils LM Studio/Ollama/llama.cpp, capacités qualifiées, localité et budgets | Context bundle autorisé → manifest/result/events | États métier, garantie local_only déduite de localhost ou inférence sous forbidden |
| Action Broker | Autorisation et dispatch contrôlés | Intent → decision → observation | Garantie syscall sans backend isolation |
| BrowserPort et adaptateur Rust | Mapping moteur/driver qualifié, identité de contexte, opérations et observations typées | Intent autorisé → commande de moteur externe → résultat/effet classifié | Autorité issue du DOM, décision d'achat par domaine ou réparation libre |
| Git | Inventaire, branches, intégration, CAS | OIDs et worktrees → preuves | Arbitrage fonctionnel non demandé |
| Knowledge | Index code/work/evidence, freshness | Captures et événements → discovery | Code source canonique |
| Document connectors | Lecture et suivi ressources autorisées | ResourceRef → snapshot | Crawl global implicite |
| Process supervision | Identité/groupe/PTY/log/ports | Spawn/attach/stop → receipts | PID comme preuve unique |
| Reports | Projections, templates déterministes et navigation causale | Observations/claims/judgments, compteurs de supervision/réparation visibles | Modifier les résultats des tests ou ajouter un rédacteur LLM caché |
| Clients | Rendu, édition de drafts, interaction | API commune | Écriture directe SQLite |
| Guard adapter | Mapping du contrat réel AI-Guard : crate `unleash-guard-adapter`, `guard_mapping_profile` versionné, profil initial `none_accepted` ([ADR-10](adr/ADR-10-ai-guard-boundary.md)) | Autorité/action → décision | API distante inventée |

## Règles de dépendance

Le domaine ne dépend d’aucun adaptateur. L’application dépend de ports ; le daemon compose leurs implémentations. Les clients dépendent du protocole et des DTO. Les adaptateurs peuvent partager clients HTTP et types d’erreur sans partager les secrets d’autres comptes. Automation est un module du moteur, pas un microservice : il partage base, writer, reducer, contrats et admission. Le profil navigateur est une spécialisation d'artefact ; les procédures de données sans navigateur et les projets sans Git utilisent les mêmes objets.

Un composant n’écrit pas directement les tables d’un autre au détour d’une commande outil. Il soumet un résultat au cas d’usage propriétaire, qui engage l’ensemble requis. Les index peuvent être mis à jour asynchroniquement, avec leur watermark visible ; une transition exigeant leur fraîcheur attend ou marque l’incertitude.

## Contrats de coordination

Un intent d’action porte toutes ses références avant dispatch. Un résultat late/stale est archivé comme observation, mais sa révision/fence interdit de muter l’étape déjà réaffectée. Une modification de contrat conserve une nouvelle version ; elle ne relit pas les anciens résultats selon un schéma changé.

Les noms exacts de crates sont dans [rust-workspace](rust-workspace.md). La carte de composants n’impose pas une crate par ligne : un module suffit tant que sa frontière est vérifiable et que l’équipe n’a pas besoin d’un package séparé.

## Frontière des trois clients — R2

La GUI Tauri 2 retenue par [ADR-03](adr/ADR-03-rust-gui.md) comprend un renderer React/TypeScript de présentation et un hôte Rust vers le client IPC `unleash-client`. La TUI Ratatui/Crossterm retenue par [ADR-15](adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la CLI scriptable utilisent ce même client, seule voie d'accès au daemon. Le catalogue de cas d’usage, les validations d’autorité et les états sont propriétaires du daemon ; la visibilité locale d’un bouton ne vaut jamais permission. Les clients ont des modèles de vue et brouillons éditables, sans état métier durable concurrent.

La [matrice GUI/TUI](../design/gui-tui-parity.md) couvre toutes les fonctions des dix espaces. Une nouvelle opération est d’abord définie avec schéma et tests communs, puis présentée dans les interfaces ; elle ne devient pas un endpoint privé GUI. Les ressources HTML/CSS, le renderer React/TypeScript et les bibliothèques natives sont autorisés par la [politique R2](../brief/implementation-policy.md) et ADR-03.

## Origines, modèles et moteurs réutilisés — R3–R5

Les parcours guidés créent ou réutilisent objets et rôles par les commandes backend révisionnées dans le mandat ; le client n'a pas besoin de masquer une orchestration privée. Un replay mécanique ne matérialise aucun agent. En mode supervised, la décision `continue/reconcile/correct/escalate` provient d'une task publiée et reste contrôlée par le contrat, notamment en présence d'un effet inconnu ou d'une assertion contraire. Une adaptation du code crée un job/version distincts.

Un serveur navigateur ou d'inférence réutilisé garde ses propres langages et protocole. L'adaptateur Rust connaît ses capacités et versions ; il ne lui délègue ni le ledger ni les droits. Un script de projet peut être JavaScript ou Python, mais une exécution opaque n'acquiert pas les garanties du profil médié par le seul fait d'avoir un exit status ou un réseau limité. Les détails et preuves sont dans [browser-execution.md](browser-execution.md), [automation-artifact-model.md](automation-artifact-model.md) et [local-models.md](../specs/local-models.md).

L'origine d'exécution est vérifiable à chaque frontière. Aucun driver, manager, rapport ou outil ne peut supprimer sa filiation pour contourner `forbidden`. Les capacités locales sont qualifiées par configuration réelle ; une erreur OOM ne transforme pas `local_only` en permission cloud. Les garanties d'isolation et leurs limites restent des propriétés démontrées du backend, pas des noms de composants.
