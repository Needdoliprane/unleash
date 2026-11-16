# Expérience guidée d’automatisation — proposition R3

**Date : 16 novembre 2026. Statut : conception, non implémentée et non testée auprès d’utilisateurs.** La GUI doit être accessible à un particulier ou à un professionnel non développeur. La TUI offre les mêmes fonctions ; elle n’est pas réservée au mode expert. Ce document complète les [principes d’interface](design-principles.md), les [dix espaces](gui-information-architecture.md) et la [matrice de parité](gui-tui-parity.md), sans créer un second backend.

## Trois décisions d’expérience

**Partir du travail attendu.** L’entrée principale propose « Décrire le résultat » et « Utiliser une procédure recommandée », ainsi que la reprise d’un travail existant. Aucun agent préalable n’est nécessaire. Le produit prépare une procédure par défaut dont les étapes sont lisibles, puis crée ou sélectionne les rôles et agents nécessaires selon le mandat. Il ne demande pas d’abord un repository, un modèle, un nombre d’agents ou une configuration technique de workflow. Les informations techniques deviennent pertinentes lorsqu’elles expliquent une décision, une limite ou un problème.

**Rendre la réutilisation visible.** Préparer une procédure et l’exécuter sont deux actions reconnaissables. La première peut nécessiter des appels modèles et produire une nouvelle version. La seconde choisit les données, contrôle l’aperçu et suit les résultats. Le coût de préparation ne doit pas être confondu avec celui de chaque utilisation.

**Adapter la densité sans retirer des capacités.** Le mode guidé explique les termes et présente les champs nécessaires dans un ordre utile. Le mode expert montre davantage de détails simultanément. Tous deux donnent accès à la procédure complète, aux versions, aux contrats, aux permissions et aux preuves. Le choix GUI/TUI est indépendant de ce réglage.

## Objets affichés et vocabulaire

| Libellé courant | Objet ou information | Détail toujours accessible |
| --- | --- | --- |
| Objectif | Goal, résultat attendu et limites | Critères de fin, révisions, missions et responsables. |
| Travail | Mission et run actif ou historique | Scope, budget, owner, dépendances et états distincts. |
| Procédure | `AutomationArtifact` publié ou brouillon de préparation | Version, provenance, étapes, bindings, contrats, capacités et qualification. |
| Données | `AutomationDataRevision` et paramètres | Fichier source, digest, mapping, règles de parsing, valeurs et lignes rejetées. |
| Sources | Ressources et contextes sélectionnés | Compte, périmètre, permissions, fraîcheur, références et limites d’accès. |
| Aperçu des actions | `EffectPreview` | Effets, cibles, révision examinée, faits observés et incertitudes. |
| Résultats | `RunResult` et artefacts | Ligne par ligne, effets confirmés ou inconnus, preuves et incidents. |

Ces noms sont des libellés de présentation proposés. Ils ne modifient ni les identités backend ni les droits. Un utilisateur qui ouvre les détails retrouve les objets précis, plutôt qu’un résumé irréconciliable avec ce que les agents ou la CLI voient.

## Parcours A — Réutiliser une procédure existante

### 1. Choisir le résultat et la procédure

L’utilisateur choisit, par exemple, « Préparer mon panier ». La fiche indique ce que la procédure fait, ses entrées, sa cible qualifiée, sa dernière qualification et les effets qu’elle peut produire. Elle distingue « Ajouter au panier » et « Valider un achat » si les deux existent dans le catalogue ; choisir l’un n’autorise pas l’autre.

Une recherche montre les procédures applicables et leurs versions. L’absence de candidat peut proposer une préparation ; elle ne lance pas automatiquement un modèle. Une procédure inapplicable explique la capacité ou la permission manquante. L’utilisateur peut consulter son contenu avant de connecter une source.

### 2. Fournir les données et sources

L’utilisateur sélectionne le CSV, le dossier ou les documents requis, puis renseigne les paramètres. Le formulaire est construit à partir du schéma de la procédure. Les libellés donnent unités, formats, valeurs attendues et conséquences utiles. Le mapping des colonnes est explicite et modifiable ; les conversions et lignes invalides ne sont pas cachées.

La sélection d’une source indique son périmètre effectif. Connecter un compte ne vaut pas sélection de tous ses documents. Le résumé montre les sources qui seront lues et, si une préparation avec modèle est demandée, celles susceptibles d’entrer dans son contexte. Une référence de secret et son état de validité sont affichables ; le secret ne se retrouve ni dans le CSV exporté ni dans un aperçu ordinaire.

### 3. Examiner l’aperçu

Le panneau présente version de procédure, révision des données, contexte cible, lignes prêtes, exceptions et actions prévues. Les nombres utilisés dans une maquette ou une fixture sont identifiés comme exemples ; ils ne deviennent pas des résultats observés.

Les faits encore inconnus restent visibles : prix à confirmer lors de la consultation, produit ambigu, document non lu, destination inaccessible. L’aperçu d’un formulaire montre les données qui seraient envoyées ; il ne les soumet pas pour obtenir une capture plus réaliste. Un essai sur une cible de test est une action distincte dont les effets sont annoncés.

La politique modèle est visible dans cette étape. Pour `forbidden`, le libellé proposé est « Rejouer sans LLM ». Un lien explique le périmètre, les outils et les limites de qualification. Pour `allowed_bounded`, le libellé proposé est « Superviser aux étapes prévues » ; il montre les blocs de script, checkpoints, exceptions, usages permis et plafonds. Le modèle lit les résultats et logs pertinents aux points indiqués, sans devoir piloter chaque clic. Le libellé « sans LLM » ne signifie pas « hors ligne » ou « sans intervention » ; la supervision n’est pas une promesse de fiabilité absolue.

### 4. Exécuter et suivre

Préparer applique le setup (ApplyGuidedSetup) ou, pour une procédure publiée choisie directement, crée le Run (CreateRun, source automation_version) et rend ses références. Exécuter appelle ensuite StartRun sur ce Run préparé. Ce sont deux actions distinctes aux reçus visibles ; aucun geste unique ne les enchaîne.

Le bouton d’exécution utilise l’autorisation existante lorsqu’elle couvre le travail. Si une décision supplémentaire est nécessaire, le produit montre précisément les effets et la révision à autoriser. Le mode guidé n’ajoute pas des confirmations répétées sans raison ; le mode expert ne peut pas supprimer les contrôles du backend.

Le suivi donne l’état global, l’étape utile, les lignes traitées et les blocages. Une animation ne suggère pas qu’un agent réfléchit alors qu’un processus attend un téléchargement. Un manager dormant n’est pas présenté comme arrêté en erreur. Pause, annulation et interruption conservent leurs sens distincts ; les effets déjà réalisés restent indiqués.

### 5. Lire et réutiliser les résultats

La première vue indique ce qui a été réalisé, ce qui reste incomplet et ce qui n’a pas pu être confirmé. Les artefacts s’ouvrent depuis ce résumé. Une table par ligne permet de retrouver valeur d’entrée, résultat et preuve ; un total ne masque pas des lignes refusées.

Les actions proposées dépendent de l’état : ouvrir le fichier produit, corriger les données, relancer les lignes éligibles, résoudre un résultat inconnu ou préparer une adaptation. « Réutiliser avec d’autres données » conserve la procédure choisie et crée une nouvelle révision d’entrée. « Tout recommencer » n’est pas le défaut après une coupure dont certains effets sont incertains.

### 6. Retrouver la procédure pour un prochain usage

Le parcours permet de retrouver la version autorisée dans le catalogue et de préparer un prochain Run avec d’autres données. Le contexte réutilisable et les paramètres sont visibles ; les autorisations sont réévaluées au lancement. Rejouer ne signifie pas programmer automatiquement une date. La planification calendaire est exclue de la V1 (Q-21, [registre](../brief/open-questions.md)) ; aucun bouton de façade. La relance explicite et la CLI non interactive appelable par un ordonnanceur externe restent ; voir la [délimitation](../specs/reusable-automations.md).

## Parcours B — Préparer ou adapter une procédure

La préparation part de l’intention, de sources choisies et d’un résultat attendu. Elle peut proposer une fonction native existante, l’utilisation d’une procédure du catalogue ou une nouvelle procédure. L’explication précise le compromis : coût de préparation, maintenance attendue et raisons de préférer une voie à l’autre. Elle ne suppose pas que l’apprentissage navigateur est toujours nécessaire.

Les étapes observées et les choix proposés deviennent une version candidate inspectable. Les paramètres variables sont extraits dans un formulaire ; les règles de correspondance et d’arrêt apparaissent avant publication. Le mode guidé présente une liste d’étapes avec entrées, action et résultat attendu. Le mode expert donne accès aux locators, contrats, dépendances et plan compilé. L’édition est possible dans les deux interfaces, avec diagnostics et conservation d’un brouillon invalide.

Le lancement d’une qualification indique si elle consulte une source, modifie un environnement de test ou produit des effets réels. Les évaluations requises viennent de la procédure et de la politique applicables. Préparation, adaptation et supervision peuvent utiliser un modèle local ou un fournisseur API ; le choix effectif reste inspectable. Le produit ne prétend pas qu’un second LLM vérifie universellement toute procédure.

Après une dérive, l’écran garde le run original, les observations et les effets déjà produits. « Analyser le changement » ouvre une activité distincte ; son utilisation éventuelle de modèle et son budget sont visibles. L’utilisateur compare la nouvelle version à l’ancienne. Une version publiée ne remplace pas une étape en cours ; le parcours explique la reprise ou le nouveau run éligible après réconciliation.

## Catalogue initial de templates (R7)

Décision Q-20 (arbitrage délégué R7). Six templates intégrés sont livrés avec la release : scope global, lecture seule, ID, version et digest. Ils se personnalisent par copie dans un scope projet. ListGuidedTemplates les expose avec l’état de leurs prérequis.

| Template | Usage | Mode et prérequis |
| --- | --- | --- |
| `template.reconcile-csv` | Rapprocher deux exports CSV (GEN-03) ; workflow `demo.reconcile.csv`. **Recommandé par défaut sur un profil vierge.** | `forbidden`, sans dépôt ni agent, owner humain, complétion mécanique. |
| `template.replay-rows` | Rejouer une procédure publiée ligne par ligne avec un nouveau CSV (GEN-02/06) ; profil v1.2 `for_each` (`fixture.rows-replay`). | `forbidden` ; une procédure publiée et, pour une procédure navigateur, un runtime navigateur préparé. |
| `template.supervised-report` | Rapport avec revue de bloc (GEN-05) ; `demo.supervised.report`. | `allowed_bounded`, au plus 1 appel et 1 USD ; un fournisseur actif. |
| `template.learn-browser` | Préparer une procédure navigateur depuis une démonstration ou une description (GEN-01/02). | LearnAutomation sous mandat, budget de conception 2 USD ; fournisseur actif et runtime navigateur. |
| `template.dev-merge-only` | Développer puis intégrer (`workflow.merge-only`), sans test ni audit imposé. | Dépôt enregistré. |
| `template.dev-feature` | Feature parallèle et intégration décidée (`workflow.feature`). | Dépôt enregistré. |

La recommandation est déterministe et n’appelle aucun modèle. Sans fournisseur actif, les templates `forbidden` sont proposés en tête et les autres affichent leur prérequis manquant. Sans dépôt enregistré, les templates de développement ne sont pas recommandés mais restent accessibles. Aucun template n’ajoute de test, d’audit ou de juge non demandé.

## Correspondance GUI / TUI

Les parcours suivants doivent être intégrés à la matrice canonique et aux critères F014/F015/F019. Ils détaillent l’adaptation de présentation ; les opérations restent celles du backend commun.

| Fonction | GUI | TUI équivalente obligatoire |
| --- | --- | --- |
| Choisir un objectif ou une procédure | Accueil, recherche et fiches de catalogue. | Liste filtrable, recherche, fiche et accès à toutes les actions. |
| Créer une mission sans dépôt | Formulaire guidé et sélection de sources. | Même formulaire ; aucun champ repository artificiellement obligatoire. |
| Importer un CSV | Sélecteur de fichier, aperçu tabulaire et mapping des colonnes. | Saisie/sélection de chemin ou source, tableau navigable, éditeur de mapping et diagnostics. |
| Éditer paramètres et règles | Champs typés, aide contextuelle, contrôle des valeurs. | Formulaire complet, unités, aide et mêmes erreurs. |
| Inspecter une source | Aperçu, provenance, fraîcheur, contexte de compte et autorisations. | Sections et tableaux complets, accès à contenu disponible, métadonnées et export autorisé. |
| Inspecter et modifier une procédure | Étapes, détails, source/plan et comparaison de versions. | Arbre ou liste d’étapes, éditeurs intégrés, source/plan et diff accessibles. |
| Lire l’aperçu des effets | Résumé et comparaison avant/après. | Résumé et tableaux avant/après, toutes les incertitudes et mêmes décisions. |
| Choisir la supervision | Choix rejeu/supervision, blocs, checkpoints, exceptions et budgets. | Même choix, liste des blocs, checkpoints, exceptions et budgets éditables selon mandat. |
| Exécuter, attendre, reprendre | Progression, contrôles et reçus. | Progression lisible, contrôles et reçus identiques ; détacher la TUI n’arrête pas le daemon. |
| Traiter MFA ou consentement | Ouverture du navigateur externe et suivi d’état, puis AcceptStep (intervention terminée) sur l’étape en attente ; le backend réobserve avant de lever le blocage. | Déclenchement du même navigateur et suivi depuis la TUI, puis le même AcceptStep (intervention terminée), sans lancer la GUI Unleash. |
| Examiner une dérive | Observations, diff et actions d’adaptation. | Même information textuelle/structurée, provenance, exports et actions ; limite de rendu d’image explicitée. |
| Lire et exporter les résultats | Tableau par ligne, artefacts, rapport et preuves. | Tableau paginé/recherchable, artefacts, rapport complet, preuves et exports. |
| Préparer une réutilisation | Retrouver version, choisir de nouvelles données, aperçu et lancement commun. | Même catalogue, formulaire de données, aperçu et opérations de Run. |
| Passer au niveau expert | Inspecteur et vues détaillées. | Inspecteur, palette et panneaux détaillés ; aucun renvoi à la GUI pour finir. |

Une capture, image ou document complexe peut être ouvert dans un outil externe depuis la TUI si son rendu terminal est limité. Cela ne remplace pas l’inspection native du manifest, des données disponibles, des contrats, des décisions et des preuves. Le lancement d’un navigateur pour le site cible reste distinct du lancement de l’interface graphique d’Unleash.

## États qui doivent rester compréhensibles

| Situation | Information et action attendues |
| --- | --- |
| Aucune procédure applicable | Expliquer le manque ; proposer recherche ou préparation explicitement lancée. |
| Aucun fournisseur modèle configuré | Laisser exécuter les procédures compatibles avec `forbidden` ; expliquer seulement les préparations nécessitant un fournisseur. |
| Premier fournisseur cloud activé | Afficher et faire confirmer ou modifier le plafond global proposé (50 USD) et le plafond projet (20 USD) avant activation, avec le même champ en TUI ; valeurs de départ de [model-routing](../specs/model-routing.md) (Q-07). |
| Intervention humaine requise (MFA, CAPTCHA, session) | Run `blocked`, étape `waiting_evidence` de raison `AUTOMATION_HUMAN_STEP_REQUIRED` : site, cause et action attendue. AcceptStep déclare la fin d’intervention ; seule une réobservation conforme lève le blocage ([machines à états](../architecture/state-machines.md)). |
| Source inaccessible | Nom autorisé, cause et dernière fraîcheur connue ; sélectionner une autre source ou rétablir l’accès. Aucun faux extrait. |
| CSV invalide | Colonne/ligne, valeur, règle attendue et possibilité de correction ; préserver les données initiales. |
| Sélecteur ambigu | Décrire la cible et le nombre de candidats ; montrer les observations disponibles et proposer adaptation ou arrêt. |
| Action peut-être effectuée | État « Résultat à confirmer », cause et procédure de réconciliation ; empêcher un renvoi aveugle. |
| Autorisation manquante | Effet demandé, portée et autorité compétente ; pas un message technique opaque ni une réussite simulée. |
| Déconnexion du client | Date de dernière observation et état de connexion ; réconcilier à la reconnexion sans double commande. |
| Résultat partiel | Comptage complet des lignes, raisons et actions éligibles ; pas un succès global sans réserve. |
| Capacité desktop non qualifiée | Indiquer la capacité non prise en charge et les alternatives connues ; aucune promesse fondée sur le seul nom `DesktopDriver`. |

## Mesure du premier parcours (R7)

Décision Q-20. Les sessions sont modérées, sans télémétrie produit ; les traces locales sont exportées avec le consentement du participant. Cinq participants non développeurs passent en GUI et trois participants en TUI, sur la plateforme macOS de référence, depuis un profil vierge. Ils démarrent le rapprochement (`template.reconcile-csv`), puis réalisent les six tâches de recette : choisir les bonnes sources ; corriger un CSV ; distinguer préparation et exécution ; repérer une ligne au résultat inconnu ; retrouver la preuve d’une action ; adapter une procédure sans écraser l’historique.

Seuils de sortie :

- au moins 4/5 (GUI) et 2/3 (TUI) terminent le rapprochement sans aide ;
- médiane d’au plus 10 min (GUI) et 15 min (TUI) du profil vierge au résultat affiché ;
- 100 % identifient correctement si un modèle est utilisé et repèrent la ligne « Résultat à confirmer » ;
- au moins 4/5 corrigent l’erreur CSV et retrouvent la preuve d’une action ;
- zéro effet externe, probe ou chargement de modèle au simple aperçu ;
- mêmes objets, reçus et événements backend entre GUI et TUI.

Le SUS est relevé à titre indicatif (cible 70 ou plus). Un seuil manqué bloque GATE-GUIDED et entraîne une correction d’UX, sans masquer les limites affichées.

Le changement de mode doit préserver les brouillons, la sélection, les droits et les paramètres. Les erreurs sont testées sur des données longues, un terminal étroit, une navigation clavier et une reconnexion. La réussite ne se mesure pas au seul nombre de clics : l’utilisateur doit comprendre ce qui sera effectué, ce qui l’a été et ce qui reste incertain. Aucun test d’utilisabilité, de performance ou d’accessibilité n’a été exécuté dans ce travail documentaire.

## Reprise d’une préparation et correction des données — R6

Overview propose « Mes préparations » à partir de GetOverview, avec nom, dernière sauvegarde, template et statut. La même liste existe en TUI. Sauvegardé signifie envoyé au backend ; les saisies locales non enregistrées sont signalées comme telles. Ouvrir une préparation appliquée montre son travail existant et ses étapes restantes ; aucun bouton ambigu ne réapplique silencieusement le même draft. « Préparer un autre travail » crée un nouveau draft depuis la référence choisie. Démarrer reste une action distincte couverte par le mandat actuel.

Un diagnostic CSV pointe ligne/colonne/type/valeur autorisée et propose « Corriger cette valeur ». Le client ouvre un formulaire intégré borné ou une cellule de sa copie de travail, navigable entièrement au clavier. L’original et la révision serveur restent immuables. Le diff affiche anciennes/nouvelles valeurs et le nombre de lignes touchées ; les identités métier restent visibles. Aucune formule ou instruction du CSV n’est exécutée. Un éditeur externe reste optionnel.

La copie de travail appartient au client et porte explicitement « non importée » ; elle peut être annulée sans toucher la révision d’origine. Pour enregistrer, le client remet la copie par une source autorisée d’ImportAutomationData définie dans [F019/interfaces](../features/F019-reusable-automations/interfaces.md) : variante `client_upload` (octets transmis par l’adaptateur client de confiance en segments bornés avec digest final, jamais par la WebView) ou `project_file` (chemin relatif sous une racine de projet validée). ImportAutomationData crée une nouvelle révision avec parent_data_revision_ref, digest source et provenance des corrections. Aucun chemin arbitraire du daemon n’est accepté. Le formulaire n’invente pas une mutation de cellule sur la révision engagée. Une validation complète de toutes les données suit, avant tout effet. Après import, GUI et TUI retrouvent la nouvelle révision commune ; les brouillons locaux non importés ne sont pas présentés comme partagés.

La sélection d’une nouvelle révision et d’un nouveau Run est explicite. Des effets inconnus du run précédent conservent leurs barrières de clé métier : « corriger les données » ne signifie ni les réconcilier ni réessayer automatiquement les lignes. Le terminal offre le même formulaire/diff/import/validation et les mêmes erreurs, sans commande shell obligatoire.
