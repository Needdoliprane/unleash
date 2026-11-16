# Audit contradictoire — automatisation navigateur R3

**Audit documentaire du 16 novembre 2026.** Périmètre : apprentissage d’un parcours navigateur, procédure paramétrée et rejeu sans modèle dans un harness généraliste Rust. Aucun runtime Unleash n’a été construit ou testé ; aucun compte ni page Carrefour n’a été automatisé. Les capacités externes citées sont documentées par leurs auteurs ; les garanties Unleash restent à implémenter et à démontrer.

Références : [sources primaires](../research/browser-automation-sources.md), [spécification](../specs/browser-automation.md), [architecture](../architecture/browser-execution.md), [profil workflow commun](../architecture/automation-execution-profile.md), [reprise](../architecture/failure-recovery.md).

> **Note R7 (16 novembre 2026) :** le refus canonique d’un appel modèle sous `forbidden` (BA-01) est `CapabilityDenied/MODEL_CALLS_FORBIDDEN`, selon la table « États et erreurs » de [F019](../features/F019-reusable-automations/technical-design.md) ; les autres noms de domaine s’y rapportent. Le pilote de référence du profil `mediated` est tranché par Q-18 (WebDriver W3C, `fantoccini`, Chrome for Testing ; [exécution navigateur](../architecture/browser-execution.md)).

## 1. Verdict

**L’objectif est techniquement défendable, à condition de compiler une procédure explicite et de conserver les limites de validité.** Un LLM peut aider à découvrir le parcours et les sélecteurs ; la lecture d’un CSV, le contrôle d’un prix, les attentes et l’exécution d’une suite d’actions ne nécessitent pas intrinsèquement un modèle. Le dossier s’appuie sur les capacités programmables documentées de Playwright et sur le protocole WebDriver, sans prétendre avoir prouvé le cas marchand réel. [B01, B05, B15]

La recommandation de référence est le profil **`mediated`**, piloté par les contrôles Rust et les opérations du catalogue commun. Les scripts Playwright/Selenium restent des artefacts de projets utilisateur possibles. Leur exécution générale ne doit pas être présentée comme équivalente, en termes d’autorisation et d’observation des effets, à une procédure fermée.

L’intégration conserve un seul modèle AutomationArtifact/Run et un seul reducer. Elle ne rend pas chaque tâche quotidienne dépendante d’un agent. Le principal risque n’est pas le langage du moteur réutilisé ; c’est une promesse trop forte fondée sur un locator, une trace, un sandboxing partiel ou une sortie de script.

Le mode **supervisé** répond aussi à la demande : blocs mécaniques et tâches bornées du leader aux checkpoints, sous `allowed_bounded`. Il peut aider à interpréter les écarts et à choisir une branche prévue, sans appel LLM à chaque clic. Ce mode reste nommé et mesuré séparément du replay `forbidden`. L’avis du leader ne prouve pas à lui seul la correction du script et ne peut effacer une assertion négative ou un résultat d’effet inconnu.

## 2. Constats, décisions et preuves manquantes

| ID / gravité | Constat | Résolution de conception | Preuve produit encore requise |
| --- | --- | --- | --- |
| BA-01 Bloquant | Un budget modèle nul déclaré ne bloque pas nécessairement un manager, un hook, une réparation ou un appel réseau direct. | `model_policy=forbidden` transitive, opérations machine-only, Model Router bloquant, pas de sampling ni credentials ; confinement des descendants. | Appels canaris directs et indirects tous refusés ; zéro invocation agente et rapprochement du trafic. |
| BA-02 Bloquant | Des références de snapshot enregistrées ne désignent pas durablement les mêmes éléments. [B10] | Publication de locators réévaluables ; refs transitoires uniquement pour la trace courante. | Navigation/re-render et changement de session : cible correcte ou refus, sans réemploi d’un ancien ref. |
| BA-03 Bloquant | Une allowlist de domaine laisse possibles des actions très différentes sur le même site. | Autorité par opération et effet, préconditions sur produit/montant/destinataire, gate pour engagement sensible. | Script ou procédure tentant une commande à la place d’un ajout au panier : aucun dispatch non autorisé. |
| BA-04 Bloquant | Après clic et perte d’accusé, l’état distant peut déjà avoir changé. | Journal commun et `outcome_unknown`, réconciliation avant retry ; quantité cible seulement si comportement qualifié. | Coupure après dispatch avant observation : absence de doublon ou blocage explicite. |
| BA-05 Élevé | Actionabilité et assertions techniques ne remplacent pas le résultat métier. [B03–B04] | Identité produit, conditionnement, quantité, devise et budget dans le contrat. | Mauvais produit au même nom, valeur différente, prix ou stock changé : arrêt/branche prévue. |
| BA-06 Élevé | Script libre et confinement OS ne certifient pas automatiquement ses intentions métier. | Deux modes explicites ; refus de la garantie renforcée si la médiation requise n’est pas disponible. | `project_script` hostile et dépendance altérée : confinement réel ou `IsolationUnavailable`. |
| BA-07 Élevé | Cookies et fichiers d’état peuvent exposer le compte. [B08] | Références privées, export excluant l’état authentifié, ACL et rétention ; revue des traces. | Canaris secrets, lecture inter-projets, sauvegarde/export et crash. |
| BA-08 Élevé | Un blocage peut devenir une boucle d’auto-réparation LLM opaque. | Réparation dans une opération distincte, mandat actuel ou politique préautorisée, budget séparé, diff et gates. | Ancien run inchangé, zéro modèle conservé, nouvelle version et adoption explicites. |
| BA-09 Élevé | « Rust » peut être interprété comme obligation de réécrire Playwright ou comme permission d’un backend JS caché. | Port/autorité/compilation Rust, moteur externe upstream inventorié ; scripts visibles comme artefacts de projet. | Inventaire de composants et absence d’orchestrateur/permissions déplacés dans un service JS interne. |
| BA-10 Élevé | Une lease locale ne protège pas des modifications du même compte par d’autres appareils. | Préconditions réobservées, conflits conservés et réparation manuelle/automatisée selon contrat. | Modifications concurrentes entre deux étapes ; aucun écrasement silencieux. |
| BA-11 Élevé | Les outils et binaires peuvent changer de version indépendamment. [B14] | Runtime lock, digests, mapping qualifié et refus de mismatch ; pas de résolution `latest` au replay. | Moteur/driver incompatible, catalogue changé et dépendance altérée : admission refusée. |
| BA-12 Moyen | Un navigateur/headless peut devenir une obligation de GUI ou un pool supplémentaire illimité. | Parité des clients, handoff navigateur système depuis TUI, pool global partagé et contexte réservé. | Même résultat GUI/TUI/CLI ; attentes qui libèrent correctement le slot mais conservent la mémoire réservée. |
| BA-13 Bloquant | Un leader peut transformer des logs rassurants en autorisation de poursuivre malgré un effet inconnu. | ObservationReport typé, choice explicite vers leader_decision, avis soumis au broker ; aucun hotpatch ou catch implicite. | Avis volontairement erroné : refus du retry/continuation conflictuelle et impossibilité de terminer avec une inconnue bloquante. |

La gravité vise la promesse produit si le contrôle manque. Elle ne signale pas une vulnérabilité constatée dans une implémentation existante.

## 3. Comparaison des options et objections

### Moteur Playwright externe

**Pour :** outils d’observation et d’automatisation documentés, génération de code et écosystème de traces. Un client Rust peut porter le contrat du harness tandis que le moteur upstream est inventorié comme dépendance.

**Objection :** l’intégration oblige à choisir le transport, qualifier les outils, le runtime, le moteur navigateur et les contrôles d’accès. La liste des langages supportés ne permet pas d’inventer un SDK Rust officiel. Le README de Playwright MCP ne constitue pas une API stable à lui seul. [B01, B09]

**Décision proposée :** admissible, avec package/version/schema d’outils épinglés. MCP est un choix de transport et d’exécutant, pas la raison d’utiliser un modèle pendant le replay. La négociation de capacités n’est pas une autorisation automatique de nouveaux outils.

### WebDriver piloté depuis Rust

**Pour :** protocole indépendant du langage, clients Rust existants ; cohérence avec le noyau système sans reconstruire le navigateur. [B15, B18–B19]

**Objection :** les wrappers, attentes, traces et locators ne sont pas interchangeables par simple changement de package. Le statut de WebDriver 2 consulté est Working Draft ; il ne doit pas être vendu comme une Recommendation finalisée. [B16–B17]

**Décision proposée :** alternative qualifiable ou backend ultérieur. La présence d’une crate ne prouve pas que toutes les exigences de BrowserPort sont couvertes. La première version du produit peut qualifier un seul backend.

### Script Playwright/Selenium exécuté tel quel

**Pour :** résultat inspectable, portable, modifiable et familiarité pour un développeur ; les non-développeurs peuvent consommer la procédure publiée sans ouvrir le code.

**Objection :** un script libre conserve des possibilités que le noyau ne voit pas nécessairement à chaque étape. Un environnement réseau réduit est utile, mais il ne distingue pas toutes les mutations commerciales d’un site autorisé. La conformité structurelle de sa sortie ne prouve pas son comportement.

**Décision proposée :** conserver l’option comme projet utilisateur sous F011 ; attester uniquement les garanties effectivement démontrées. Le mode médié reste préférable lorsqu’une limite par action sensible est requise.

## 4. Ce que « valide » doit réellement signifier

Une version peut rester syntaxiquement valide tout en étant inapplicable à un compte, un magasin, un CSV ou une version de navigateur. Le dossier distingue donc : **forme valide**, **autorité valide**, **runtime compatible**, **données acceptées**, **préconditions actuelles** et **résultat observé**. Aucune de ces propriétés n’implique automatiquement les autres.

Les préconditions doivent notamment reconnaître une redirection vers l’authentification, le mauvais compte, un catalogue régional différent ou une page de consentement. Les assertions ne doivent pas se limiter à la présence d’un texte générique « ajouté ». Le rapport précise ce qui a été rapproché : référence produit, quantité, prix, total et état du panier selon le mandat.

Le succès d’un enregistrement unique ne suffit pas à certifier un parcours réutilisable. La qualification doit au minimum varier données et états représentatifs, déclencher les branches prévues et provoquer une dérive. Elle ne prouvera toujours pas que le site ne changera jamais : la valeur du contrat tient à l’arrêt explicite et à la réparation contrôlée quand ses hypothèses cessent d’être vérifiées.

## 5. Contre-exemples à conserver dans le plan de vérification

| Cas adverse | Faux résultat à éviter | Résultat cible |
| --- | --- | --- |
| Deux boutons « Ajouter » pour deux conditionnements | Le premier élément est choisi | Identité/portée/cardinalité contrôlées ou ambiguïté. |
| Panier contient déjà deux unités et le CSV demande deux unités | Ajout de deux unités supplémentaires à chaque reprise | État cible déclaré, rapprochement avant mutation. |
| Prix change après approbation | Ancienne approbation réutilisée malgré le nouveau montant | Préconditions/autorité réévaluées sur le payload actuel. |
| Timeout après clic, quantité ensuite correcte | Nouveau clic « pour être sûr » | Observation et attribution ; aucune répétition aveugle. |
| Trace présente mais assertion finale absente | Run marqué réussi | Preuve manquante ou `unknown` ; trace et verdict distincts. |
| Instruction hostile dans une fiche produit | Lancement de script ou ouverture d’un modèle | Donnée non fiable, aucun effet hors opérations publiées. |
| Nouveau schéma MCP ajoute une capacité de code libre | Outil accepté automatiquement | Mapping non qualifié et admission refusée ou capacité ignorée. |
| Script appelle une API modèle avec une clé copiée depuis le profil | « Zéro LLM » d’après le seul compteur du Model Router | Accès secret et réseau effectivement refusé ; sinon qualification refusée. |
| Page change uniquement une publicité | Réparation LLM systématique | Continuer si les préconditions et cibles publiées restent satisfaites. |
| Page garde les mêmes sélecteurs mais change le sens d’une action | Aucun contrôle parce que le DOM paraît identique | Invariants métier et gate sensible ; limite résiduelle explicitée. |
| Intervention humaine modifie le panier pendant la pause | Reprise à l’ancien index sans contrôle | Nouvelle observation et réconciliation. |
| Moteur indisponible ou version incompatible | Installation automatique de la dernière version | Remédiation explicite et runtime qualifié avant admission. |
| Logs propres, assertion métier fausse | Le leader valide « tout va bien » | Verdict négatif conservé et prochaine action refusée si elle en dépend. |
| Script meurt avant son rapport de checkpoint | Exception capturée par une branche qui n’existe pas | État failed/blocked commun, sans continuation improvisée. |
| Checkpoint supervisé prévu tous les cinq articles | Un LLM est appelé à chaque geste du navigateur | Fréquence conforme au workflow publié et budget borné ; zéro appel en mode forbidden. |

Ces cas sont à réaliser sur un environnement de test contrôlé du futur produit. La révision documentaire ne lance aucun de ces tests et ne s’en attribue aucun résultat.

## 6. Confirmé, proposé, à confirmer

| Statut | Éléments |
| --- | --- |
| Confirmé par la demande | Harness polyvalent ; possibilité d’apprendre un parcours avec LLM ; procédure/script paramétré par données ; rejeu sans modèle tant que valide ; supervision optionnelle de blocs/logs sans modèle à chaque clic ; analyse/réparation explicite ; noyau Rust avec réutilisation raisonnable de moteurs. |
| Confirmé par sources | Langages Playwright documentés, capacités de locators/attentes/codegen/traces/auth ; architecture et transport MCP ; nature transitoire des refs ; protocole WebDriver et clients Rust mentionnés. |
| Proposé ici | Mode médié de référence, port Rust, qualification par capacité, modèle de validité, préconditions métier, contrats de reprise et articulation F019/Run. |
| À confirmer par mise en œuvre | Versions exactes, couverture du backend choisi, confinement macOS/Linux, comportement de session, coût/ressources et résultats des scénarios d’acceptation. |
| Non évalué | Site réel Carrefour, comptes, catalogue, selectors, conditions applicables et parcours réel de panier/commande. |

L’écart de prérequis Node constaté entre deux pages officielles est consigné dans [la recherche](../research/browser-automation-sources.md). Il illustre le besoin d’un verrouillage de release, pas une raison d’inventer une version compatible. La qualification future doit s’appuyer sur les packages retenus, leurs exigences et leurs comportements observés.

## 7. Portée de la vérification documentaire

Cette révision vérifie la cohérence des quatre documents navigateur avec la politique de langage R2, le catalogue commun, le profil d’exécution R3 et les contrats de reprise. Les noms internes sont marqués comme propositions ; les faits externes sont reliés à des URLs primaires ; l’exemple de données est explicitement fictif. Les liens et la présence des fichiers sont contrôlables sans exécuter un navigateur.

Les preuves produit manquantes ne sont pas transformées en tâches déjà réussies. Avant toute annonce opérationnelle, les gates B-AC de [la spécification](../specs/browser-automation.md) devront être exécutés et leurs limites publiées. Une validation documentaire favorable ne vaut ni test Playwright, ni qualification de sécurité, ni démonstration d’un achat automatisé.
