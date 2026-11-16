# Verdict consolidé — audit architectural OVPT Unleash

**Perspective 14 : Technical Product Architect, avec consolidation des quatorze regards.** Date : 16 novembre 2026. Statut : conception proposée, après seconde contradiction documentaire. Aucun produit déjà construit n'est certifié.

> **Note R7 (16 novembre 2026) :** ce verdict décrit l’état R1–R6. Les arbitrages qu’il laisse ouverts (frontière de migration Q-03, GUI, AI-Guard, modèle de menace, licence, OAuth, branches checkout, OS minimaux, rétention, calendrier) sont rendus par l’arbitrage délégué R7, et les dix-neuf ADR sont acceptés ([révision R7](../brief/revision-2026-11-16-r7.md), [registre](../brief/open-questions.md), [ADR](../architecture/adr/README.md)). Ce qui reste relève des gates de preuve de la [readiness checklist](../plans/readiness-checklist.md).

## 1. Synthèse critique initiale

La vision est cohérente si Unleash reste un système d'exécution local dont les décisions LLM sont des entrées contrôlées. Elle devient fragile si « intelligence collective », « reprise de session » ou « compilation de skill » servent à masquer des garanties impossibles à établir.

La recommandation conserve un daemon Rust utilisateur, SQLite, des workers éphémères, des process-hosts ciblés pour les PTY/services, un broker d'actions et des clients sans logique métier autonome. Elle évite Redis, un cluster distribué, un event sourcing intégral et une nouvelle VM de plugins. Cette simplicité ne supprime pas les protocoles nécessaires : intention durable, identité d'action, autorité, effet observé, résultat inconnu, réconciliation et version de contrat.

Le périmètre V1 est vaste. Les dix écrans, trois connecteurs, moteur de workflows, skills/rôles, connaissance multilangage, intégration Git et sécurité représentent un programme de construction, pas un petit prototype. Le dossier propose un ordre M0–M7 sans retirer de fonctions ni promettre un calendrier non étayé.

## 2. Ce que les audits ont réellement changé

| Sujet | Objection | Arbitrage proposé | Alternative et coût |
| --- | --- | --- | --- |
| Persistance | SQLite ne rend pas atomiques Git, HTTP et fichiers | État+journal+outbox/inbox transactionnels ; journal d'actions séparant effet et preuve | Event sourcing intégral : replay/migrations plus complexes, sans résoudre atomicité externe |
| Workflows | « Prochaine invocation » permettrait hot swap rétroactif | Définition immuable pendant run ; adoption au run suivant terminal/quiescent | Migration explicitement mappée après frontière plus fine ; contrat et tests supplémentaires |
| Rôles/skills | Tout épingler au début empêcherait création dynamique utile | AST/engagements figés ; slots tardifs déclarés, exacts et append-only | Tout précomposer : moins de variabilité, moindre autonomie dans mission |
| Manager | Cinq N attendent leur N+1 en occupant tous les slots | Checkpoints et attentes durables rendent capacité ; priorités/équité | Réserver strictement un slot : meilleure disponibilité, débit N réduit |
| Knowledge | Deux scans dirty identiques ne prouvent pas un instant cohérent | Provenance et capture_consistency distinctes ; découverte ≠ disponibilité | Snapshots FS atomiques qualifiés : portée OS et coût supplémentaires |
| Skills | Le texte du prompt ne se compile pas en garantie de qualité | Schémas + catalogue fermé de règles sur preuves ; entrée et profil exacts | Sous-typage fermé de profils, plus expressif mais à formaliser/tester |
| Git | Lease interne n'empêche ni Git externe ni cible checkout | Candidat exact, CAS old/new OID, stratégie d'adoption explicite | Forge/merge queue selon capacité ; dépendance réseau et contrat externe |
| Reprise | PID et relance du fork peuvent répéter un effet déjà réussi | Identité complète, attempts/fences, OutcomeUnknown puis réconciliation | Isolation plus forte/système de jobs externe : coût d'exploitation accru |
| AI-Guard | Une API inventée rendrait l'intégration illusoire | Port Unleash proposé et qualification réelle obligatoire | Broker externe obligatoire : dépendance forte disponibilité/performance |
| GUI et TUI | Un framework ne garantit ni beauté, ni accessibilité, ni parité | R2 confirme bibliothèques/HTML/CSS et parité complète ; Tauri et Ratatui/Crossterm proposés, comparatif et gates | GUI native ou Rust/WASM comparés selon coût ; aucune fonction avancée réservée à la GUI |

Ces décisions sont détaillées dans les [ADR](../architecture/adr/README.md). Les objections initiales sont conservées dans les audits, avec leur résolution ; aucune unanimité artificielle n'est revendiquée.

## 3. Les quatre zones à haut risque

### 3.1 Connaissance entre worktrees

Le moteur ne doit pas créer une fiction de code partagé déjà intégré. Une ressource trouvée porte repository, commit/tree ou manifeste dirty, génération, état d'intégration, disponibilité locale, propriétaire et fraîcheur. Les agents voient un helper en développement ailleurs et peuvent attendre, coordonner, copier avec provenance ou créer un composant distinct justifié.

Le reçu de découverte est un objet du harness. Une création structurée identifiable peut exiger ce reçu avant effet. Une fonction découverte après un patch opaque est marquée a posteriori ; on ne falsifie pas la chronologie pour prétendre que l'agent a cherché avant. Un rebase ou une mutation invalide les reçus affectés. Les ACL filtrent aussi compteurs et extraits.

La connaissance combine des projections spécialisées code/travail/preuves autour d'identités et de sources communes. Git reste autoritaire sur le code, le journal sur les activités enregistrées, les index sur leur génération dérivée. Les faux positifs sémantiques sont inévitables ; le jugement de domaine reste au N+1. Tests discriminants : helper non mergé indisponible localement, dirty changé pendant capture, faux ami métier, cycle d'attente concurrent, rebase après discovery.

### 3.2 Invocation du N+1

Une invocation logique est une unité durable et bornée de décision/travail ; une tentative est son incarnation de worker ; un appel API est une action distincte. Le N+1 ne possède pas une boucle d'orchestration cachée qui avance les étapes par prose. Il soumet des commandes autorisées sur une révision et des preuves. Le reducer applique la transition.

La publication d'un workflow v2 n'affecte pas le run v1. Le cycle retenu par défaut correspond au run complet, branches et effets réconciliés. Une invocation nouvelle à l'intérieur de ce run reste sous v1. Les rôles/skills déjà engagés ne changent pas ; un slot explicitement prévu mais vierge peut recevoir un binding exact compatible. Ce point préserve l'autonomie demandée sans modifier rétroactivement l'exécution.

La question de la frontière finale reste Q-03 : il s'agit d'une proposition explicite, pas d'une décision attribuée à Louis. Les délais de réveil dépendent de slots, quotas, checkpoints et disponibilité API. Le contrôle déterministe peut arrêter/révoquer sans attendre un manager LLM. Tests : plusieurs branches actives, publication en pleine revue, ancien résultat leader, cinq enfants attendant leur parent, baisse de 50 à 5.

### 3.3 Reprise après crash

La session conserve entrées, décisions et références ; elle ne sauvegarde pas la RAM du fork. Le worker repris garde invocation_id et reçoit attempt_id/fence nouveaux. Une commande externe dispatchée sans résultat durable reste OutcomeUnknown. Certaines opérations ne sont pas retentables automatiquement : publication externe sans idempotence vérifiable, entrée terminal interactive, action réseau dont l'effet ne peut être observé, opération financière ou administrative opaque.

Le process-host peut conserver PTY et spool lors d'un crash du daemon, sous réserve de qualification OS. Il ne promet ni survie au reboot ni récupération d'un maître PTY détruit. PID, heure de démarrage et boot renforcent l'identification ; le canal authentifié et la lignée d'autorité déterminent ce qui peut encore agir.

La seconde revue a trouvé un cas plus subtil : restaurer un backup avec epoch40 alors qu'un host a déjà accepté100. Un simple incrément à41 ne suffit pas. La proposition impose restauration offline, neutralisation vérifiée des anciens exécuteurs, nouvelle authority_generation et nouveaux secrets ; aucune adoption à chaud des vieux hosts. Une inconnue locale bloque l'admission. Un nouveau boot supprime les vieux processus, pas leurs effets distants. Ces règles restent à tester sur OS réels.

### 3.4 Compilation des skills en contrats

Un skill contient des instructions et un contrat ; les instructions ne deviennent pas des prédicats exécutables par magie. Le compilateur vérifie le format, les références, les permissions et un catalogue fermé de contrôles. Le moteur peut prouver qu'une commande s'est terminée, qu'une sortie satisfait un schéma, qu'un patch correspond à des fichiers observés ou qu'une preuve vise un snapshot. Il ne peut garantir que la nouvelle abstraction est bonne.

Le rôle désigne entry_skill, référence exacte appartenant à ses skills. Le binding tardif V1 exige digests input/output exacts, conservation des règles obligatoires, extensions seulement conjonctives autorisées et intersection des capacités. Compilateur, évaluateur et catalogue sont épinglés avant exécution ; leur indisponibilité bloque une reprise qui prétendrait conserver la sémantique.

N+2 gouverne la création/publication, mais peut déléguer la rédaction technique. Les évaluations sont proportionnées au contrat de publication. Aucun second juge LLM ni validation humaine permanente n'est imposé. Tests : claim de test fausse, schéma conforme mais preuve absente, règle retirée, entrée ambiguë, adaptation autorisée et permissions étendues refusées.

## 4. Conclusions identifiables des experts

Les méthodes et attributions sont dans [agents/README](../agents/README.md). Trois groupes ont porté neuf regards ; le coordinateur a porté API, sécurité, UX et produit ; un sous-agent distinct a réalisé la seconde contradiction. Ce sont quatorze perspectives, pas quatorze personnes indépendantes.

| Regard | Conclusion déterminante | Réserve conservée |
| --- | --- | --- |
| Chief | Ports et identités partagés, un noyau local | Ne pas créer une crate/service par mot du brief |
| Rust Systems | Faisabilité du code applicatif Rust ; async et bloquant séparés | Dépendances natives et GUI à qualifier |
| Workflow | AST fermé, bornes, reducer pur et gardes unknown | Liberté utilisateur dans un profil explicitement limité |
| Multi-agent | Owners uniques, décisions révisionnées, managers événementiels | Autonomie n'élimine pas le coût organisationnel |
| API | Adaptateurs natifs et profils de capacité explicites | Documentation publique ≠ accès de compte |
| Git | Candidat exact et CAS ; cible checkout traitée | Conflits fonctionnels non garantis détectables |
| Knowledge | Provenance/fraîcheur et recherche tracée | Similarité et capture dirty ont des limites |
| Skills/contracts | Validation formelle séparée du jugement | Publication autonome ne prouve pas compétence parfaite |
| Reliability | Actions inconnues et restauration conservatrice | Reprise universelle transparente impossible |
| Security | Broker, autorité et isolation distincts | Même UID et shell arbitraire restent frontière critique |
| UX | Supervision par missions/décisions/preuves ; design soigné | Framework ne se choisit pas sur promesse |
| QA | Oracles indépendants, cutpoints et traces par préfixe | Tests live/hardware indispensables à qualification |
| Adversarial | Huit contradictions inter-documents corrigées après relecture | Correction documentaire ≠ test produit réussi |
| Product | Socle initial 18 features, 199 exigences ; R2 210 ; R3–R5 étendent à19 features et couverture courante vérifiée | V1 large et dépendances externes à gérer tôt |

## 5. Seconde revue et corrections

[La revue finale indépendante](final-independent-review.md) documente huit findings : états d'invocation divergents ; vocabulaire discovery ; IPC/causalité ; fallback qui modifiait un bundle figé ; restauration d'un ancien domaine de fencing ; compatibilité de contrat non algorithmique ; baisse de concurrence ambiguë ; historique de rapport non reconstructible.

Ils ont été corrigés dans les documents et relus indépendamment. Les tests proposés couvrent maintenant ces contre-exemples. Les contrôles JSON et de références exécutés vérifient les artefacts de conception ; ils ne démontrent pas le comportement d'un daemon inexistant. Le [rapport documentaire global](../testing/document-validation.md) fournit les comptes et résultats exacts.

## 6. Décisions résiduelles

Le [registre des questions](../brief/open-questions.md) précise propriétaire, statut et gate. Q-01 et Q-17 sont résolues par R2 : dépendances et HTML/CSS autorisés, références Kivy et Tauri identifiées. Restent à qualifier le framework GUI et son renderer, l’adoption des nouvelles versions, la frontière AI-Guard et le modèle de menace, la licence open source, le parcours OAuth distribué, l’adoption des branches checkout, les OS minimaux et la rétention. La parité GUI/TUI et la qualité du terminal sont des exigences confirmées, pas des options de cette sélection.

La base de conception est exploitable pour M0 et les travaux indépendants. Elle n'autorise pas une équipe à décider silencieusement ces points à la place d'OVPT. L'absence du dépôt AI-Guard et de clés de test ne justifie pas d'inventer une API ni de marquer la V1 complète comme déjà acceptée.

## 7. Construction et preuve

Le [plan V1](../plans/v1-implementation-plan.md) commence par identités, autorité, persistance et expériences GUI/reprise/knowledge. Viennent runtime/API/processus, puis workflows/équipes/skills, Git/connaissance, expérience et connecteurs, qualification AI-Guard et recette transversale. F018 et le design commencent dès le socle ; ils ne sont pas des réparations tardives.

Les équipes disposent de contrats, erreurs, états, dépendances et critères par feature. Les matrices relient invariants, exigences, architecture, tests et preuves attendues. Le futur Unleash pourra organiser son développement à partir de ce dossier, à condition de conserver la distinction entre une exigence, une déclaration de réussite et sa preuve observée.

## Mise à jour R2 — interfaces et politique d’implémentation

La clarification utilisateur change le périmètre de l’interdiction de langage, sans retirer le cœur Rust : réutilisation des moteurs/bibliothèques, HTML/CSS et choix de présentation motivés sont autorisés. Le coût d’une GUI web et sa frontière d’autorité sont explicitement comparés. Kivy est identifié comme référence Python ; Tauri devient le candidat GUI de référence proposé. Le terminal devient une TUI complète, avec Ratatui/Crossterm proposés, et la CLI scriptable reste un troisième mode d’accès.

Cette révision ajoute les exigences de parité et de qualité terminal sans déclarer leur implémentation réalisée. Les parcours et erreurs doivent être prouvés dans les dix espaces, y compris édition et configuration. Deux analyses spécialisées puis une [revue contradictoire ciblée](frontend-terminal-revision-review.md) complètent les audits initiaux ; elles ne sont pas présentées comme une nouvelle exécution des quatorze regards d’origine.

## Mise à jour R3–R5 — procédures, supervision, local et guidage

La spécialisation développement est conservée avec une capacité généraliste réutilisant le même moteur. F019 ajoute des artefacts de procédure et de données, des actions sans agent dans le profil v1.1 et deux modes d’exécution : sans modèle, ou supervisé à des points publiés. Le modèle peut améliorer le traitement d’une exception ; les logs seuls ne prouvent pas la réussite métier. Les assertions et le journal d’effets empêchent qu’un avis remplace une observation manquante.

Le parcours guidé prépare les objets nécessaires à partir d’un objectif ou d’un template ; aucune notion d’agent, de scope ou de workflow n’est exigée au premier usage. Les mêmes réglages et fonctions existent en TUI. Un template de qualité proposée ne devient pas un test/audit universel imposé à tous les workflows. Les utilisateurs peuvent choisir et modifier les contrôles adaptés à leur mission.

Les modèles locaux sont natifs dans le Router, avec authentification adaptée, dialectes, capacités et ressources explicites. local_only exige une preuve de placement : localhost peut servir de point d’entrée à un calcul distant. L’exécution forbidden exclut modèles locaux comme distants. Le coût nul du replay ne masque pas les coûts de conception ou de réparation distincts.

Les [analyses navigateur](browser-automation-audit.md) et [produit généraliste](general-purpose-product-review.md), la [recherche locale](../research/local-model-sources.md) et les ADR-16/17 préservent leurs objections. La V1 compte dix-neuf features. La couverture actuelle et les nombres vérifiés sont dans [document-validation](../testing/document-validation.md), sans confondre ces contrôles avec des tests produit. Les nouveaux scénarios W/X/Y/Z portent sur le comportement réel à construire.

## Mise à jour R6 — contrats confrontés à quatorze expertises réelles

Les [quatorze rapports R6](r6/README.md) sont issus d’instances spécialisées réellement déléguées. Trente findings ont conduit à des corrections de conception ; les constats initiaux et leurs alternatives sont conservés. La [contrelecture de remise](r6/release-readthrough.md) a fermé un raccord supplémentaire de l’algorithme de commande. Le [verdict produit E14](r6/expert-14-product.md) ne relève pas de nouvelle suppression de périmètre et conserve les gates externes.

Les quatre sujets prioritaires sont précisés : connaissance multi-worktrees avec snapshots typés et disponibilité locale distincte ; invocations de managers bornées sans moteur parallèle ; effets inconnus et commandes liées à la lignée de restauration ; contrats fermés dont une exécution de test attendue est scellée avant l’effet. La découverte préalable survit comme preuve historique à la création qu’elle autorise, tandis que sa fraîcheur actuelle reste réévaluable. Un lien vers un log ne vaut pas validation d’une affirmation.

La revue a aussi corrigé la concurrence entre racines de données, la course PID/signal, le probe fournisseur sans faux agent, les restrictions héritées par les résumés, les refs Git symboliques/de remplacement, la réauthentification OAuth, les obligations d’autorisation et les reprises du guidage. Le profil de workflow v1.2 formalise les lignes et lots d’un jeu de données, leurs résultats et leurs barrières d’incertitude. Ces précisions utilisent le noyau commun et ne créent aucun service distribué supplémentaire.

Pour l’application consommatrice, le [catalogue structuré](../catalog/README.md) et le [contrat d’ingestion](../specs/document-package-ingestion.md) rendent les identifiants, relations, opérations et statuts vérifiables. L’inventaire et les empreintes détectent la dérive du paquet ; les comparaisons bidirectionnelles détectent une exigence omise, un doublon ou une chaîne permutée. L’import prépare la consultation et la planification sous mandat ; il ne produit pas de preuve métier ni d’autorisation d’exécuter les textes.

La [validation documentaire](../testing/document-validation.md) donne les résultats effectivement exécutés sur le paquet stabilisé. Les 239 preuves produit demeurent attendues. M0 reste nécessaire pour adopter les arbitrages réservés, figer les DTO communs et qualifier les hypothèses par prototypes ; M1–M7 réalisent toute la V1. Les limites AI-Guard, GUI, OAuth, OS, modèles, navigateur et licence sont maintenues explicitement.
