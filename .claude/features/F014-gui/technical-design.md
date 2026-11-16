# F014 — conception technique

**Révision : 16 novembre 2026. Statut : conception sur la pile adoptée par ADR-03 (R7), sans prototype ni qualification exécutée.** Les choix arrêtés dans [ADR-03](../../architecture/adr/ADR-03-rust-gui.md) et la [comparaison sourcée](../../research/technology-comparison.md) remplacent l’ancienne exclusion générale des frontends web.

## Couches et propriété de l’état

| Couche | Responsabilité | Frontière |
| --- | --- | --- |
| Présentation HTML/CSS en WebView Tauri | Navigation, filtres, sélection, brouillons, cache de projections et interactions ; renderer React/TypeScript strict (ADR-03), bundle Vite embarqué dans `apps/gui-renderer/` | Ni règles métier autoritaires, ni credential IPC, ni SQL, ni accès système arbitraire |
| Hôte/adaptateur Tauri en Rust (`unleash-gui`) | Valider strictement les DTO de présentation, borner commandes et abonnements, porter l’identité du client commun, adapter les reçus/erreurs | N’héberge pas le scheduler, les jobs, la base ou une seconde machine workflow |
| Client IPC Rust commun (`unleash-client`) | Handshake, transport local, corrélation des command_id, reprise de flux, versions et bornes | Même contrat que CLI/TUI ; une identité déclarée par la WebView ne devient pas une autorité |
| Daemon Unleash indépendant | Autorisation, métier, jobs, état durable, persistance, outbox et contrôles d’effets | Seule autorité métier ; durée de vie indépendante de chaque frontend |

AppState contient seulement l’état de présentation. Les lectures et commandes passent de la WebView à l’adaptateur, puis à l’[IPC commun](../../architecture/ipc-and-apis.md). Un worker I/O borné reçoit les réponses et événements hors thread de rendu. Les charges coûteuses restent backend ; aucun calcul métier ne dépend du rythme de rafraîchissement de l’écran.

Les drafts guidés, procédures et révisions de données sauvegardés sont des agrégats backend ; AppState ne conserve que les modifications locales non soumises et leur base_revision. Un formulaire guidé n’embarque pas de compilateur, de scheduler ou de règles d’autorité parallèles. Le même Run est affiché pour source workflow_version ou automation_version ; source et mode sont discriminés dans les DTO, sans propriété inférée de la présence d’un agent.

Le client démarre par négociation de protocole, snapshot et watermark. Il applique les événements dans l’ordre de sequence, déduplique et recharge un snapshot sur gap ou stream_id changé. Les commandes incertaines sont recherchées avec leur command_id et leur authority_generation conservés, pas réémises sous un nouvel ID ni avec la lignée annoncée par un nouveau Hello. Un refus StaleFence/STALE_AUTHORITY_GENERATION est présenté comme commande d’une lignée antérieure ; une nouvelle intention exige la décision de récupération applicable, un nouveau command_id et un lien causal explicite. Le registre de corrélation client permet de retrouver les soumissions après redémarrage de l’hôte ; il ne remplace ni le journal ni les reçus du daemon. Les brouillons restaurés ne sont jamais soumis automatiquement.

## Cycle de vie local et durable

La GUI se connecte à F001 et peut demander son démarrage via le mécanisme de service utilisateur défini par F001. Le démarrage est idempotent et vérifie le handshake d’une instance existante. Le lancement de la GUI ne crée pas une seconde instance propriétaire de la même base.

Fermer la dernière fenêtre, quitter l’application ou faire crasher WebView et hôte libère abonnements et ressources du client. Les jobs déjà acceptés, leurs hosts et le daemon continuent. L’arrêt d’un job ou du service reste une commande métier explicite, ciblée et assortie d’un reçu ; la fermeture de fenêtre ne l’appelle pas. Ce comportement est à prouver sur le service macOS/Linux réel, y compris sous disparition du parent et politique de restart, pas uniquement avec un daemon lancé au premier plan.

Un binaire empaqueté avec la GUI peut faciliter l’installation. La fonction [sidecar Tauri](https://v2.tauri.app/develop/sidecar/) (v2, consultée le 16/11/2026) ne constitue pas un contrat de daemon durable. F001 reste propriétaire du démarrage, de l’arrêt, de la reprise et des migrations. Les assets UI sont embarqués ; aucun site distant, SSR ou serveur Node applicatif n’est requis pour piloter le service local. Les fonctions nécessitant un fournisseur externe signalent leur indisponibilité sans bloquer la consultation locale.

## Choix du renderer et qualité visuelle

Le renderer adopté par [ADR-03](../../architecture/adr/ADR-03-rust-gui.md) est React avec TypeScript strict, construit par Vite et embarqué ; Node.js et pnpm servent au seul build, versions figées par `pnpm-lock.yaml`. À l’exécution, ni serveur Node, ni SSR, ni CDN, ni code distant. Les composants sont réutilisés plutôt que réécrits : CodeMirror 6 et @codemirror/merge (édition, diff), xterm.js (inspecteur PTY), TanStack Virtual (listes et journaux), React Aria Components (primitives accessibles), @xyflow/react (vue graphe, toujours doublée de l’arbre navigable). Le TypeScript se limite à la vue, la navigation, la saisie et l’adaptation IPC.

Les DTO restent définis en Rust ; les types TypeScript sont générés par ts-rs dans `apps/gui-renderer/src/generated/`, et la CI échoue si la régénération produit un diff. L’hôte désérialise strictement. Les versions et configurations réelles sont figées par la tranche verticale M0, sans API présumée. Source : [Tauri frontend](https://v2.tauri.app/start/frontend/), v2 consultée le 16/11/2026.

Réexamen : un défaut éliminatoire (IME, lecteur d’écran, charge AC-F014-011, fermeture des commandes) se corrige d’abord en remplaçant le composant fautif. S’il est intrinsèque à la WebView ou à Tauri, ADR-03 est rouvert par supersession avec egui/eframe ou Iced comme candidat natif, sans retrait de capacité V1. La revue porte sur un écran mission réaliste : objectif et statut en tête, progression workflow, travaux/responsables, décisions/preuves et contexte. Le design system impose grille, typographie, couleurs sémantiques, densité réglable, focus et états ; afficher un bouton ne suffit pas à qualifier la pile.

## Parité GUI, CLI et TUI

Chaque lecture, mutation et parcours des **58 groupes** de la [matrice de parité](../../design/gui-tui-parity.md) correspond aux mêmes contrats backend. Les trois clients offrent création/révision, guidage, procédures/données F019, conversation, édition/validation/simulation/publication, assignation, connaissances, profils locaux/distants, paramètres, rapports, preuves et interventions. L’autorité ne dépend pas du client ; les erreurs de permission, révision ou effet incertain restent les mêmes erreurs métier.

Une vue graphique d’un workflow possède une représentation d’étapes, liens, branches et conditions navigable au clavier en TUI et exportable en CLI. Un document riche conserve texte, structure disponible, provenance et accès autorisé à l’original. Un média expose ses métadonnées, description/transcription existante et actions de lecture/export adaptées ; l’absence de description est explicite. Les interactions de rendu peuvent différer, mais aucune action ne requiert de démarrer la GUI. La recette compare informations et effets, pas les pixels.

## Parcours guidé et projection d’automatisation

Overview consomme ListGuidedTemplates puis CreateGuidedSetupDraft, SaveGuidedSetupDraft, ValidateGuidedSetup et les lectures de validation/preview. Le template est épinglé ; goal, sources, paramètres, mode, provider conditionnel et policies sont des champs éditables. Le renderer n’appelle pas un LLM pour choisir des defaults ou donner l’apparence d’un aperçu complet. Le mode guidé masque la densité initiale, pas l’accès aux contrats, au script/plan ou à l’historique.

ApplyGuidedSetup rend un reçu et les références des agrégats locaux créés/réutilisés. Les éventuels rôles/agents proviennent de la préparation backend, sans agent préalable demandé à l’utilisateur ; un replay compatible n’en crée aucun par nécessité artificielle. L’application prépare un Run seulement si sa source publiée est prête et ne déclenche jamais StartRun. Les sources encore inconnues et validations périmées sont visibles. Un mandat préconfiguré suffit lorsque sa portée couvre l’action ; la GUI ne rajoute pas une confirmation obligatoire par écran.

Workflows accueille draft/registre/données F019 ; Activity rend la projection AutomationRun de GetRun ; Reports rend les résultats typés et la réparation séparée. Replay/forbidden montre zéro dispatch modèle, tentatives refusées éventuelles et worker mécanique distinct d’un agent. Supervised/allowed_bounded montre blocs, checkpoints/exceptions publiés, tâches leader_decision, budgets, usage connu/inconnu et décisions continue/reconcile/correct/escalate. Les logs restent des données non fiables et les assertions conservent leur verdict ; aucun bouton ne valide une continuation que le broker refuse.

## Modèles locaux et tests explicites

La GUI n’appelle pas directement LM Studio, Ollama ou llama.cpp depuis la WebView. Elle utilise SetProviderProfile/TestProviderProfile et les lectures F012 communes. Le DTO distingue profil/dialecte, auth_mode, SecretRef optionnelle, endpoint, data_locality, placement vérifié/inconnu, capacités déclarées/éprouvées/périmées, état de chargement, santé et limites de ressources. Une clé factice n’est pas exigée pour un backend qualifié sans token.

Le bouton de test indique phase et coût/ressources possibles ; configuration, connexion, inventaire et génération sont distincts. Un rafraîchissement de profil lit des observations et leur fraîcheur ; aucun chargement, benchmark, téléchargement ou génération n’est déclenché par montage d’un composant, polling ou auto-complétion. local_only exige une preuve de placement applicable ; un proxy loopback n’est pas présenté comme garanti local. Le fallback conserve localité et capacités. Le parcours replay n’exige pas de provider et n’effectue pas de probe attribuable à son run. [Contrat local](../../specs/local-models.md).

## Frontière WebView et contenus non fiables

Les règles suivantes sont **adoptées avec ADR-03, à implémenter et tester** ; elles ne sont pas une propriété acquise par le choix de Tauri.

- **Commandes :** registre explicite des commandes accessibles par fenêtre/WebView nommée, schémas stricts, tailles et débits bornés, scopes de projet/ressource validés dans l’adaptateur. Pas de commande proxy universelle qui accepterait un verbe, un chemin ou un script arbitraire. Déclarer les commandes applicatives dans le mécanisme de permissions Tauri et tester le refus hors périmètre ; une capability ne sécurise pas automatiquement tout handler enregistré.
- **Autorité métier :** le daemon revalide identité, autorité, révision et fence. Un champ actor_ref ou un bouton masqué n’accorde aucun droit. Les capacités Tauri réduisent l’exposition du client ; elles ne remplacent pas les permissions métier ni la protection contre un processus local hostile du même compte.
- **Secrets :** socket et credentials restent côté Rust. Les clés API/tokens et le contenu du coffre ne passent ni en props, ni en DOM, ni en stockage WebView, ni en événements. Les parcours de configuration passent par le broker/coffre et une fenêtre native de saisie masquée créée par l’hôte hors WebView, implémentée dans `unleash-platform`, ou un flux d’authentification externe contrôlé avec F017 ; la valeur suit le canal privé BeginSecretImport et le renderer reçoit seulement référence et statut. Aucune API Tauri de saisie sécurisée n’est supposée : cette fenêtre native et son alternative CLI/TUI font partie du gate.
- **Rendu :** texte échappé par défaut, Markdown analysé côté Rust en AST borné puis rendu en éléments React ; `dangerouslySetInnerHTML`, `innerHTML` et `eval` sont interdits par un lint bloquant ; aucun script, handler HTML, HTML actif de document, navigation automatique ou iframe de contenu non fiable dans une WebView privilégiée. Les séquences de contrôle de logs sont neutralisées hors de l’inspecteur PTY contrôlé. Les ouvertures externes passent par une action utilisateur et une validation de schéma/cible.
- **CSP et assets :** activer explicitement une politique de production restrictive adaptée aux protocoles Tauri réellement employés ; scripts/styles/assets locaux, pas de CDN ni de code distant, pas de privilèges IPC pour des URLs distantes. Ni `unsafe-eval`, ni `wasm-unsafe-eval`, ni script inline non haché ; les directives exactes sont figées et testées au M0. Retirer les serveurs, permissions et plugins d’automatisation de développement du build livré.

Sources des mécanismes : [capabilities](https://v2.tauri.app/security/capabilities/), [scopes](https://v2.tauri.app/security/scope/), [CSP](https://v2.tauri.app/security/csp/), Tauri v2 consultés le 16/11/2026. La source capabilities documente notamment le défaut des commandes personnalisées et les limites iframe ; ces détails sont consignés dans la comparaison G3. CSP/capabilities ne corrigent ni un handler Rust vulnérable ni une WebView compromise : maintenir versions, dépendances et tests négatifs reste nécessaire.

## Projections et performance

Les listes d’activité, agents et contexte sont paginées/virtualisées. Les flux de logs sont regroupés avec limite de mémoire, indicateur de retard et accès au journal complet. Pas de rerender de toute l’application par token ni chargement de tous les logs en RAM. Les abonnements WebView/adaptateur sont bornés comme l’abonnement IPC ; fermeture ou client lent libère les ressources sans bloquer le writer.

Budget de qualification proposé sur les machines de référence minimales d’[ADR-14](../../architecture/adr/ADR-14-local-data-oss.md) (amendement R7) : action locale accusée côté interface en moins de 100 ms, état engagé visible p95 inférieur à 500 ms hors panne, interactions p95 sous 100 ms avec le jeu de charge d’AC-F014-011 (1000 agents logiques, 100000 événements, logs soutenus). Ce sont des objectifs à mesurer, pas des performances revendiquées. Qualifier séparément les limites mémoire du renderer, de l’hôte et des files pour éviter de déplacer une fuite entre processus.

## Actions, éditions et erreurs

Une pause affiche pause demandée puis pause atteinte ; un stop de process porte la génération exacte. Les drafts skill/workflow/automation et GuidedSetup sont séparés des versions ou configurations appliquées. Une révision concurrente produit conflit et diff ; elle n’est pas écrasée. Le frontend prévalide la forme pour aider l’utilisateur, mais toute validation métier, application et publication reste serveur. Apply reçu après coupure se retrouve par command_id ; le client ne reconstruit pas une deuxième mission.

Connexion perdue, daemon arrêté, rattrapage, tâche bloquée, agent dormant, fournisseur indisponible et OutcomeUnknown ont textes, icônes et actions adaptés. Les écritures sont désactivées hors connexion ; les brouillons restent modifiables sans fausse soumission. Une réponse IPC tronquée ou incompatible ne devient pas un succès local. Une couleur seule n’est jamais l’information et le contexte supprimé par rétention conserve un marqueur de disponibilité.

## Livraison interne

Définir DTO, registre de commandes exposées, scénarios d’erreurs et matrice de parité, puis implémenter chemin nominal, conflits, reprises et sécurité. Qualifier renderer et daemon sur la matrice de plateformes d’ADR-14, sur la pile adoptée, avec l’accessibilité des parcours avancés. Les tests d’adaptateur utilisent des doubles ; la recette finale emploie un daemon réel, des builds identifiés, des fenêtres non autorisées et des contenus hostiles. Les critères de sortie sont AC/T/EV-F014-001 à 016 ; aucun n’est présenté comme passé ici.
