# F001 — Noyau et daemon utilisateur

## Objectif et valeur

F001 fournit l'autorité locale unique d'OVPT Unleash. Le travail continue après fermeture de la GUI, et les clients retrouvent le même état par commandes et événements. Le daemon est un service utilisateur, sans privilèges root, qui compose le harness, le stockage, les adaptateurs et l'ordonnanceur. Il ne contient pas un second manager LLM : les niveaux N+2/N+1/N utilisent tous les services déterministes communs.

La V1 couvre la matrice qualifiée d'[ADR-14](../../architecture/adr/ADR-14-local-data-oss.md) (macOS 15, 26 et 27 sur Apple Silicon, prioritaire ; Ubuntu 24.04 et 26.04 LTS x86_64), les clients GUI/CLI/TUI et les mécanismes de connexion, récupération et arrêt. Le service utilisateur retenu (agent `SMAppService` ou LaunchAgent sous macOS, `systemd --user` sous Linux) est décrit par [runtime-lifecycle.md](../../architecture/runtime-lifecycle.md) ; son comportement au logout reste à qualifier ; la continuité à la fermeture d'une fenêtre ne signifie pas continuité au logout ou à la panne machine. Les informations de service exposent cette différence.

## Périmètre et limites

Le daemon possède le socket local, le verrou d'instance, les commandes et la publication des événements. F002 possède la transaction durable ; F003 l'admission et les invocations ; F011 les process-hosts ; F018 la récupération. `.claude/` contient le dossier de conception et l'outillage optionnel de développement ; le produit ne le lit jamais à l'exécution et n'y écrit aucun état. Configuration, données, secrets, artefacts, worktrees, logs et cache ont des racines distinctes documentées dans [runtime-lifecycle.md](../../architecture/runtime-lifecycle.md).

Le cœur et le daemon sont Rust ; les dépendances existantes et la présentation autorisée suivent la [politique R2](../../brief/implementation-policy.md). Les repositories développés peuvent être Python, TypeScript, Java, Go, Rust ou multilangages. Le daemon ne convertit pas cette contrainte interne en restriction des projets.

## Livrables et statut

Les contrats ci-dessous sont proposés, sans implémentation ni tests exécutés. Les erreurs distinguent commande reconnue durablement et effet terminé. Le dossier contient [exigences](requirements.md), [conception](technical-design.md), [interfaces](interfaces.md), [dépendances](dependencies.md), [acceptation](acceptance.md) et [tests](tests.md). Les preuves EV resteront `planned` jusqu'à production d'un résultat reproductible sur la cible concernée.
