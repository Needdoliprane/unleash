# AI-Guard : frontière retenue côté Unleash, contrat externe à qualifier

Statut : contrat Unleash adopté ([ADR-10](../architecture/adr/ADR-10-ai-guard-boundary.md) accepté, Q-04 résolue R7 par arbitrage délégué) ; **aucune API AI-Guard existante confirmée**. L’intégration et son administration appartiennent à V1. Pièces attendues et gate EXT-GUARD : [registre](../brief/open-questions.md). [F017](../features/F017-ai-guard-integration/README.md).

## Autorités indépendantes

Unleash reste responsable de l’identité logique, du mandat, des limites budgétaires, de la validation des commandes, de la corrélation des traces et de l’état de reprise. AI-Guard peut apporter décision de sécurité, contrôle d’exécution, isolation et audit selon son contrat réel. Une décision « allow » d’AI-Guard n’autorise pas Unleash à dépasser un budget, appliquer une transition illégale ou changer le workflow.

L’utilisateur configure guard_mode : broker_only, guard_optional, guard_required. `broker_only` remplace l’ancien libellé `local_only` et signifie « broker Unleash seul, sans AI-Guard ». Défaut : `broker_only` tant qu’aucun AI-Guard n’est configuré, puis `guard_required` dès qu’un endpoint est enregistré par ConfigureGuard. Ce sont des états du côté Unleash, pas des modes déclarés du produit AI-Guard. Les politiques peuvent exiger une isolation précise par action, en plus de l’`isolation_mode` du projet ([frontières de sécurité](../architecture/security-boundaries.md)) ; si cette capacité est indisponible, l’action reste bloquée (IsolationUnavailable) même en broker_only.

## Port interne à implémenter en Rust

GuardPort expose conceptuellement NegotiateCapabilities, CheckHealth, EvaluateAction, RevokeSubject, ReadPolicySummary, ReadAuditReference et, si négociés, SuspendSubject/ResumeSubject et ExecuteAuthorizedAction. La suspension et sa levée existent toujours localement par les commandes Unleash du même nom ; GuardPort n’en assure la propagation distante que s’il la négocie. Ces noms définissent l’interface Unleash ; les endpoints, payloads externes et mécanismes d’auth restent à mapper sur les fichiers réels. Le trait Rust `GuardPort` vit dans `unleash-action-broker` ; l’adaptateur réel est la crate séparée `unleash-guard-adapter`, écrite seulement à partir des schémas fournis et identifiée par un `guard_mapping_profile` versionné ; un double déterministe vit dans `unleash-testkit`.

EvaluateAction reçoit action_id, subject {kind invocation|replay|control_plane, principal_id, cause_ref}, agent_id?/role_version?/mission_id? selon kind, project_id, scope_revision, verb, resource_refs, payload_digest (= `actions.payload_hash`), effects_class, policy_revision, action_deadline et isolation_requirements (dérivées de l’autorité effective). Réponse normalisée : Allow(decision_ref, constraints, expiry), Deny(reason_code, decision_ref), RequireApproval(approval_scope), Unavailable ou UnsupportedCapability. L’expiry d’un Allow borne la décision et reste distincte d’action_deadline. Une réponse malformée est un échec, pas une autorisation.

`effects_class` suit une liste fermée versionnée : `local_read`, `local_write`, `process_exec`, `terminal_input`, `git_local_write`, `git_remote`, `connector_read`, `connector_write`, `browser_action`, `model_dispatch`, `network_egress`. Une classe inconnue est refusée avant GuardPort (`InvalidArgument/EFFECTS_CLASS_UNKNOWN`) ; ajouter une classe crée une nouvelle version du mapping.

Le résultat est lié au digest exact de l’action. Changer commande, cible, environnement, réseau ou scope invalide la décision. Une décision n’est réemployée que pour un digest, une policy_revision et une scope_revision identiques, avant l’expiry retournée et au plus 300 s ; sans expiry, aucun cache. Une décision mise en cache ne sert jamais après révocation, suspension ou changement de policy_revision.

## Flux d’exécution

1. Une commande authentifiée produit une action validée localement et un intent persistant.
2. Soumise au broker, l’action passe en `authorizing` ; le broker évalue capacités et permissions locales, puis appelle GuardPort pour toute action hors `local_read` lorsque guard_mode vaut guard_optional ou guard_required.
3. Il persiste la décision et les contraintes sans libérer de secret aux agents.
4. Juste avant exécution, il vérifie fencing, révision du mandat, expiration et droits toujours valides.
5. L’exécuteur autorisé lance l’action ; événements et preuves portent les deux identifiants de corrélation.
6. Une fin ambiguë devient OutcomeUnknown ; la réconciliation utilise les preuves locales et, si disponible, la référence distante.

Issues de l’évaluation et projection wire :

- Allow sans contrainte additionnelle → `authorized`. Sous le profil initial `obligations=none_accepted`, tout autre Allow → `CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED`.
- Deny → `CapabilityDenied/GUARD_DENIED`, `retry_class=never` pour ce digest.
- RequireApproval → `approval_required`, sans secret ni dispatch, seulement si le `guard_mapping_profile` qualifie la remise de l’approbation au protocole réel (`GUARD_APPROVAL_REQUIRED`, `retry_class=after_human_decision`) ; la décision humaine passe par ResolveActionApproval, sur l’approval_scope et le digest exacts. Sinon → `CapabilityDenied/GUARD_APPROVAL_UNSUPPORTED`.
- Unavailable, timeout ou réponse malformée → `CapabilityDenied/GUARD_UNAVAILABLE`, `retry_class=after_backoff`. Sous guard_required, l’action reste en `authorizing`, fail-closed ; le broker la réévalue après backoff, sans relance immédiate de l’appel expiré ni allow implicite.
- UnsupportedCapability ou version non négociée → `ProtocolIncompatible/GUARD_PROTOCOL_UNSUPPORTED`.

Deny, obligation insatisfaite, approbation refusée ou non prise en charge et refus de politique locale (`CapabilityDenied/POLICY_DENIED`), y compris celui du repli guard_optional, mènent l’action à `denied`, terminal sans dispatch ni secret résolu, et émettent ActionDenied ; un protocole incompatible fait de même sous guard_required. Sous guard_optional, une indisponibilité ou un protocole incompatible déclenche seulement le repli défini plus bas. Les états d’action sont ceux de la table `actions` du [schéma SQLite](../architecture/sqlite-design.md).

Aucune transaction SQLite n’est ouverte pendant l’appel AI-Guard. La double vérification n’élimine pas toute course d’une révocation pendant l’effet : le contrat doit définir point d’engagement et comportement d’annulation.

## Frontière retenue

| Frontière | Coût / disponibilité | Contournement possible | Condition d’acceptation |
| --- | --- | --- | --- |
| Actions sensibles | Peu de décisions externes | Action composite mal classée | Classificateur versionné, coverage visible, tests de contournement |
| Tous effets initiés par harness | Surcoût par action, backpressure nécessaire | Descendants d’un shell non confinés | Autorisation par action + garanties OS déclarées |
| Broker obligatoire | Forte dépendance AI-Guard et protocole | Sorties directes si worker garde accès hors broker | Suppression des capacités ambiantes et preuve d’isolation |

Décision (Q-04, ADR-10) : l’ActionBroker interne reste l’unique point de lancement des effets (REQ-SEC-002). Lorsque AI-Guard est configuré, GuardPort évalue chaque action du broker sauf la classe `local_read`, ce qui correspond à la deuxième ligne du tableau ; la V1 n’offre pas de frontière « actions sensibles seulement ». L’exécuteur par défaut est `executor=unleash` ; le profil `executor=guard` (ExecuteAuthorizedAction, troisième ligne) ne s’active que si NegotiateCapabilities l’annonce et après qualification. Le tableau reste la justification de ce choix. Endpoints, payloads, authentification et sémantique des contraintes du vrai AI-Guard ne sont pas décidés faute de pièces : ils relèvent du gate EXT-GUARD et de la recette F017, sans retrait de la V1.

Valeurs par défaut configurables : délai d’EvaluateAction de 2 000 ms, sans relance immédiate de l’appel expiré ; sous guard_required, l’action reste `authorizing` et le broker la réévalue après backoff (`retry_class=after_backoff`), jamais par allow implicite ; CheckHealth toutes les 30 s ; réemploi d’une décision au plus 300 s. Transport : socket Unix local avec identité du pair, ou HTTPS avec TLS vérifié ; mTLS seulement si le protocole réel l’exige ; credentials par SecretRef. Configuration par ConfigureGuard, lecture par ReadGuardHealth et ReadPolicySummary.

## Administration et incidents

Settings affiche endpoint/configuration, identité du client, version du protocole négociée, capacités, `guard_mapping_profile`, santé, dernière policy_revision et état des autorisations. L’utilisateur voit les incidents, peut suspendre un sujet (SuspendSubject) puis lever la suspension (ResumeSubject), révoquer des permissions (RevokeSubject) et inspecter les références d’audit. La suspension locale bloque immédiatement les nouvelles actions du sujet et reste réversible ; les effets en vol sont réconciliés ; sa propagation distante n’a lieu que si GuardPort la négocie, sinon la limitation est visible. Une modification de politique à distance n’est proposée que si l’API la supporte et que l’identité possède ce droit ; sinon consultation et lien vers administration externe, avec limitation visible.

guard_required indisponible → nouvelles actions concernées bloquées (`GUARD_UNAVAILABLE`), missions et logs restent inspectables. guard_optional exige une `fallback_policy_ref` préautorisée et versionnée, qui ne peut qu’appliquer la politique locale ; chaque repli émet un événement et l’avertissement de mode. Aucun timeout ne devient un allow silencieux.

La recette réelle exige tests d’autorisation, de révocation, d’arrêt, de panne, de corrélation et de non-contournement. Un double de test permet de construire le port mais n’est pas une preuve d’intégration AI-Guard.

## Application des obligations Allow — R6

Une réponse Allow conditionnelle n’est pas une autorisation inconditionnelle. Le port Unleash mappe les constraints externes vers un catalogue **interne** versionné d’obligations typées. Ce mapping ne prétend pas décrire l’API AI-Guard encore inconnue. Chaque obligation doit avoir un mécanisme, un composant responsable, une frontière d’application, une durée, une condition d’échec et une preuve de capacité applicable à l’action exacte. Limites locales et obligations s’intersectent ; aucune obligation n’élargit les grants.

Avant résolution de secrets et dispatch, le broker vérifie la totalité des obligations. Inconnue, contradictoire, non imposable ou profil de mapping indisponible → `CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED`, zéro dispatch, audit expurgé. Une obligation continue (réseau, isolation ou durée) exige un mécanisme imposable pendant l’exécution ; sa perte suspend les effets futurs et déclenche arrêt/réconciliation selon le mandat. Une ligne de log ou une intention de prompt n’est pas ce mécanisme.

Le journal associe décision, action/digest, version du mapping, liste d’obligations, enforcement refs, instant d’engagement et raison de refus/incertitude. Toute modification pertinente ou perte de capacité avant lancement invalide l’application. Le repli de guard_optional ne s’applique qu’en l’absence de décision exploitable (Unavailable, timeout, protocole incompatible). Il suit alors sa politique préautorisée et produit une décision locale distincte, avec le changement de mode visible. Un Deny, un RequireApproval ou un Allow dont une obligation est inconnue, contradictoire ou non imposable n’est jamais remplacé par cette décision locale, quel que soit guard_mode. Profil initial décidé : `obligations=none_accepted` ; tout Allow assorti d’une contrainte additionnelle est refusé `GUARD_OBLIGATION_UNSATISFIED` jusqu’à qualification du mapping réel (EXT-GUARD). Qualifier un profil d’obligations plus large crée une nouvelle version du mapping, sans rouvrir la décision.

Le sujet d’action est typé invocation/replay/control_plane, avec principal et cause authentifiés ; agent_id/role/mission sont présents seulement lorsqu’applicables. Une automatisation mécanique ou un probe ne fabrique pas d’agent pour demander une autorisation. Les informations minimales équivalentes de scope, propriétaire et mandat restent requises.
