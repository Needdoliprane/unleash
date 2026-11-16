# Automatisation navigateur réutilisable — spécification R3

**Statut : contrat proposé R3 complété par la supervision R5 et par la décision R7 du pilote de référence ([Q-18](../brief/open-questions.md)), 16 novembre 2026 ; aucune implémentation ni évaluation sur Carrefour annoncée.** Cette spécification applique F019 au profil `browser` d’`AutomationArtifact`. Elle complète les [sources vérifiées](../research/browser-automation-sources.md) et l’[architecture navigateur](../architecture/browser-execution.md). Les mots « doit » décrivent le comportement cible.

## 1. Résultat attendu et périmètre

Un utilisateur doit pouvoir apprendre ou décrire un parcours navigateur, en obtenir une procédure paramétrée, publier sa version, puis la relancer avec de nouvelles données sans appeler un modèle. Le même mécanisme convient à un développeur qui vérifie son application, à un comptable qui collecte des pièces autorisées ou à un particulier qui prépare un panier. Aucune étape agente, aucun repository Git et aucune connaissance de Playwright ne sont obligatoires pour déclencher une procédure déjà publiée.

Le résultat conservé est un **AutomationArtifact versionné**, avec son contrat de données, son workflow commun, ses cibles navigateur, ses preuves d’évaluation et, lorsque prévu, un script utilisateur dérivé. `BrowserProcedureVersion` désigne le contenu navigateur d’une telle version ; ce nom ne crée ni registre parallèle, ni version indépendante du parent. `BrowserRunManifest` est un sous-manifest du Run commun.

Le workflow emploie un profil commun épinglé : [`unleash.workflow.v1.1`](../architecture/automation-execution-profile.md) pour des blocs sans itération générique, ou [`unleash.workflow.v1.2`](../architecture/workflow-data-iteration.md) dès que la procédure parcourt les lignes d’une AutomationDataRevision (`for_each`, portée `/items`, `after_batch`). Les feuilles `kind=action` réfèrent une opération cataloguée et sa version ; compositions `sequence`, `choice`, `repeat`, `parallel`, `fail` et, en v1.2, `for_each`. Les opérations navigateur sont des capacités d’action, pas une nouvelle syntaxe de workflow. Le DSL v1 existant et ses fixtures restent identifiables séparément.

**Sans modèle** signifie que ce run et tous ses descendants ne déclenchent aucun appel LLM, y compris manager, réparation, embedding, classement sémantique ou résumé. Le reducer mécanique, les branches sur données typées, les assertions et les attentes bornées restent disponibles. Une page visitée peut utiliser ses propres services ; Unleash ne prétend pas connaître leurs traitements internes.

Un **mode supervisé facultatif** répond à un besoin différent : les scripts ou blocs d’actions restent mécaniques, et un leader LLM borné intervient aux checkpoints ou exceptions prévus. Il utilise `model_policy=allowed_bounded`, expose son budget et ne prétend pas être sans modèle. Le navigateur n’a pas besoin d’un appel LLM pour chaque clic. Le champ commun `automation_mode` prend la valeur `replay` ou `supervised` (`learn` et `repair` sont des jobs de conception, jamais des valeurs de ce champ) ; il est indépendant du choix technique `execution_mode=mediated|project_script`. Les limites de contrôle d’un script libre restent les mêmes, conformément au [modèle d’artefact commun](../architecture/automation-artifact-model.md).

## 2. Cycle d’un parcours

| Phase | Entrée et travail autorisé | Sortie et gate |
| --- | --- | --- |
| Apprentissage | Intention, données d’exemple, observations accessibles et mandat de navigation. LLM autorisé seulement si l’opération le prévoit ; un enregistrement manuel est aussi recevable. | Draft, observations sources, hypothèses, actions réellement effectuées, questions restantes et coût modèle éventuel. Rien n’est publié automatiquement. |
| Normalisation | Transformation déterministe du draft : paramètres, identités d’éléments, branches, limites, pré/postconditions et classement des effets. | Représentation typée et diff explicite. Pas de nouveau choix métier caché. Cette phase appartient à `ValidateAutomationDraft`. |
| Validation | Schéma, données, digests, dépendances, scopes, couverture du moteur, bornes, reprise et politique de modèles. Aucun appel réseau navigateur ni LLM implicite. | Diagnostics attachés à l’entrée exacte. Un draft invalide peut être sauvegardé mais pas publié comme exécutable. |
| Évaluation | Scénarios locaux contrôlés ou parcours réel explicitement autorisé par `EvaluateAutomationDraft`. Un parcours réel conserve ses effets même si son objectif est « tester ». | Rapport avec environnement, mode, assertions et effets. « Simulation » ne peut qualifier une action réellement envoyée au site. |
| Publication | Version exacte, résultats requis, diff, autorité et profil de confinement. | Version immuable, digest et reçu. Une nouvelle publication ne réécrit aucun run. |
| Rejeu | Version publiée, AutomationDataRevision valide, mandat, runtime compatible, `model_policy=forbidden`. | Exécution déterministe du plan, résultats par ligne, preuves et erreurs typées. Aucune recherche de solution par modèle. |
| Exécution supervisée | Version publiée avec blocs, checkpoints et branches de supervision déclarés, données validées, `allowed_bounded`. | Actions mécaniques puis décisions bornées du leader selon le workflow ; chaque appel et sa portée sont visibles. |
| Analyse ou réparation | Commande `RepairAutomation` explicite, périmètre et politique modèle déclarés ; analyse seule possible sans publication. | Diagnostic et nouveau draft lié (contenu éventuellement inchangé si l’analyse conclut sans correction) ; réévaluation et nouvelle publication avant toute adoption. |

Un utilisateur ayant déjà autorisé une portée n’est pas sollicité à chaque clic prévu. Une autorisation ne peut toutefois être élargie par du texte de page, par le modèle ou par un nouveau CSV. Une action engageante hors mandat crée une attente d’autorité explicite ou un refus, selon la politique commune.

L’autorisation d’une réparation séparée peut provenir d’une politique déjà approuvée, avec portée, budget et gates de publication. Elle ne nécessite pas systématiquement une validation humaine nouvelle ; le run `forbidden` ne peut toutefois ni s’accorder cette autorité ni appeler le modèle de réparation. Le choix et le coût de cette opération restent distincts de son historique.

## 3. Contrat du profil navigateur

Ces champs sont un contrat de conception à raccorder au schéma commun F019 ; ils ne constituent pas une API externe ou un schéma JSON déjà implémenté.

| Ensemble | Contenu exigé |
| --- | --- |
| Identité et provenance | Parent AutomationArtifact/version, digest, projet, auteur/origine, version précédente, preuves d’apprentissage et version de normalisation. |
| Runtime | Backend et capacités requis, release et digests, moteur navigateur, driver éventuel, plateforme, profil d’isolation et mode `mediated` ou `project_script`. |
| Données | AutomationDataRevision/version/digest, schéma, dialecte CSV éventuel, paramètres, contraintes de taille, règles de doublons, règles de mapping et politique de lignes invalides. |
| Contexte | Origines autorisées, identité du compte attendue par référence, magasin/périmètre métier éventuel, langue/devise, état initial acceptable et type de session. |
| Cibles | Locators déclaratifs réévaluables, portée de frame/page, cardinalité, propriété métier attendue et provenance de chaque cible. |
| Actions | Opération/version du catalogue, entrées typées, effet demandé, timeout, nombre d’essais et stratégie de réconciliation. |
| Contrat métier | Préconditions, postconditions, invariants, bornes de quantité/montant, traitement des exceptions et résultat terminal attendu. |
| Exploitation | Checkpoints, conditions de reprise, expiration, critères de dérive, limites de parallélisme, références de traces et politique de conservation. |

Les cookies, mots de passe et tokens ne font pas partie de ces octets publiés. Le manifeste conserve uniquement des références de session/secret autorisées et leur état de résolution. Un navigateur isolé constitue un compartiment de session ; ce n’est pas une preuve de sandbox OS. [Sources B08–B09](../research/browser-automation-sources.md)

## 4. Exemple conceptuel : préparer des courses Carrefour depuis un CSV

**But illustratif :** obtenir un panier correspondant aux lignes acceptées et un rapport d’écarts ; l’exemple s’arrête avant la commande ou le paiement. Les identifiants ci-dessous sont fictifs. Aucun élément n’a été observé sur le site réel.

```csv
row_id,product_ref,search_text,quantity,max_unit_price_minor,currency,substitution_policy
L001,FICTIF-PRODUIT-A,pates exemple 500 g,2,190,EUR,forbid
L002,FICTIF-PRODUIT-B,lait exemple 1 L,6,125,EUR,forbid
```

Le dialecte choisi ici utilise UTF-8, virgule et en-tête exact. Un projet peut déclarer un dialecte français différent ; il doit être enregistré dans la version des données plutôt que deviné à chaque exécution. `quantity` est un entier positif borné. `max_unit_price_minor` exprime le prix maximal d’une unité vendue, en centimes pour EUR ; un prix au kilogramme ne lui est pas substitué silencieusement. Le parseur n’interprète ni formule de tableur, ni commande, ni fragment de locator dans les cellules.

`product_ref` doit correspondre, lors d’une future qualification réelle, à une identité produit observée ou à un mapping validé avec format/conditionnement. Un simple résultat de recherche textuelle ne suffit pas. Si seule la recherche est disponible, plusieurs candidats donnent `AmbiguousTarget` ou une demande de choix ; le replay n’utilise pas un LLM pour choisir le plus ressemblant. Les doublons, produits au poids, substitutions et unités incompatibles doivent posséder une règle explicite ou être refusés.

Le run déclare séparément compte, magasin ou périmètre de livraison, budget total, devise et politique de panier existant. La proposition de référence **préserve les lignes non concernées** et traite la quantité des lignes concernées comme une quantité cible, pas comme une instruction répétable « ajouter N ». Le contrôle du budget indique s’il concerne les seuls articles gérés ou le panier entier. Taxes, frais, promotions et arrondis sont conservés dans le total effectivement observé ; une somme partielle n’est pas affichée comme le prix final de commande.

| Étape prévue | Vérification nécessaire | Résultat en cas d’écart |
| --- | --- | --- |
| Ouvrir le contexte autorisé | Origine, session, compte et périmètre attendus | Arrêt ou attente d’authentification ; aucun autre compte choisi silencieusement. |
| Observer le panier initial | État initial et quantités, identité des lignes, absence de run concurrent médié | État initial inconnu ou conflit conservé ; pas de vidage automatique. |
| Rechercher/résoudre la ligne | Un seul candidat, bonne référence, conditionnement et devise | Ligne rejetée par route publiée, sinon blocage du run ; aucune substitution implicite. |
| Contrôler prix et stock | Prix dans la borne, disponibilité et limite de quantité | Écart métier dans le rapport, sans réparation du sélecteur par défaut. |
| Ajuster vers la quantité cible | Autorité de modification du panier et observation précédente encore applicable | Un effet journalisé ; changement concurrent = arrêt avant le prochain effet. |
| Observer le résultat | Référence et quantité effectivement présentes, total recalculé | `outcome_unknown` si l’effet ne peut être établi ; pas de clic supplémentaire aveugle. |
| Terminer | Toutes les lignes requises rapprochées et panier relu | Sortie métier `cart_status` ∈ {`prepared_cart`, `partial`, `blocked_lines`} évaluée par `completion.success_when` ; ce n’est pas un état de Run ; commande/paiement hors de ce parcours. |

Dans un parcours `for_each` v1.2, une cible absente ou ambiguë, ou un écart prix/stock détecté avant effet, ne conduit à `partial` que si le body publie la route correspondante : ObservationReport `expected_failure` (`safe_to_continue=false`, `effect_state=none`) puis `choice` vers un résultat de ligne rejetée, ce qui fait avancer le curseur. Sans cette route, la ligne échoue, le curseur s’arrête et le run suit `failed` ou `blocked` ([itération de données](../architecture/workflow-data-iteration.md#curseur-durable-et-effets)). Un effet `unknown` ne fait jamais avancer le curseur.

Les mots du bouton, son rôle et ses attributs doivent être appris sur un environnement autorisé. Cette table n’invente pas un `data-testid` Carrefour. Le panier demeure une ressource distante mutable ; un autre appareil peut le modifier malgré une lease locale.

## 5. Locators, attentes et validité

Un locator publié exprime une identité réévaluable : rôle et nom accessibles, libellé, test-id observé ou sélecteur CSS précis justifié. XPath, coordonnées d’écran et script fourni par un modèle sont exclus du profil médié V1. Il porte une portée et une cardinalité attendue. Une liste de fallbacks est possible uniquement si chaque variante est publiée, mutuellement contrôlée et soumise aux mêmes assertions. L’emploi opportuniste de `first()` pour masquer plusieurs correspondances, d’un index visuel changeant ou de `force` pour poursuivre après échec est interdit dans le profil médié de référence.

Les références éphémères d’une observation (éléments WebDriver ou, dans un profil MCP futur, références de snapshot) servent à l’interaction courante. Leur journal peut documenter l’apprentissage, mais le plan publié doit les remplacer par des locators ; le replay les résout à nouveau sur la page actuelle. [Source B10](../research/browser-automation-sources.md)

Les attentes concernent des conditions explicites et possèdent une borne. Une expiration ne constitue pas automatiquement la preuve que le site a changé. La procédure sépare : problème de transport, session expirée, stock/prix divergent, cible absente, cible ambiguë, postcondition contradictoire et runtime incompatible. Une attente ou assertion peut être réessayée sans répéter l’effet qui la précède. [Sources B03–B04, B17](../research/browser-automation-sources.md)

La validité dépend des assertions métier et des hypothèses de contexte. Un hash global du DOM peut détecter un changement, mais ne remplace pas ces critères : publicité ou horloge peuvent modifier le DOM sans invalider la procédure ; un changement de sens d’un bouton peut conserver une structure proche.

### Matrice V1 du pilote de référence

Le pilote de référence V1 du profil `mediated` est WebDriver W3C piloté en Rust (`fantoccini`) sur Chrome for Testing et chromedriver épinglés ([Q-18](../brief/open-questions.md), [architecture](../architecture/browser-execution.md#21-mediated--procédure-fermée-contrôle-rust)). Les noms ci-dessous sont des opérations du catalogue d’actions qualifié d’Unleash (OperationRef), ni des commandes IPC ni des commandes WebDriver.

| Ensemble | Contenu V1 |
| --- | --- |
| Opérations | `open_context`, `navigate`, `observe` (snapshot structuré produit par un script d’adaptateur versionné), `resolve_target`, `click`, `fill`, `select_option`, `set_checked`, `press_key`, `upload_file` (depuis des données en lecture seule), `download` (vers la racine assignée), `wait_for` (condition déclarée et bornée), lecture de texte, de valeur ou d’attribut, capture d’écran, `switch_frame`, `checkpoint` et `close_context`. |
| Locators | Rôle et nom accessible, résolus par Get Computed Role et Get Computed Label sur l’ensemble candidat avec contrôle de cardinalité ; libellé ; test-id (attribut déclaré, `data-testid` par défaut) ; CSS justifié. |
| Exclus | XPath, coordonnées d’écran et `execute_script` fourni par un modèle ; seuls les scripts propres à l’adaptateur, versionnés avec son mapping, sont exécutés. |
| Navigateurs | Chrome for Testing de référence ; Firefox/geckodriver optionnel, hors recette V1 ; Safari non pris en charge en `mediated` V1 (`BrowserCapabilityMissing`). |

Une opération absente de cette matrice est une capacité manquante jusqu’à qualification ; elle n’est jamais remplacée par un script libre.

## 6. Actions sensibles, session et reprise

La classification distingue observation, mutation réversible du projet, mutation distante telle qu’un panier, et engagement sensible : achat, paiement, envoi, suppression ou dépôt définitif. Une classification générique de `click` ne suffit pas : le contrat désigne la cible, l’effet prévu et les données engagées. Le mandat préexistant peut autoriser un engagement déterminé ; sinon le run se suspend avec un résultat concret à examiner. L’approbation porte sur le digest de la version, les données et l’effet courant. Un changement pertinent invalide cette approbation.

Un CAPTCHA, une authentification renforcée ou une session expirée mène à une attente d’intervention prévue : le Run passe `blocked` (`NeedEvidence`, raison `AUTOMATION_HUMAN_STEP_REQUIRED`) et l’étape `waiting_evidence` ; la fin d’intervention est déclarée par `AcceptStep`, puis seule une observation publiée conforme lève le blocage ([F019](../features/F019-reusable-automations/technical-design.md#états-et-erreurs)). La reprise réobserve identité, page et données ; elle ne réutilise pas les anciennes références d’éléments. Aucune API d’authentification privée, aucun contournement et aucune conservation de cookies en clair dans le projet ne sont requis par ce dossier.

Après une pause, une intervention humaine ou un crash, le run reprend seulement depuis un checkpoint reconnu, avec nouvelle génération d’autorité et préconditions revérifiées. Un checkpoint indique les lignes observées, les effets confirmés et les inconnues ; il ne suffit pas de mémoriser « prochaine ligne CSV = 12 ». Les règles de [réconciliation communes](../architecture/failure-recovery.md) s’appliquent aux clics comme aux autres effets.

Une réparation publie une nouvelle version. La première exécution interrompue conserve sa version et son historique. Un nouveau run peut cibler les lignes restant à traiter après réconciliation, avec une référence explicite à son prédécesseur. Il ne reprend pas le même panier au début par défaut.

### Supervision de blocs sans réinterprétation de chaque clic

Un bloc mécanique produit un `ObservationReport` typé : `safe_to_continue`, `effect_state`, `reason`, `assertion_refs`, ainsi que ses références de logs et de données. `safe_to_continue` est un verdict borné aux règles publiées et aux observations disponibles ; une valeur favorable n’accorde aucune permission et n’efface aucun effet inconnu. Les logs fournissent du contexte, pas une preuve suffisante de justesse.

Une branche `choice` explicite du workflow peut router les résultats métier prévus vers une tâche `leader_decision`. Cette tâche reçoit le rapport et les preuves autorisées, sous `allowed_bounded`, avec limites d’appels/coût/temps et décisions de sortie fermées. La collecte d’un rapport peut s’achever normalement tout en attestant que l’opération métier a échoué ou reste inconnue. Les pannes non traitées par une branche publiée restent `failed` ou `blocked` ; ce mécanisme n’invente pas un `catch` ou `on_error` implicite dans le DSL.

Le leader utilise les décisions communes `continue`, `reconcile`, `correct` ou `escalate`. Une observation supplémentaire ou une correction n’est exécutée que dans la branche publiée correspondante ; `correct` ne permet pas de modifier le programme en vol. Le harness et le broker appliquent seuls cette décision : ils revérifient autorité, assertions, préconditions et journal. Un effet inconnu bloque les actions dépendantes et la clôture qui suppose un état rapproché. Une sortie du modèle ne permet ni retry aveugle, ni modification du script, ni changement de locator ou de version en vol.

Exemple : après un bloc de cinq lignes, le script fournit les références et quantités rapprochées, les écarts et les assertions. Le leader peut expliquer un écart de stock et sélectionner la branche déjà autorisée « rapport partiel » ; il ne peut inventer une substitution ou cliquer pour compenser. Le nombre de lignes par bloc et la fréquence des checkpoints appartiennent à la version publiée et restent bornés. Le mode purement mécanique conserve zéro appel même dans cette situation.

## 7. Erreurs et comportements attendus

| Nom de domaine | Forme wire `code/reason_code` ([F019](../features/F019-reusable-automations/technical-design.md#noms-de-domaine-du-profil-navigateur-et-forme-wire)) | Sens | Suite permise sans modèle |
| --- | --- | --- | --- |
| `InvalidAutomationData` | `InvalidArgument/AUTOMATION_INPUT_INVALID` | Schéma, devise, borne, doublon ou identité de ligne invalide | Corriger/importer les données et les revalider. |
| `BrowserRuntimeMismatch` / `BrowserCapabilityMissing` | `CapabilityDenied/CAPABILITY_UNAVAILABLE`, capacité ou runtime manquant en `details_ref` | Moteur, driver, version ou capacité non qualifié ou non préparé ; navigateur non pris en charge, dont Safari en `mediated` V1 | Préparer le runtime épinglé par l’étape explicite sous mandat décrite dans l’[architecture navigateur](../architecture/browser-execution.md#3-contenu-de-la-version-et-du-manifest-de-run), sans commande dédiée, puis créer un nouveau manifest. |
| `AuthenticationRequired` / `AccountMismatch` | `AuthenticationRequired` : Run `blocked` (`NeedEvidence`, `AUTOMATION_HUMAN_STEP_REQUIRED`) ; commande refusée : `ResourceBusy/AUTOMATION_HUMAN_STEP_REQUIRED`. `AccountMismatch` : `ContractViolation/AUTOMATION_ACCOUNT_MISMATCH`. | Session absente/expirée ou compte différent | Intervention autorisée, fin déclarée par `AcceptStep` ; recontrôle avant reprise. |
| `TargetMissing` / `AmbiguousTarget` | `ContractViolation/AUTOMATION_TARGET_MISSING` / `ContractViolation/AUTOMATION_TARGET_AMBIGUOUS` | Cible absente ou non unique | Arrêt de la ligne via route publiée, sinon arrêt du run ; diagnostic et réparation explicite éventuelle. |
| `BusinessConstraintViolated` | `ContractViolation/BUSINESS_CONSTRAINT_VIOLATED` avant effet ; `BUSINESS_POSTCONDITION_FAILED` après effet | Prix, stock, quantité, devise ou bénéficiaire hors contrat | Appliquer uniquement la branche publiée ; sinon attente ou échec. |
| `ExternalStateChanged` | `ContractViolation/EXTERNAL_STATE_CHANGED` | État observé contradictoire avec le checkpoint ou modification concurrente | Reconciliation ; aucune suppression de l’écart par retry. |
| `ActionabilityTimeout` / `AssertionTimeout` | `ContractViolation/ACTIONABILITY_TIMEOUT` / `ContractViolation/ASSERTION_TIMEOUT` avant dispatch ; `OutcomeUnknown/UNRESOLVED_EFFECT` si l’effet peut avoir eu lieu | Attente échue ; la cause reste à qualifier | Capturer les preuves ; retry d’observation borné si prévu. |
| `OutcomeUnknown` | `OutcomeUnknown/UNRESOLVED_EFFECT` | Mutation possiblement effectuée, résultat insuffisamment établi | Bloquer les actions dépendantes et réconcilier. |
| `IsolationUnavailable` | `IsolationUnavailable/AUTOMATION_PROFILE_UNAVAILABLE` | Garantie exigée absente du confinement effectif | Refuser l’exécution qualifiée ; ne pas annoncer zéro modèle sur simple déclaration. |
| `ModelUseForbidden` | `CapabilityDenied/MODEL_CALLS_FORBIDDEN` | Tentative directe ou indirecte d’appel modèle sous `forbidden` | Refus et événement auditable ; pas de fallback. |

Ces raisons se sérialisent dans l’Error commun (classe, `reason_code` et `retry_class`) selon la correspondance de [F019](../features/F019-reusable-automations/technical-design.md#noms-de-domaine-du-profil-navigateur-et-forme-wire), qui fait autorité, et conservent un état d’effet distinct. Une raison de blocage d’état, comme `AuthenticationRequired`, est portée par l’état `blocked` du Run ; l’Error ne concerne qu’une commande refusée pendant l’attente. Un `AssertionTimeout` après dispatch peut donc être associé à `outcome_unknown`. Le code d’erreur ne transforme pas une action distante déjà possible en absence d’effet.

## 8. Opérations, interfaces et admission

F019 fournit les opérations communes : `CreateAutomationDraft`, `SaveAutomationDraft`, `LearnAutomation`, `ValidateAutomationDraft`, `EvaluateAutomationDraft`, `PublishAutomationVersion`, `ImportAutomationData`, `ValidateAutomationData`, `RepairAutomation`, `DeprecateAutomationVersion` et `RevokeAutomationVersion`. Les lectures `Search/GetAutomation*` exposent les profils et rapports sans exécuter de navigateur ou de modèle. Le catalogue canonique précise leurs payloads ; aucun verbe `CreateBrowserRun` ou registre navigateur distinct n’est ajouté ici.

L’exécution utilise `CreateRun` avec `source=automation_version`, puis les mêmes `StartRun`, `PauseRun`, `ResumeRun`, `CancelRun` et `GetRun`. Enveloppes, acteur authentifié, révisions attendues, receipts et contrôle d’autorité suivent le [catalogue commun](../architecture/client-operation-catalog.md). La commande d’analyse/réparation est une mutation suivie, jamais une conséquence cachée de la lecture d’un rapport d’échec.

GUI, TUI et CLI permettent toutes de sélectionner version et données, inspecter le diff, déclencher, suivre, suspendre, réparer et lire les preuves. La TUI peut ouvrir un navigateur système pour l’intervention requise ; elle n’exige pas le lancement de la GUI Unleash. Un compte-rendu textuel conserve lignes traitées, quantités, montants, inconnues et action suivante.

Un replay `forbidden` actif consomme une unité du **pool global d’exécution**, partagé avec les invocations agentes et les `provider_probe` : défaut 5, plafond 50 au total selon la décision Q-15 ([ADR-13](../architecture/adr/ADR-13-scheduler-budget.md)). Il crée zéro invocation agente. Les contextes navigateur ont une réservation mémoire/processus distincte, avec défaut 2 ([ADR-16](../architecture/adr/ADR-16-reusable-automation-and-supervision.md), amendement R7), à mesurer en recette, et bornes de déploiement. Une attente humaine peut libérer l’unité active après checkpoint sûr, tout en conservant la réservation du contexte ; elle ne crée pas une capacité d’exécution gratuite. Les règles canoniques sont celles de [F019](../features/F019-reusable-automations/README.md) et du [scheduler](scheduling.md).

## 9. Critères d’acceptation à réaliser ultérieurement

Cette matrice décrit les validations du futur produit. **Aucun de ces tests n’a été exécuté dans cette révision documentaire.**

| Critère | Scénario de vérification | Résultat exigé |
| --- | --- | --- |
| B-AC01 Rejeu sans modèle | Version qualifiée, données nouvelles, fournisseurs et sampling indisponibles | Le parcours valide réussit avec zéro appel modèle et zéro invocation agente ; seules les ressources d’exécution nécessaires sont réservées. |
| B-AC02 Absence de fallback caché | Cible devenue ambiguë ; hooks de rapport/repair demandant un modèle | Refus `CapabilityDenied/MODEL_CALLS_FORBIDDEN` (`ModelUseForbidden`) pour les hooks, run bloqué sans réparation autonome. |
| B-AC03 Résolution renouvelée | Re-render entre deux étapes, puis deux cibles de même nom | Réévaluation sur DOM courant ; refus de la cible ambiguë. |
| B-AC04 Données bornées | CSV mal formé, quantité négative, doublons, texte ressemblant à du code | Rejet avant mutation ; aucune interpolation exécutable. |
| B-AC05 Contraintes métier | Prix changé, produit ressemblant, conditionnement différent | La bonne identité est exigée et aucune substitution non déclarée n’a lieu. |
| B-AC06 Reprise sans duplication | Coupure après clic d’ajout et avant observation durable | Pas de nouveau clic ; réconciliation attribuable ou état inconnu. |
| B-AC07 Intervention | Session expirée puis authentification autorisée | Compte et préconditions revérifiés ; génération des cibles renouvelée. |
| B-AC08 Autorité sensible | Action finale hors mandat ou montant modifié après accord | Aucun dispatch tant que l’autorité applicable n’est pas établie sur les données actuelles. |
| B-AC09 Concurrence | Deux runs médiés du même panier et modification externe | Lease locale ; divergence externe conservée au lieu d’être écrasée. |
| B-AC10 Publication | Modification de locator, dépendance ou script après évaluation | Digest différent, validation périmée et nouvelle publication requise. |
| B-AC11 Confidentialité | Canaris dans session, DOM, téléchargements et trace | Ni secret dans bundle publié ni exposition non autorisée ; pertes de fidélité signalées. |
| B-AC12 Parité | Même version/données lancées depuis GUI, TUI et CLI | Mêmes préconditions, effets, erreurs et preuves métier. |
| B-AC13 Limites réelles | `project_script` tente réseau/processus hors profil | Refus effectif ou qualification refusée ; une simple sortie JSON « safe » ne suffit pas. |
| B-AC14 Supervision bornée | Blocs publiés avec checkpoint et exception métier prévue | Actions mécaniques sans LLM par clic, task leader seulement sur les branches prévues, budget et provenance visibles. |
| B-AC15 Avis contre preuve | Leader conseille continuer malgré assertion négative ou effet inconnu | Broker refuse la continuation dépendante ou le retry ; le run ne termine pas en succès par l’avis seul. |
| B-AC16 Panne hors contrat | Arrêt du script sans ObservationReport valide | Échec/blocage explicite ; pas de capture d’exception ou réparation inventée par le moteur. |

Le pilote de référence est décidé en R7 ([Q-18](../brief/open-questions.md)). Sa qualification réelle (gate EXT-BROWSER), la capacité du confinement sur macOS/Linux ([Q-05](../architecture/security-boundaries.md)) et le parcours réel Carrefour restent **à prouver par qualification**. L’objectif fonctionnel et la séparation apprentissage/rejeu sont, eux, déjà exprimés par Louis et ne doivent pas être retransformés en questions d’autorisation.
