# Comité d'experts : protocoles et revue réellement menée

Les quatorze fiches de ce répertoire définissent des perspectives de revue et leurs responsabilités. Leur en-tête YAML les rend invocables dans Claude Code comme sous-agents de revue en lecture seule (`expert-01-chief-architect` … `expert-14-product`, outils Read/Grep/Glob) ; les autres outils les lisent comme protocoles Markdown. Elles ne sont ni des prompts opaques ni une dépendance à un abonnement CLI. Le sous-répertoire `dev/` contient séparément les neuf rôles exécutables de développement décrits par le [protocole de goal](../dev/goal-protocol.md) ; ils outillent la construction d'Unleash et ne font pas partie de son runtime.

## Organisation de la revue initiale

Trois sous-agents spécialisés ont été réellement délégués en parallèle ; chacun a produit plusieurs analyses de domaine avant de rédiger ses spécifications. Le coordinateur a assuré les autres perspectives et la consolidation. Un quatrième sous-agent, avec un contexte de revue distinct, réalise la seconde contradiction sur les documents rédigés. Il serait inexact de présenter ce travail comme quatorze experts humains ou quatorze sous-agents tous indépendants.

| Regard | Auteur de perspective dans cette mission | Conclusion principale |
| --- | --- | --- |
| 1 Chief Software Architect | Groupe Git/Knowledge/Architecture | [Architecture](../audits/architecture-audit.md) |
| 2 Rust Systems Architect | Groupe Systems/Reliability/QA | [Rust](../audits/rust-feasibility.md) |
| 3 Workflow Engineer | Groupe Workflow/Agents/Contracts | [Déterminisme](../audits/workflow-determinism.md) |
| 4 Multi-Agent Architect | Groupe Workflow/Agents/Contracts | [Coordination](../audits/multi-agent-coordination.md) |
| 5 LLM API & Context | Coordinateur, analyse distincte | [APIs](../audits/api-compatibility.md) |
| 6 Git Engineer | Groupe Git/Knowledge/Architecture | [Git](../audits/git-concurrency.md) |
| 7 Knowledge Engineer | Groupe Git/Knowledge/Architecture | [Knowledge](../audits/knowledge-audit.md) |
| 8 Skill & Contracts | Groupe Workflow/Agents/Contracts | [Skills](../audits/skills-and-contracts.md) |
| 9 Reliability | Groupe Systems/Reliability/QA | [Reprise](../audits/persistence-and-recovery.md) |
| 10 Security / AI-Guard | Coordinateur, analyse distincte | [Menaces](../audits/security-threat-model.md) |
| 11 UX / GUI | Coordinateur, analyse distincte | [UX](../audits/gui-ux-audit.md) |
| 12 QA | Groupe Systems/Reliability/QA | [Vérification](../audits/qa-verification.md) |
| 13 Adversarial | Première contradiction coordinateur ; seconde par sous-agent distinct | [Première](../audits/adversarial-findings.md), [seconde](../audits/final-independent-review.md) |
| 14 Technical Product Architect | Coordinateur de consolidation | [Verdict](../audits/consolidated-verdict.md) |

## Protocole futur

Une revue reçoit un paquet limité : objectif, REQ/INV, commit ou version documentaire, contrats affectés, hypothèses, décisions proposées et preuves existantes. L'expert doit rendre constats, risques, objections, alternatives, recommandations, vérifications manquantes et acceptation. Une critique sans scénario ou source est une question, pas une preuve d'échec.

Au cadrage, Chief + Product définissent frontières et propriétaires. Avant une interface nouvelle, domaine + Security/Reliability la challengent selon risque. Avant merge, QA vérifie les critères choisis pour le projet et l'expert inspecte le diff. Avant release, un reviewer indépendant contrôle les interactions et la couverture V1.

L'activation se fait sur événement utile : changement de protocole, nouveau privilège, incident, écart mesuré ou jalon. Ces experts ne constituent pas une boucle permanente de juges LLM. Ils ne sont pas automatiquement les N+1/N+2 métier de l'application.

## Gestion des désaccords

Le finding conserve auteur, faits, hypothèses, scénario, option recommandée et alternative. Le propriétaire répond avec changement ou justification. Le Product/Chief consolide l'ADR ; Louis/OVPT tranche une modification d'invariant ou une décision réservée au porteur. La clôture exige une preuve adaptée, même lorsque le désaccord est volontairement conservé comme risque accepté.


## Révision R2 — mandat réel

Après clarification utilisateur sur Rust, Kivy/Tauri et parité GUI/TUI, deux sous-agents spécialisés ont réellement révisé les volets GUI et terminal. Le volet GUI a bénéficié d’une relecture distincte limitée à ses neuf documents. Un autre reviewer indépendant a ensuite comparé la révision consolidée, les parcours et les contrats communs ; le coordinateur a intégré les corrections et régénéré la couverture. Ce travail ciblé ne constitue pas une nouvelle exécution des quatorze audits initiaux. Son résultat est dans [frontend-terminal-revision-review](../audits/frontend-terminal-revision-review.md).

## Révision R6 — délégation distincte des quatorze expertises

La dernière demande de Louis a conduit à une nouvelle revue : **quatorze instances d’agents spécialisées ont réellement reçu des mandats distincts**, correspondant aux quatorze expertises demandées. Elles ont lu les contrats du dossier et produit les [quatorze rapports R6](../audits/r6/README.md). Les douze premières revues précèdent leurs corrections ; le reviewer contradicteur E13 intervient sur la consolidation et E14 vérifie le périmètre produit. Plusieurs spécialistes relisent ensuite les corrections de leur domaine. Le coordinateur intègre les arbitrages, contrôle les raccords et exporte les projections.

Les trente constats de cette passe sont enregistrés avec auteur, sévérité, correction, contrat et recette attendue. Une [contrelecture de remise distincte](../audits/r6/release-readthrough.md) a trouvé puis relu un raccord supplémentaire sur l’ordre d’idempotence après restauration. Ces agents sont des instances IA ; aucun nombre d’experts humains, diversité de modèles ou indépendance de formation n’est revendiqué. Les rapports historiques restent conservés et ne doivent pas être confondus avec cette nouvelle exécution.

La clôture d’un finding signifie correction de conception et, lorsque mentionné, contrôle des artefacts documentaires. La preuve du runtime Rust reste `not_run`. La [validation du dossier](../testing/document-validation.md) et les gates produit ont des périmètres distincts. Les fiches d’experts de ce répertoire restent des protocoles mobilisables pendant le développement ; leur simple import ne lance aucun agent ni n’accorde de permission au-delà de celles de la session qui les invoque.

## Révision R7 — revues par domaine

La révision R7 a conduit seize revues par domaine, chacune contre-vérifiée par un vérificateur indépendant ([révision R7](../brief/revision-2026-11-16-r7.md)). Les constats confirmés sont corrigés dans les documents canoniques ; les audits historiques ne sont pas réécrits : ils reçoivent au plus une note datée « Note R7 » vers la correction canonique, et seules leurs erreurs factuelles sur l’état actuel sont corrigées. La R7 ajoute aussi les en-têtes YAML des quatorze fiches et les rôles `dev/` décrits ci-dessus. Les tests du produit restent `not_run`.
