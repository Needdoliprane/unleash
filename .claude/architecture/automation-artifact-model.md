# Automatisations réutilisables — artefacts, données et exécutions

**Statut : contrat proposé R3, complété par la supervision R5 et par les décisions R7 Q-15 et Q-18 ; aucun runtime ni test produit réalisé.** L'intention confirmée est d'apprendre une procédure avec un LLM lorsque cela est utile, d'en faire une automatisation paramétrée puis de choisir entre replay sans modèle et orchestration supervisée depuis ses logs/résultats. Les choix de représentation, de limites et de qualification ci-dessous sont des propositions d'architecture. Ils complètent le [profil d'exécution](automation-execution-profile.md), le [DSL existant](workflow-dsl.md), les [sessions](session-model.md), les [contrats](skills-contract-model.md) et la [reprise](failure-recovery.md).

## Objets et sources de vérité

| Objet | Responsabilité et identité |
|---|---|
| `AutomationDraft` | Révision éditable d'un candidat, issue d'une saisie, d'une trace, d'un import ou d'un travail `learn`/`repair`. Il n'accorde aucun droit et ne constitue pas une version exécutable publiée. |
| `AutomationArtifact` | Version publiée immuable, identifiée par `artifact_id`, entier `version` et digest du bundle canonique. Elle décrit procédure, paramètres, contrats, dépendances et limites. |
| `AutomationDataRevision` | Snapshot immuable des données d'entrée : octets importés, profil de parsing, mapping, valeurs typées, identités de lignes et digests. Modifier un CSV crée une autre révision. |
| `AutomationRun` | Projection enrichie du `Run` existant dont `source` vaut `automation_version`. Ce n'est ni un second agrégat d'orchestration, ni un agent. |
| Manifeste de replay | Manifest du run contenant versions et données exactes, `model_policy`, contrats, profil d'isolation, grants référencés, budgets et sous-manifestes de drivers. |
| Évaluation de candidat | Run de qualification référencé par le digest exact du draft ; conserve scénario, environnement, observations, résultats métier et limites. Son existence ne garantit pas tous les futurs résultats. |

Les registres, objets adressés par contenu, commandes révisionnées, événements/outbox et contrôles d'accès réutilisent F002/F007/F008. La découverte utilise `DiscoverResources` avec le type `automation` ; un contrat peut exiger ce reçu par le validateur `discovery_receipt_present` avec la catégorie `automation` ([discovery](../specs/shared-knowledge.md#discovery--invariant-transversal)). Une décision de réutilisation ou de création et les candidats pertinents précèdent une nouvelle identité, selon le [contrat de discovery](skills-contract-model.md). Le registre d'automatisations ajoute une catégorie d'objet, pas une base ou une autorité concurrente.

## Deux modes d'exécution publiés et deux jobs de conception

| Mode ou job | Usage | Politique modèle et frontière |
|---|---|---|
| `learn` (job de conception) | Observer une tâche autorisée, relever actions/locators, proposer une procédure et ses paramètres. | `allowed_bounded` seulement avec un mandat de conception explicite et le budget agent habituel ; `forbidden` reste possible pour une construction manuelle. Le résultat est un draft. |
| `replay` | Exécuter une version publiée avec une révision de données. | `model_policy=forbidden` obligatoire, hérité par tous les descendants contrôlés ; compilation, sélection de branche, validation, reprise et rapport sont mécaniques. |
| `supervised` | Faire exécuter les blocs par le script/driver puis solliciter un superviseur sur des résultats utiles ou une exception déclarée. | `allowed_bounded`, tâches `leader_decision` explicites entre les blocs, fréquence et budgets publiés. Le LLM ne pilote pas chaque clic et ne remplace pas les assertions métier. |
| `repair` (job de conception) | Examiner un échec ou une dérive et préparer une nouvelle version. | Nouveau travail explicitement autorisé, qui peut utiliser `allowed_bounded`. Une politique préautorisée peut le déclencher dans son périmètre et son budget. Le run source demeure figé ; l'échec seul n'accorde aucun droit modèle. |

`AutomationRun.model_policy` est la projection de la politique effective du `Run`. Les valeurs canoniques sont `forbidden` et `allowed_bounded` ; `model_calls=forbidden` n'est pas un autre champ. `automation_mode` ne prend que les valeurs `replay` et `supervised`, seules publiables dans `automation_version_modes` ; `learn` et `repair` désignent les jobs de conception lancés par `LearnAutomation`/`RepairAutomation` (voir [Modes publiés et jobs de conception](#modes-publiés-et-jobs-de-conception)), jamais une valeur d'`automation_mode` acceptée par `CreateRun`. Cet axe est distinct du `execution_mode` navigateur (`mediated` ou `project_script`). Un `CreateRun` de source `automation_version` en `replay` refuse `allowed_bounded` ; `supervised` requiert le profil et les bindings de supervision publiés. Changer de mode n'est pas un hot swap de run.

Les travaux de conception référencent le run source par provenance, sans constituer une continuation permettant de contourner sa politique. Un driver ou worker de replay ne peut effacer sa filiation d'exécution pour demander `RepairAutomation`. Une commande utilisateur, ou une politique préautorisée versionnée appliquée par le contrôle déterministe, peut ouvrir un job distinct avec périmètre, budgets et gates de publication propres. Sans ce mandat, le run reste bloqué. La référence causale à l'incident ne supprime pas la distinction des mandats ; le rapport de mission montre et additionne les coûts de supervision et de réparation au lieu de présenter tout le cycle comme exempt de modèle.

Dans le mode supervisé, le script ou les actions médiées exécutent les gestes. À un checkpoint publié ou une exception attendue, le worker persiste les résultats et atteint une frontière sûre ; une tâche `leader_decision` du workflow existant reçoit des logs structurés, assertions, snapshots et limites. Ses sorties typées sont `continue`, `reconcile`, `correct` ou `escalate`, avec références de preuves et justification partageable. Le harness valide les préconditions et routes autorisées ; la décision du LLM ne lève pas une autorisation absente, un effet inconnu ou une assertion métier obligatoire en échec.

Les fréquences sont explicites : checkpoints nommés ou lots de lignes, déclenchement `on_exception` déclaré, plafond d'invocations de supervision, appels par invocation, tokens, coût et durée. Aucun réveil à chaque ligne de log ou à chaque clic n'est ajouté. Les exceptions métier anticipées peuvent être des outputs d'observation typés (`safe_to_continue=false`, raison et état de l'effet) routés par `choice` vers un `task` déclaré ; cela n'est pas une réussite métier. Les erreurs non traitées gardent les états F005/F018. `correct` sélectionne une branche corrective déjà publiée ou demande une réparation distincte dans son mandat ; il ne modifie pas le script en cours.

Une trace de navigation n'est qu'un ensemble d'observations. Elle peut contenir une erreur, un chemin accidentel, un compte différent ou un succès apparent. Le générateur produit un candidat ; la validation structurelle, les contrôles métier et l'évaluation sur entrées représentatives restent des étapes distinctes. Une contradiction entre trace, données et résultat attendu est conservée et empêche la publication tant que le contrat requis n'est pas satisfait. Le LLM ne transforme pas son propre résumé en preuve.

## Contenu d'une version publiée

Les noms sont des champs de contrat proposés, à matérialiser dans un schéma dédié avant implémentation. Ils ne décrivent pas une API de bibliothèque externe.

| Champ ou référence | Règle |
|---|---|
| Identité et format | `artifact_id`, `version`, `schema_profile`, digest, nom et description. Un statut de dépréciation/révocation est un événement de registre séparé des octets immuables. |
| Procédure compilée | Référence exacte et digest d’un workflow de profil exact `unleash.workflow.v1.1` ou `unleash.workflow.v1.2` par mode supporté (v1.2 pour for_each) : contrôles et actions fermées ; en mode supervisé seulement, tâches de décision et bindings exacts déclarés. |
| Source de conception | Draft exact, auteur authentifié, discovery receipt, traces/observations, source de génération éventuelle et diff examinable. Une provenance absente reste absente. |
| Schémas de paramètres | Schémas d'entrée/sortie, paramètres requis, valeurs par défaut explicites, types, unités, clés métier et règles de mapping. |
| Contrat observable | Profil exact de préconditions, postconditions, done, validateurs et politique d'inconnu. Les validations ne contiennent aucun prompt libre à exécuter. |
| Catalogue et compilation | Versions/digests exacts du compilateur, du reducer, de l'évaluateur et des opérations/validateurs utilisés. |
| Drivers et dépendances | Identité/version/digest du driver, protocole, moteur externe, runtime, paquet ou lockfile applicable ; capacités et plateforme qualifiées. |
| Profil d'exécution | `mediated` ou `project_script` lorsqu'applicable, limites d'observabilité et mécanismes requis de confinement/egress. Un script externe n'est pas réputé équivalent aux actions typées. |
| Autorité maximale | Capacités demandées, ressources, domaines, chemins, opérations et périmètre de mandat ; jamais des grants acquis pendant l'apprentissage. |
| Effets et reprise | Identité logique d'effet, stratégie de réconciliation, méthode d'observation, idempotence documentée éventuelle et frontières de checkpoint. |
| Limites | Durée, activations cumulatives, tentatives, concurrence, processus, contexte navigateur, données et sortie ; en mode supervisé, fréquence et budgets modèle ; la section supervision de l'entrée `supervised` (dite SupervisionPolicy) fait partie des octets immuables de la version. Les limites effectives sont l'intersection des plafonds du run et des autorités supérieures. |
| Qualification | Références d'évaluation du même digest, oracles métier, environnement, traces négatives et lacunes. Les rapports de qualification ne contiennent pas de secrets. |

La publication est atomique : résoudre les références autorisées, vérifier les digests, compiler et valider, vérifier les preuves exigées puis écrire version/événement/receipt ensemble. Un alias tel que « courses habituelles » peut faciliter la sélection ; il est résolu avant création du run et disparaît de son manifest effectif. Publier v2 ne modifie aucun run v1, même en file, en pause ou à la prochaine ligne CSV. Une dépendance épinglée indisponible ou révoquée bloque ; aucune mise à jour implicite ni réinstallation non déclarée.

## CSV et autres données paramétrées

Le CSV est un format d'entrée, pas un programme. Un profil de parsing versionné fixe séparateur, encodage, échappement, en-tête et règles de valeurs nulles. Proposition initiale : UTF-8, BOM optionnel, séparateur virgule ou point-virgule choisi explicitement, fins de ligne LF/CRLF, guillemets doubles et échappement doublé. Aucune détection probabiliste de locale, de colonne ou de type par modèle pendant le run.

Avant tout effet métier, l'import et la validation vérifient l'ensemble de la révision : en-têtes uniques, colonnes requises, types, bornes, unités, identifiants de lignes, doublons métier interdits et nombre maximal d'activations. Proposition de limites d'import : 10 MiB, 10 000 lignes, 64 colonnes et 64 KiB par cellule ; un profil différent exige une version explicite. Les lignes vides et valeurs absentes ont une règle déclarée. Une chaîne commençant par `=`, un fragment HTML ou une instruction de prompt reste une donnée inerte ; les exports tableur utilisent une politique de neutralisation explicite.

Les montants utilisent des unités mineures entières avec devise, ou un type décimal à échelle déclarée ; les identifiants conservant des zéros initiaux sont des chaînes. Le mapping associe des colonnes à des paramètres typés et à des valeurs par défaut déclarées. Il n'exécute ni JavaScript, ni shell, ni formule CSV. Les transformations complexes sont des opérations de catalogue versionnées, bornées et autorisées, ou un artefact externe dont les limites sont déclarées.

Chaque ligne possède une identité stable dans la révision. Une colonne `row_id` unique est préférée ; si le profil autorise des IDs d'import générés, ceux-ci sont matérialisés dans la révision et ne prétendent pas être des clés métier stables entre deux fichiers. Le contrat d'un effet exige sa propre clé métier, par exemple `invoice_id` ou `item_key`. Trier ou éditer le fichier source après import ne change jamais les lignes engagées. Le fichier original et les valeurs normalisées sont tous deux liés au manifest par digest et soumis aux ACL/rétentions applicables.

Exemple synthétique pour une préparation de panier de test, sans commande réelle :

```csv
row_id,item_key,quantity,max_unit_price_minor,currency
r01,SYNTHETIC-APPLE-1KG,2,350,EUR
r02,SYNTHETIC-RICE-1KG,1,280,EUR
```

La valeur attendue porte sur l'article, l'unité, la quantité, la devise et le plafond déclaré ; elle ne se réduit pas à « deux boutons ont été cliqués ». Modifier une quantité crée `AutomationDataRevision` suivante sans réapprentissage si le schéma et le contrat de procédure restent applicables. Une reprise ne remplace pas les données en cours. Toute exécution nouvelle qui pourrait répéter un effet antérieur reste soumise à la barrière de réconciliation de cet effet, même avec une nouvelle version de données ou d'artefact.

## Un seul moteur de contrôle

La compilation cible un profil épinglé [v1.1](automation-execution-profile.md) ou [v1.2](workflow-data-iteration.md), requis pour for_each ; les définitions précédentes restent inchangées. `sequence`, `parallel`, `choice`, `repeat`, `fail` et, en v1.2, `for_each` gardent leurs règles. Une feuille `action` référence opération/version exactes, entrées typées/littérales/référencées, profil de contrat exact, timeout et tentatives bornées ; elle n'invoque pas un rôle ou un prompt. Le catalogue d'opérations est fermé. Une extension exige un adaptateur Rust et une qualification explicite, pas un mot-clé libre dans le Markdown.

Le profil permet des bindings vides et des plafonds LLM/tokens/exécutions agentes à zéro pour le replay. Le mode supervisé compose les mêmes blocs avec des tâches agentes explicites ; il n'ajoute ni moteur d'orchestration LLM hors graphe ni obligation universelle d'avoir un juge. `max_concurrent_action_executions` borne les workers d'action simultanés ; `max_step_activations` borne le travail cumulé, notamment après expansion des données, boucles et tentatives. Cent actions séquentielles ne nécessitent pas cent places concurrentes. Les bornes du DSL sont vérifiées avant publication et création du run ; un lot trop grand est refusé ou partitionné par une nouvelle commande explicite, jamais tronqué silencieusement.

Un artefact de script utilisateur peut être une feuille externe `project_script` avec timeout, ressources et contrat, sans être prétendu compilable intégralement dans l'AST. Son contrôle interne n'est pas un deuxième orchestrateur du harness : il ne peut créer des agents, modifier les grants, gérer son propre ledger autoritaire ou relancer un DSL Unleash. Ses effets opaques sont classés comme tels. Le [contrat navigateur](browser-execution.md) précise la médiation ; F011 supervise le processus éventuel. Une scriptothèque de projet JavaScript/Playwright est compatible avec un cœur Rust selon la [politique d'implémentation](../brief/implementation-policy.md), sous réserve du profil réellement qualifié.

## Interdiction transitive des appels modèle

`forbidden` est une politique d'autorité, pas un simple budget financier nul ni un flag d'interface. Le daemon attribue chaque action, worker, processus, notification et rapport à son origine. Le broker et le routeur modèle vérifient cette politique avant tout dispatch. Un descendant ne peut effacer son origine ni demander une invocation neuve pour acquérir le droit modèle. Le profil interdit appels texte/vision, modèles locaux, embeddings, reclassements, compactions et résumés LLM, y compris via un outil, un protocole de sampling ou un proxy. Une recherche déterministe sur des données déjà disponibles reste possible sans calcul d'embedding.

Sous `forbidden`, les notifications N+1/N+2 mettent à jour leurs projections durables sans les réveiller pour « juger » ou rédiger le rapport. La clôture est calculée par les contrats et observations ; la mise en forme emploie des templates versionnés. Un choix inconnu bloque ou sollicite une décision humaine déclarée. Il ne devient jamais une consultation LLM invisible. Sous `allowed_bounded`, seuls les points de supervision publiés peuvent matérialiser leurs tâches agentes ; leurs invocations et coûts apparaissent dans les compteurs, sans prétendre respecter la garantie zéro modèle.

Le profil d'exécution supprime les capacités et secrets modèle des drivers/scripts, interdit l'accès au canal administratif, contrôle egress et descendants selon le backend qualifié et instrumente les routes de modèle. Bloquer quelques noms de fournisseurs ne suffit pas à certifier un script arbitraire. Si les garanties requises ne peuvent être appliquées, l'admission retourne `IsolationUnavailable` ; le système n'affiche pas un replay garanti sans LLM. Un script peut rester exécutable comme outil de projet sous un autre contrat clairement identifié, mais cette exécution n'est pas la réussite du présent contrat.

La preuve porte sur les appels attribuables à Unleash et à ses descendants contrôlés. Elle n'affirme pas que le site tiers n'utilise aucun modèle dans son implémentation interne. Un contrôleur déterministe n'implique pas non plus des données distantes constantes : disponibilité, prix, authentification et état métier sont réobservés.

## Admission, budgets et ressources

**Décision R7 (Q-15, [ADR-13](adr/ADR-13-scheduler-budget.md)) :** un plafond global utilisateur d'unités actives de 5 par défaut, configurable jusqu'à 50, couvre une invocation agente active, un worker de replay actif ou un `provider_probe` actif (voir [scheduling](../specs/scheduling.md)). Un replay utilise zéro invocation agente et zéro appel modèle, mais une unité d'exécution globale. Les agents N/N+1/N+2 n'ont aucune exemption. Les microsteps purs du reducer ne créent pas un worker ou un slot modèle.

Le scheduler, la file durable et le ledger sont uniques. Les sous-compteurs séparent agents, appels modèle, workers de replay, processus auxiliaires et contextes navigateur ; ils ne créent pas deux plafonds additionnables de 50. Les plafonds de concurrence d'un run sont des sous-limites du plafond global. Les réservations du worker, des ressources requises, du budget de coût non modèle et des scopes sont engagées atomiquement avant dispatch.

Le `browser_context_limit` est un budget distinct de ressources, de défaut 2 ([ADR-16](adr/ADR-16-reusable-automation-and-supervision.md), amendement R7), à mesurer en recette et contraint par configuration, capacité globale et mémoire qualifiée. Un worker navigateur actif réserve unité globale et contexte ; un contexte conservé pendant une attente humaine sûre peut garder ses réservations mémoire/processus après libération de l'unité active. La présence du contexte ne permet aucune continuation. Un processus auxiliaire ou navigateur encore capable d'effet conserve son inventaire, son owner et son fence. Les baisses de limites et la reprise suivent le drainage/réconciliation existants ; aucun compteur n'est remis à zéro par changement de données ou de worker.

À une frontière de supervision, le worker rend son unité seulement après checkpoint sûr ; la tâche de décision acquiert ensuite une unité agente du même pool. Le navigateur conservé reste compté séparément. Des blocs parallèles et un superviseur réellement actifs occupent chacun leurs unités ; le parent ne retient pas une unité inutile en attendant son enfant.

Un `replay_worker` est admis par activation `kind=action` (`replay_workers.step_id`) ; « plusieurs actions séquentielles » désigne les actions journalisées d'une même activation, par exemple un `project_script`. Son unité est rendue à la clôture de l'activation ou après checkpoint sûr. Un contexte navigateur conservé entre activations est une réservation `capacity_reservations` du pool navigateur dont le propriétaire est le Run (`owner_type=run`, `owner_id=run_id`), liée à sa génération et à la lease du périmètre métier. Chaque activation suivante l'utilise sous un nouveau fence, réobserve page et identité et ne réutilise aucun handle éphémère.

## Effets, checkpoints et preuves métier

Avant un effet, persister identité d'action, ligne/activation, paramètres, autorité actuelle, préconditions observées, stratégie de reprise et identité logique d'effet. La clé de déduplication locale du dispatch reste stable entre tentatives du même effet. La clé métier servant à comparer deux runs/versionnements est définie par le contrat : la paire `run_id/row_id` seule n'empêche pas de refaire une opération après réparation.

Le driver réobserve l'identité de la cible et les préconditions juste avant engagement. Les locators sémantiques et identifiants stables sont privilégiés ; les alternatives doivent être publiées et bornées. Zéro correspondance, plusieurs correspondances ou identité métier différente donnent un blocage. Aucun choix implicite du premier résultat, index opportuniste ou guérison de sélecteur par modèle. Le contrat navigateur précise les limites des actions atomiques et des scripts opaques.

Une postcondition observe le résultat métier indépendamment du simple retour d'appel : facture et total corrects, données exportées attendues, article/unité/quantité exacts ou assertion E2E portant sur l'état serveur de fixture. Un effet connu mais incorrect conduit à échec métier ; un effet ou une preuve inconnu bloque et n'autorise pas sa répétition. Une compensation n'est ni un rollback magique ni une action implicite : elle exige son propre contrat et mandat.

Le checkpoint conserve curseur d'AST et de ligne, entrées épinglées, effets engagés et réconciliés, preuves, budgets, leases et état de rendez-vous humain. Il ne contient pas une promesse de restaurer RAM du navigateur, handles DOM, socket, page ou session serveur. La reprise recrée un contexte autorisé si nécessaire, réauthentifie par le canal prévu, réobserve la page et reprend depuis une frontière durable seulement après réconciliation. Un cookie ou état d'authentification éventuel est une référence protégée hors bundle publié, jamais un secret dans le CSV.

Les cutpoints C3–C9 et C12 de [failure-recovery.md](failure-recovery.md) s'appliquent sans exception. `outcome_unknown` interdit un retry aveugle. Les méthodes read-only de réconciliation restent déterministes. Sous `forbidden`, une décision non automatisable est humaine ; sous `supervised`, le superviseur peut proposer une suite autorisée depuis les preuves sans transformer un résultat inconnu en succès. Une nouvelle version ne contourne pas l'incident irrésolu du run précédent.

## Cycle de vie et limites de la preuve

Déprécier retire la version des sélections automatiques futures tout en conservant les runs épinglés applicables. Révoquer pour sécurité bloque les actions concernées selon le broker sans réécrire l'histoire. Purger suit la rétention : objets manquants et preuves tombstonées rendent une reprise ou une reconstitution incomplète explicitement visible, jamais remplacée par `latest`.

Les garanties et oracles sont détaillés dans [F019](../features/F019-reusable-automations/README.md) et [automation-replay-tests.md](../testing/automation-replay-tests.md). Le pilote navigateur de référence V1 est fixé par la décision Q-18 ([exécution navigateur](browser-execution.md)). Les recommandations de dépendances navigateur s'appuient sur les [sources primaires](../research/browser-automation-sources.md) ; leurs capacités externes ne prouvent ni la qualité d'un candidat généré, ni l'absence d'appels modèle dans une implémentation future d'Unleash.

## Modes publiés et jobs de conception

Une version d’AutomationArtifact peut publier replay, supervised ou les deux. Chaque entrée porte sa référence exacte de workflow/policy/driver ; CreateRun sélectionne une entrée existante et épingle cette résolution. Choisir un mode absent est un refus, pas une compilation implicite. La projection SQL correspondante est automation_version_modes ; le Run possède une FK de mode et aucune substitution en vol.

learn et repair décrivent les jobs de conception lancés par leurs commandes sous mandat distinct. Ils produisent un draft ; ils ne sont pas des modes d’exécution d’une version d’automatisation publiée. Ils réutilisent les tâches/invocations et, lorsqu’un plan de conception est utilisé, un Run de source workflow_version avec leurs références de draft/provenance. Les évaluations utilisent des snapshots candidats immuables sous autorité de qualification, jamais l’autorisation générale de lancer un draft mutable. L’interface distingue conception, évaluation et exécution publiée.

### Snapshot de qualification

Une évaluation F019 utilise le Run commun de source workflow_version avec purpose=evaluation, candidat/digest et cible autorisée. Le snapshot immuable est marqué qualification_only dans les métadonnées de version ; il n’est pas une version publiée sélectionnable en exécution opérationnelle. Le broker vérifie cette visibilité et l’autorité de qualification avant CreateRun/StartRun. Les colonnes purpose/candidate_ref du Run et visibility du snapshot relient les preuves sans modifier les octets exécutés. Le contrat détaillé est dans [F019](../features/F019-reusable-automations/technical-design.md).
