# Dépendances F001

| Dépendance | Contrat requis | Défaillance traitée |
|---|---|---|
| [F002](../F002-persistence-events/README.md) | Receipt atomique, journal, outbox, snapshot cohérent | Mode dégradé/récupération, aucune fausse confirmation durable. |
| [F003](../F003-agent-runtime/README.md) | Admission globale et checkpoint | Drain sans nouvelle invocation ; compteurs persistés. |
| [F011](../F011-terminal-pool/README.md) | Propriété de hosts et actions de contrôle | Rapport processus non arrêtés, pas de kill par PID seul. |
| [F018](../F018-reliability-recovery/README.md) | Classification des ressources après crash | Admission gelée sur les ressources ambiguës. |
| Services sécurité | Identité locale, capacités, secrets, politiques | Unauthorized ou état bloqué ; AI-Guard externe non supposé disponible. |
| Clients GUI/CLI/TUI | Même envelope et flux | Reconnexion par curseur, jamais DB directe. |

Tokio ([rust-workspace](../../architecture/rust-workspace.md)) et `rusqlite` avec les features `bundled` et `backup` ([ADR-01](../../architecture/adr/ADR-01-sqlite-persistence.md)) sont retenus. Les gestionnaires de services retenus (agent `SMAppService` ou LaunchAgent utilisateur sous macOS, `systemd --user` sous Linux) sont décrits par [runtime-lifecycle](../../architecture/runtime-lifecycle.md#service-utilisateur) ; les interfaces Unix et leurs sources restent documentées dans [systems-sources.md](../../research/systems-sources.md). Les versions réellement compilées, features et bibliothèques natives sont figées par `Cargo.lock` et publiées par le build. La GUI retenue (Tauri 2 et renderer React/TypeScript, [ADR-03](../../architecture/adr/ADR-03-rust-gui.md)) ne change pas l'autorité backend, conformément à la [politique R2](../../brief/implementation-policy.md). Les programmes externes des repositories restent autorisés indépendamment de leur langage.

Le risque d'intégration principal est le packaging du helper et la survie des hosts au restart/logout. Un spike OS est une dépendance de preuve, pas une dérogation de périmètre V1.
