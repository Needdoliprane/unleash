# F012 — critères d’acceptation

Tous les critères sont **à exécuter** sur l’implémentation future. Les preuves documentaires actuelles ne sont pas des résultats de recette produit.

## AC-F012-001 — API directe

Exigence : REQ-MODEL-001.

- Étant donné : Des profils cloud avec clés référencées et locaux avec mode d’authentification explicite, puis les doubles protocolaires correspondants.
- Lorsque : Démarrer une invocation sur chaque adaptateur natif.
- Alors : Le mock reçoit le bon endpoint/protocole ; aucun faux secret n’est exigé par la voie locale qualifiée et la trace ne dépend d’aucun fichier de CLI d’abonnement.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-002 — Priorité

Exigence : REQ-MODEL-002.

- Étant donné : Agent, rôle, skill principal et projet définissent des préférences concurrentes ; le skill possède des ModelSelectionBinding versionnés ciblant son ID/version/namespace exacts, deux skills auxiliaires et des homonymes dans d'autres scopes.
- Lorsque : Résoudre sous le même snapshot avec un ordre de maps différent, modifier/supprimer un binding après matérialisation, puis présenter une référence active invalide ou un effort natif incompatible.
- Alors : Priorité inter-niveaux et repository > projet > global au niveau skill produisent le même profil ; versions/provenance restent épinglées pour l'invocation en file ou en reprise. Une version removed laisse chercher au scope suivant ; une référence active invalide ou une option incompatible échoue avant réseau. Les auxiliaires n'imposent aucune préférence et aucune configuration n'élargit les grants.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-003 — Capacités

Exigence : REQ-MODEL-003.

- Étant donné : Profil Fable interdit tool_choice forcé et profil compatible sans strict output.
- Lorsque : Exiger ces capacités.
- Alors : CapabilityUnavailable sans requête ; aucune dégradation cachée.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-004 — Streaming

Exigence : REQ-MODEL-004.

- Étant donné : Flux découpé au milieu d’une chaîne JSON.
- Lorsque : Interrompre puis envoyer un fragment terminal dupliqué.
- Alors : Aucun outil sur fragment ; un seul intent après réponse valide.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-005 — Sorties

Exigence : REQ-MODEL-005.

- Étant donné : Réponses HTTP réussies mais refusal/max_tokens/JSON invalide.
- Lorsque : Normaliser ces réponses.
- Alors : Trois résultats non réussis, preuves conservées et aucun task complete.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-006 — Budgets

Exigence : REQ-MODEL-006.

- Étant donné : 50 demandes se disputent un budget inférieur à leur somme.
- Lorsque : Admettre en concurrence puis interrompre une réponse usage.
- Alors : La somme des réservations reste dans le plafond ; inconnu n’est pas zéro.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-007 — Retries

Exigence : REQ-MODEL-007.

- Étant donné : 429 avec Retry-After puis timeout après envoi.
- Lorsque : Déclencher la politique de reprise.
- Alors : Attente durable bornée ; pas retry aveugle de l’effet ambigu.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-008 — Fallback

Exigence : REQ-MODEL-008.

- Étant donné : Second fournisseur disponible mais non autorisé pour le document.
- Lorsque : Provoquer erreur du premier.
- Alors : Aucun transfert au second ; blocage explicable ; cas autorisé tracé.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-009 — Contexte

Exigence : REQ-MODEL-009.

- Étant donné : Documents épinglés, source local_only, résumé puis résumé de résumé, résultat de récupération documentaire et un secret configuré.
- Lorsque : Assembler les contextes v2, retirer/purger une source, révoquer son accès, puis tenter un nouveau contexte distant et une déclassification autorisée ou non.
- Alors : Politiques et filiation transitive persistent ; retrait de source et changement d'invocation ne déclassifient aucun dérivé. Secret absent, exclusions motivées, stockage et transmission interdits absents. Seule une décision explicite autorisée sur les sources, transformation et destination exactes permet l'exception tracée ; filiation inconnue/conflit de conservation bloque le contenu concerné. Les obligations de suppression s'appliquent aussi aux dérivés.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-010 — Sessions

Exigence : REQ-MODEL-010.

- Étant donné : Historique Unleash durable, IDs distants présents uniquement dans des métadonnées de corrélation, blocs natifs opaques qualifiés et source ensuite exclue.
- Lorsque : Reprendre après crash, essayer previous_response_id/conversation ou un équivalent en extension de dialecte, perdre un bloc opaque et changer de fournisseur.
- Alors : Historique autorisé explicitement sérialisé, zéro référence de continuation distante ; paramètres interdits refusés avant réseau. Aucun bloc opaque ne réintroduit une source exclue ni ne passe à un autre fournisseur. Les décisions nécessaires sont reconstituées depuis les sources portables conservées ; pièce essentielle manquante = CONTEXT_INCOMPLETE, sans reconstruction fictive d'un raisonnement privé.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-011 — Usage

Exigence : REQ-MODEL-011.

- Étant donné : Réponse partielle d’usage, prix versionné et historique explicite comportant un bloc opaque qualifié.
- Lorsque : Estimer toutes les entrées du payload, réserver, calculer puis réconcilier ; présenter une métrique opaque dont aucun plafond n'est qualifié.
- Alors : Champs inconnus conservés ; calcul reproductible avec pricing_version et méthode d'estimation. Aucun antécédent explicitement inclus ni bloc opaque n'est compté zéro par défaut ; coût non bornable bloque l'admission. Aucune continuation distante cachée ne complète le contexte. Tarif périmé refusé PRICING_STALE sous block, marqué sous warn explicite ; tarif distant absent refusé PRICING_UNKNOWN ; profil local `pricing=none_local` admis sous ses bornes de tokens et de temps ; nouveau catalogue sans effet sur les réservations engagées (Q-14).
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F012-012 — Accès réel

Exigence : REQ-MODEL-012.

- Étant donné : Modèle documenté mais clé refusée sur un profil qui en exige une ; puis modèle indisponible sur un profil local sans secret autorisé.
- Lorsque : Faire le probe explicitement configuré.
- Alors : Profil non activé et erreur d’authentification ou d’accès modèle explicite ; la voie locale ne réclame pas une clé fictive ; aucun fallback non autorisé.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.


### Précision AC-F012-008/009 — manifests par appel

Dans le cas autorisé, le fallback ne modifie aucun octet du bundle d'invocation ; il produit un manifeste distinct lié au précédent. L'inspecteur retrouve modèle et contexte propres à chaque appel, y compris après redémarrage. Un payload préparé mais non dispatché apparaît comme tel. Une transmission ambiguë est affichée inconnue, jamais comme réception certaine ou coût nul.

## AC-F012-013 — Trois backends locaux et transport

Exigence : REQ-MODEL-013.

- Étant donné : Profils LM Studio, Ollama et llama.cpp, chacun avec endpoint, dialecte, version/configuration et authentification explicites.
- Lorsque : Tester les voies sans/avec token qualifiées, un token invalide, une redirection et un endpoint HTTP non loopback.
- Alors : Chaque backend utilise le protocole prévu ; les cas interdits sont refusés avant transfert de contexte et les secrets restent hors sorties publiques. Un état de santé ne vaut pas preuve d’authentification.
- Limite : doubles et preuve réelle sont séparés ; une documentation officielle ne qualifie pas le serveur installé.

## AC-F012-014 — Capacités liées à une configuration

Exigence : REQ-MODEL-014.

- Étant donné : Deux configurations d’un même modèle dont l’une ne satisfait pas le contrat d’outil, et un snapshot devenu périmé.
- Lorsque : Exiger texte, streaming, outils, sortie structurée ou contexte selon le rôle, puis modifier template/configuration derrière un alias.
- Alors : Seules les capacités qualifiées sont admises ; aucune action sur arguments partiels, aucun champ ignoré silencieusement ; changement pertinent signalé et snapshot requalifié.
- Limite : la réussite d’une fixture ne démontre pas une qualité sémantique universelle.

## AC-F012-015 — Localité et fallback

Exigence : REQ-MODEL-015.

- Étant donné : Politiques `local_only` et `approved_remote`, profils locaux vérifiés, relais distant ou inconnu et fallback cloud.
- Lorsque : Présenter un modèle Ollama cloud derrière loopback, un placement LM Link distant, un proxy inconnu ou une panne locale ; réutiliser aussi un résumé local dans une nouvelle invocation approved_remote.
- Alors : Aucun contexte ni dérivé strictement local ne quitte le placement autorisé ; inconnus refusés ; fallback distant seulement dans le cas explicitement autorisé et sous un nouveau manifeste traçable. L'autorisation générale d'un nouveau run ne supprime pas les restrictions des données héritées.
- Limite : une capture réseau bornée complète les contrôles ; elle ne prouve pas à elle seule l’absence de tout relais futur.

## AC-F012-016 — Ressources et arrêt

Exigence : REQ-MODEL-016.

- Étant donné : Plafond global 5/50, plafond local par défaut V1 d’un appel par serveur (Q-19), serveur froid/chaud et ressource partagée.
- Lorsque : Mettre en concurrence, provoquer file pleine, chargement lent, erreur mémoire et annulation sans preuve d’arrêt.
- Alors : Plafonds et délais respectés, attente durable, erreurs distinguées, état incertain conservé ; aucune relance supposant sans preuve la libération du GPU ni destruction d’un serveur utilisateur partagé.
- Limite : performances et capacité mémoire doivent être relevées sur la configuration qualifiée ; aucune capacité de cinquante générations locales n’est promise.

## AC-F012-017 — Identité, métriques et coûts honnêtes

Exigence : REQ-MODEL-017.

- Étant donné : Réponses avec puis sans build, digest, quantification, template, tokens ou métriques de durée.
- Lorsque : Préparer un appel, redémarrer, produire son résultat et changer le modèle derrière un alias.
- Alors : Manifest immuable, valeurs observées sourcées, absences explicites, identité des poids non inventée ; coût fournisseur, estimation matérielle et métriques inconnues restent distincts.
- Limite : un identifiant modèle lisible n’atteste pas à lui seul des poids ni de la configuration effectivement utilisés.

## AC-F012-018 — Parité et interdiction d’appel local

Exigence : REQ-MODEL-018.

- Étant donné : Même identité/mandat GUI/TUI, préférences de rôle/skill/workflow, run `forbidden` et run supervisé `allowed_bounded`.
- Lorsque : Configurer, tester et inspecter un profil, puis demander appel, résumé, embedding, chargement ou préchauffage dans le run interdit ; exécuter ensuite un checkpoint autorisé.
- Alors : Même configuration et mêmes reçus depuis les interfaces ; aucun appel, chargement ou démarrage de moteur au nom du run interdit ; checkpoint autorisé sous budgets et politique de données.
- Limite : une commande indépendante de test utilisateur ne peut être utilisée comme voie de contournement d’un run interdit.

Les étapes de test et preuves associées sont détaillées dans [local-model-tests](../../testing/local-model-tests.md). Aucun de ces critères n’a été exécuté sur un moteur local durant la rédaction.

### Qualification initiale R6 — AC-F012-013/014/018

Sur installation sans agent/session, une commande generation_probe autorisée prépare un ProviderProbeContext, réserve une unité globale, appelle uniquement le profil/fixture déclaré et produit manifest/résultat. Cinq unités occupées font attendre le probe ; aucune admission supplémentaire. Une demande issue d’un run forbidden est refusée, même si le payload prétend être indépendant. Un timeout ambigu conserve la réserve applicable et ne démarre pas un deuxième probe automatiquement.

## Statut des extensions R6 E05

AC-F012-002/009/010/011/015 ci-dessus sont des critères normatifs proposés. Statut produit : `not_run`. La validation structurelle du schéma v2 et de sa fixture ne valide ni la propagation réelle des politiques, ni un adaptateur fournisseur, ni le calcul de coût. Les étapes et preuves attendues sont décrites dans [tests](tests.md).
