# F017 — conception technique

## Autorité et exécution

Le module security calcule les capabilities effectives par intersection des mandats, versions de rôle/skill et politique projet. ActionBroker est la seule entrée de lancement d’actions par le harness, y compris Git, connecteurs écriture, processus et saisie PTY. Il normalise la ressource, hache les arguments et produit une intention durable.

Une décision est attachée à action_id, policy_revision, scope_revision, digest et expiration. Le broker revalide ces éléments juste avant effet, puis persiste l’observation. Le passage en revue ne tient pas une transaction SQL pendant le réseau. L’autorisation après timeout ou réponse malformée n’est jamais présumée.

## Isolation et promesses

Les outils typés font appliquer leurs limites de fichiers et ressources dans les deux modes. Un shell lancé sous le même UID peut avoir des droits ambiants plus larges. La politique projet porte `isolation_mode` (Q-05) : `standard`, défaut, affiche cette portée et reste non confiné ; `strict`, choix explicite, fait appliquer au lancement par `unleash-process-host` un backend qualifié (profil Seatbelt via `/usr/bin/sandbox-exec` sur macOS ; bubblewrap avec seccomp sur Linux, Landlock admis comme backend de fichiers lorsque son ABI couvre les obligations) et limite la sortie réseau au proxy d’egress local d’Unleash, selon la liste d’hôtes du mandat. Le mode est fixé au lancement et hérité par les descendants. Une regex sur la commande ne transforme pas un build en exécution sûre.

Les adapters OS rapportent leurs capabilities. En mode strict, ou si une politique exige une isolation absente, l’action est refusée avant lancement avec `IsolationUnavailable/ISOLATION_BACKEND_MISSING` ou `IsolationUnavailable/ISOLATION_OBLIGATION_UNENFORCEABLE` et une remédiation, jamais par un repli silencieux. Les workers reçoivent un canal et un mandat d’action spécifiques ; ils n’obtiennent ni clés API générales ni accès administratif au socket. Détail : [frontières de sécurité](../../architecture/security-boundaries.md).

## AI-Guard

GuardPort est un contrat Unleash ; son trait Rust vit dans `unleash-action-broker`. Son adaptateur réel, la crate `unleash-guard-adapter` sous un `guard_mapping_profile` versionné, n’est développé qu’après lecture des schémas et code AI-Guard fournis. Un double déterministe (`unleash-testkit`) permet de tester allow/deny/require_approval/unavailable/unsupported, la révocation et la suspension étant testées à part, mais ne satisfait pas le gate d’intégration réel EXT-GUARD. La frontière retenue évalue toute action du broker hors `local_read` ; l’exécuteur AI-Guard n’est activé qu’après négociation et qualification (ADR-10).

guard_required bloque les nouvelles actions concernées en panne, fail-closed. guard_optional ne peut se replier qu’en l’absence de décision exploitable (Unavailable, timeout, protocole incompatible) et seulement si cette politique est déjà autorisée (`fallback_policy_ref`), avec trace de couverture ; un Deny, un RequireApproval ou un Allow dont une obligation est inconnue, contradictoire ou non imposable n’est jamais remplacé. broker_only reste une configuration visible, pas une fausse connexion AI-Guard saine ; c’est le défaut tant qu’aucun endpoint n’est configuré, et guard_required le devient dès sa configuration (Q-04). Valeurs par défaut configurables : EvaluateAction 2 000 ms, sans relance immédiate de l’appel expiré ; sous guard_required, l’action reste `authorizing` et le broker la réévalue après backoff (`retry_class=after_backoff`), jamais par allow implicite ; CheckHealth toutes les 30 s, réemploi d’une décision au plus 300 s. Détail : [ai-guard](../../specs/ai-guard.md).

## Secrets, données et attaques

SecretStore résout des références opaques vers coffre OS. Aucun secret général n’est passé dans argv ou env des outils du projet. Les parsers de résultats, imports de skills, previews et logs ont tailles bornées et ne sont pas évalués comme code.

La redaction s’effectue avant persistance ordinaire ; les artefacts bruts sensibles ont politique séparée et accès réduit. La suppression retire copies et index concernés selon un plan et la politique `unleash.retention.default.v1` ([modèle de stockage](../../architecture/storage-model.md)), conserve tombstones nécessaires et signale les limites des backups et services distants. Un journal local ne prétend pas résister à l’administrateur de la machine.

Les sources externes peuvent tenter une prompt injection. Les contrôles d’autorité restent indépendants de ce qu’un modèle croit : aucun texte documentaire ne change fournisseur, budget, permission, scope ou contrat engagé.

## Livraison interne

Définir d’abord les DTO communs et scénarios d’erreurs, puis implémenter le chemin nominal, les conflits de versions, les pannes et la qualification de plateforme. Conserver la logique métier dans le backend et les mécanismes I/O dans leurs adaptateurs. Les bibliothèques candidates ne sont pas des versions de dépendances déjà validées en compilation.

