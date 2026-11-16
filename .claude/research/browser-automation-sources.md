# Automatisation navigateur — sources et conclusions R3

**Recherche documentaire vérifiée le 16 novembre 2026.** Sources primaires uniquement : documentation des projets, spécifications et dépôts de leurs mainteneurs. Aucune navigation sur Carrefour, aucune utilisation de compte, aucun parcours de courses et aucun test du produit Unleash n’ont été effectués. Les choix d’architecture sont arrêtés par le dossier (Q-18 tranchée en R7, [exécution navigateur](../architecture/browser-execution.md)) ; les capacités documentées des outils ne prouvent pas leur intégration dans Unleash.

## 1. Conclusions exploitables

| Question | Fait confirmé ou conclusion bornée | Conséquence proposée pour Unleash |
| --- | --- | --- |
| Playwright exige-t-il un LLM ? | La bibliothèque, le générateur d’actions et le runner documentés s’utilisent par programme. La génération par enregistrement ne requiert pas de modèle. [B01, B05] | Apprentissage assisté facultatif ; exécution ultérieure de la procédure publiée sans modèle. |
| Existe-t-il un SDK Playwright Rust officiel ? | La liste officielle expose JavaScript/TypeScript, Python, Java et .NET ; Rust n’y figure pas. Cela ne démontre pas l’absence de tout binding communautaire. [B01] | Ne pas inventer d’API Rust Playwright. Réutiliser un moteur externe documenté ou choisir un client WebDriver Rust évalué. |
| Un locator peut-il résister aux changements du DOM ? | Playwright propose notamment rôle/nom accessible, libellé et test-id ; il recherche l’élément au moment de l’action. [B02] | Préférer des cibles sémantiques observées et une unicité vérifiée. Aucun locator n’est garanti stable sur un site tiers. |
| L’attente automatique garantit-elle le résultat métier ? | L’action attend des conditions d’interactivité ; les assertions web peuvent attendre une condition observable. [B03, B04] | Déclarer séparément préconditions, action et postconditions. Un clic abouti ne prouve pas l’ajout du bon produit. |
| Une trace est-elle un reçu de commande externe ? | Le visualiseur permet d’examiner les opérations enregistrées. L’API `context.tracing` n’enregistre pas les assertions de test ; le mode Playwright Test les inclut. [B06, B07] | Conserver les verdicts d’assertions et les observations métier à côté de la trace. Un ZIP de trace ne prouve pas un paiement ni une absence d’effet. |
| L’état authentifié peut-il être réutilisé ? | Playwright documente la réutilisation d’état et prévient que son fichier peut permettre l’usurpation du compte. La persistance de `sessionStorage` n’est pas couverte par une API dédiée. [B08] | Référence opaque vers état privé ; jamais cookies ou jetons dans le bundle versionné. Réauthentification explicite si nécessaire. |
| MCP impose-t-il une boucle LLM ? | MCP sépare hôte, client et serveur ; les messages sont encodés en JSON-RPC 2.0 et transportés par `stdio` ou Streamable HTTP [B12]. Le sampling est une capacité déclarée distincte. [B11–B13] | **Inférence de conception :** un client déterministe peut appeler un serveur d’outils sans déclencher de modèle. Cette propriété doit être vérifiée pour le serveur et ses dépendances retenus. |
| Les références de snapshots MCP sont-elles persistantes ? | Les références d’éléments appartiennent à un snapshot et deviennent périmées lorsque la page change. [B10] | Une procédure conserve un locator réévaluable ; jamais la suite de références `e12`, `e35` issue de l’apprentissage. |
| L’option d’origines MCP est-elle une sandbox ? | Le projet Playwright MCP précise que ses filtres d’origines et son serveur ne forment pas une frontière de sécurité ; les filtres cités ne couvrent pas les redirections. [B09] | Confinement réseau/OS séparé, qualifié sur chaque plateforme. Ne pas transformer une option de confort en garantie d’isolement. |
| WebDriver permet-il un client Rust ? | WebDriver définit un protocole indépendant du langage. Les mainteneurs de Fantoccini et Thirtyfour documentent leurs clients Rust. [B15, B18, B19] | Retenu en R7 pour le pilote de référence `mediated` (`fantoccini`, Q-18) ; ces crates ne sont ni un SDK Microsoft Playwright ni des bindings Selenium officiels présumés. |

La séparation **apprentissage → normalisation → validation → publication → rejeu** est une décision Unleash, pas une fonction universelle que fournirait déjà Playwright ou Selenium. L’aptitude d’une procédure à commander correctement un panier reste à démontrer sur le site, le compte, les données et le moteur qualifiés.

La supervision LLM de blocs est également une **décision Unleash distincte du replay sans modèle** ([ADR-16](../architecture/adr/ADR-16-reusable-automation-and-supervision.md)) : une exécution `allowed_bounded` peut insérer des checkpoints de lecture de rapports et de logs, tandis que `forbidden` reste strictement mécanique. Aucune source citée ne permet de conclure qu’un LLM lisant des logs garantit la correction d’un script. Les preuves typées, contrôles d’autorité et résultats d’effets restent nécessaires dans les deux modes.

## 2. Registre des sources primaires

Les numéros Bxx sont locaux à ce dossier. Toutes les pages ont été consultées à la date ci-dessus. Une URL `main`, `latest` ou une documentation courante sert à la recherche ; elle ne devient pas une dépendance flottante d’un run.

| ID | Source officielle ou du mainteneur | Périmètre confirmé et limite |
| --- | --- | --- |
| B01 | [Playwright — Supported languages](https://playwright.dev/docs/languages) | Langages officiellement documentés et différences d’intégration des runners. Aucune conclusion sur la qualité de bindings Rust tiers. |
| B02 | [Playwright — Locators](https://playwright.dev/docs/locators) | Rôles, noms accessibles, labels, test-id, résolution renouvelée. Un test-id sur un site tiers n’est pas un contrat accordé à Unleash. |
| B03 | [Playwright — Auto-waiting](https://playwright.dev/docs/actionability) | Contrôles d’interactivité, timeout, effet de `force`. Pas de garantie transactionnelle sur un service distant. |
| B04 | [Playwright — Assertions](https://playwright.dev/docs/test-assertions) | Assertions web avec répétition bornée jusqu’à satisfaction. Ce mécanisme ne doit pas répéter une mutation métier. |
| B05 | [Playwright — Test generator](https://playwright.dev/docs/codegen) | Enregistrement, sélection de locators, génération d’assertions. Le résultat doit encore être paramétré et validé. |
| B06 | [Playwright — Trace viewer](https://playwright.dev/docs/trace-viewer) | Inspection locale des traces et éléments de diagnostic. Pas un journal transactionnel du marchand. |
| B07 | [Playwright — Tracing API](https://playwright.dev/docs/api/class-tracing) | Différence entre trace de contexte et trace via Playwright Test. Les options dépendent de la version choisie. |
| B08 | [Playwright — Authentication](https://playwright.dev/docs/auth) | État authentifié réutilisable et sa sensibilité ; limites de persistance. Le rejeu ne dispense pas d’un contrôle d’identité du compte. |
| B09 | [Microsoft — Playwright MCP](https://github.com/microsoft/playwright-mcp) | Serveur upstream, configuration, outils et limites explicites de sécurité. README de branche courante, pas contrat binaire immuable. |
| B10 | [Playwright MCP — Snapshots](https://playwright.dev/mcp/snapshots) | Références transitoires, sélection par cible et renouvellement des snapshots. Le schéma exact reste à épingler au runtime retenu. |
| B11 | [MCP — Architecture, révision 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/architecture) | Séparation hôte/client/serveur et négociation de capacités. Révision choisie pour la référence, sans prétendre être la seule utilisable. |
| B12 | [MCP — Transports, révision 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) | `stdio`, Streamable HTTP et mesures requises au transport HTTP. Aucune équivalence entre `localhost` et authentification. |
| B13 | [MCP — Sampling, révision 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/client/sampling) | Déclaration de la capacité de génération par le client. En rejeu Unleash, cette capacité ne doit pas être annoncée ni exécutée. |
| B14 | [Playwright — Browsers](https://playwright.dev/docs/browsers) | Couplage entre version Playwright et binaires navigateur. Un navigateur mis à jour séparément peut modifier la compatibilité. |
| B15 | [W3C — WebDriver, Recommendation du 5 juin 2018](https://www.w3.org/TR/2018/REC-webdriver1-20180605/) | Référence publiée stable pour le protocole WebDriver classique. La conformité des drivers retenus nécessite une qualification. |
| B16 | [W3C — WebDriver 2](https://www.w3.org/TR/webdriver2/) | La version consultée se présente comme Working Draft du 2 juillet 2026. Ce statut ne doit pas devenir « norme finalisée » dans le dossier. |
| B17 | [Selenium — Waiting Strategies](https://www.selenium.dev/documentation/webdriver/waits/) | Attentes explicites et conditions bornées. L’adaptateur Rust doit fournir les comportements requis ; aucune parité implicite avec Playwright. |
| B18 | [Fantoccini — dépôt du mainteneur](https://github.com/jonhoo/fantoccini) et [documentation de crate](https://docs.rs/fantoccini/latest/fantoccini/) | Client Rust WebDriver, sélection et interaction avec le navigateur. Version et couverture à verrouiller, maintenance non évaluée empiriquement ici. |
| B19 | [Thirtyfour — dépôt du mainteneur](https://github.com/stevepryde/thirtyfour) et [documentation](https://stevepryde.github.io/thirtyfour/) | Client Rust WebDriver, attentes et extensions documentées. Les fonctionnalités annoncées sur la branche courante ne valent pas présence dans toute release. |
| B20 | [Selenium — Downloads](https://www.selenium.dev/downloads/) | Source de distributions et bindings du projet. Ne pas attribuer le statut de binding officiel Selenium à une crate Rust tierce. |
| B21 | [Playwright — Getting started with MCP](https://playwright.dev/docs/getting-started-mcp) | Présentation orientée agents et prérequis d’installation. Son orientation LLM ne transforme pas le transport en appel modèle obligatoire. |

**Sources à consulter et dater au gel M0 (Q-18).** Elles n’ont pas été consultées par cette recherche ; aucun fait n’en est déduit ici.

| ID | Source officielle | Point à vérifier au gel |
| --- | --- | --- |
| B22 | [Chrome for Testing](https://developer.chrome.com/blog/chrome-for-testing) et [tableau des versions](https://googlechromelabs.github.io/chrome-for-testing/) | Canal officiel, couple navigateur/chromedriver du même build, plateformes et empreintes. |
| B23 | [ChromeDriver](https://developer.chrome.com/docs/chromedriver) | Capacités WebDriver du driver épinglé et limites connues. |
| B24 | Sections Get Computed Role et Get Computed Label de WebDriver 2 [B16] | Résolution rôle/nom accessible utilisée par les locators sémantiques. |

## 3. Versions : ce qui est fixé et ce qui reste à choisir

**Fixé dans cette recherche :** références MCP datées `2025-11-25`, WebDriver Recommendation `2018-06-05`, statut de la page WebDriver 2 consultée. Ces dates identifient les textes cités, pas une configuration produit déjà installée.

**Choisi R7 (Q-18) :** WebDriver W3C piloté en Rust par `fantoccini`, sur Chrome for Testing et chromedriver du même build, épinglés par version et SHA-256 ; versions figées au gel M0. Leur préparation est une étape explicite sous mandat, par le parcours de configuration existant ou une exécution F011 suivie, sans commande dédiée. Les OS minimaux sont fixés par Q-11 ([ADR-14](../architecture/adr/ADR-14-local-data-oss.md)).

**Non choisi :** release Playwright du profil `project_script`, release `@playwright/mcp`, Node/Python/Java/.NET éventuel. Le dossier ne présente aucun de ces composants comme exécuté ou qualifié.

Une divergence documentaire mérite une vérification au choix du runtime : la page de démarrage MCP consultée indique Node.js 20 ou plus, tandis que le README du dépôt consulté indique 18 ou plus. Il faut retenir les exigences du package et de sa release, ainsi que la politique de support du runtime sélectionné ; recopier l’un de ces chiffres comme minimum Unleash garanti serait prématuré. [B09, B21]

Le manifeste d’exécution devra consigner versions exactes, digests du bundle et des exécutables, lockfile, moteur, driver, révision du navigateur, profil de confinement et version du schéma d’outils. Une mise à jour déplace un candidat de runtime et exige une nouvelle évaluation ; elle ne modifie pas un run engagé. Aucun lancement certifié ne résout `@latest` sur Internet.

## 4. Variantes d’intégration et choix R7

| Variante | Bénéfice | Travail et limite à assumer |
| --- | --- | --- |
| Procédure fermée pilotée par Rust, moteur Playwright MCP upstream local | Réutilisation des primitives Playwright, apprentissage et rejeu possibles avec le même moteur | Client MCP Rust, mapping de capacités, version précise, autorisation avant chaque effet et sandbox distincte. La négociation ne permet pas d’accepter arbitrairement de nouveaux outils. Non retenu comme pilote de référence V1 (Q-18). |
| Script de projet Playwright dans un langage officiellement supporté | Artefact inspectable, portable et rejouable ; écosystème d’assertions et de traces | Exécution de code utilisateur via F011. Les garanties doivent correspondre à l’enveloppe réelle du processus ; un script libre ne reçoit pas automatiquement la qualification des primitives médiées. |
| **Client Rust WebDriver avec driver externe — variante retenue R7 (Q-18)** | Adaptation système Rust et protocole indépendant du langage | Qualification des sélecteurs, attentes, erreurs, traces et compatibilité navigateur/driver. Ni réécriture complète de Selenium, ni compatibilité Playwright supposée. |
| Réimplémenter un moteur navigateur ou un SDK Playwright Rust maison | Aucun besoin établi dans ce dossier | Coût et divergence non justifiés ; option rejetée pour la conception de référence. |

Le choix retenu garde un seul contrat de procédure et plusieurs capacités de moteurs. Il n’impose pas de réaliser tous les backends avant de rendre un premier parcours utile. Le backend accepté pour une version doit satisfaire ses exigences ; un backend manquant produit un refus explicite.

## 5. Limites de l’étude

Carrefour est un **cas d’usage conceptuel fourni par Louis**. La structure actuelle de ses pages, les identifiants de produits, les mécanismes d’authentification, les parcours de livraison/retrait, les conditions d’utilisation et la faisabilité d’une automatisation effective n’ont pas été examinés. Aucun sélecteur Carrefour réel n’est livré dans ce dossier. Il serait inexact de présenter un exemple illustratif comme un script fonctionnel pour ce site.

Le DOM et l’arbre d’accessibilité n’offrent pas, à eux seuls, une preuve de l’intention du marchand ou de la sémantique de ses requêtes. Les critères techniques de rejeu sont donc conditionnels et accompagnés de postconditions métier. Les détails normatifs proposés figurent dans [la spécification navigateur](../specs/browser-automation.md), [l’architecture d’exécution](../architecture/browser-execution.md) et [l’audit](../audits/browser-automation-audit.md).
