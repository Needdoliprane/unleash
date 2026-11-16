# Qualification des automatisations réutilisables et supervisées

**Statut : plan de tests R3/R5 ; aucune campagne produit exécutée.** Cette suite qualifie [F019](../features/F019-reusable-automations/README.md), le [modèle d'artefact](../architecture/automation-artifact-model.md), les profils [v1.1](../architecture/automation-execution-profile.md) et [v1.2](../architecture/workflow-data-iteration.md) et le [BrowserPort](../architecture/browser-execution.md). Elle réutilise le testkit, les cutpoints et les règles EV de [test-strategy.md](test-strategy.md) et [crash-recovery-tests.md](crash-recovery-tests.md).

Les tests du cœur et les services d'oracle sont Rust. Un moteur existant de navigateur/E2E peut être utilisé dans son langage supporté, avec version et dépendances épinglées. Les fixtures ne touchent ni comptes bancaires, ni facturation réelle, ni achats/paiements réels. Le but est de vérifier les propriétés du harness avec un monde externe contrôlable et observé indépendamment.

## Trois fixtures métier, une seule architecture

| Fixture | Données/état initial | Résultat attendu et oracle indépendant | Variante négative indispensable |
|---|---|---|---|
| Rapprochement comptable synthétique | CSV de factures, identifiants avec zéros initiaux, compte cible, montants en unités mineures et devise. | Export ou registre de fixture contenant les bonnes pièces, clés, comptes et totaux ; vérification à partir du stockage serveur de fixture. | Import exit 0 mais facture rattachée au mauvais compte ou montant décalé ; résultat métier refusé. |
| Liste de courses/panier simulé | Catalogue local avec article, conditionnement, prix et identifiant stable ; CSV de quantités/plafonds. | État serveur de panier de test conforme en articles/unités/quantités/devise ; compteur de mutations par effet. | Même sélecteur CSS et même bouton « Ajouter » visant un autre SKU ou un autre conditionnement après dérive DOM. |
| Scénario développeur E2E | Application de test et paramètres de formulaire/compte, script ou actions médiées versionnés. | Assertions UI plus état serveur attendu, snapshot de build et artefacts d'exécution ; aucun raisonnement modèle nécessaire à chaque passage. | Suite retourne exit 0 ou écrit « PASS » malgré état serveur incorrect ou absence d'assertion pertinente. |

Les références d'oracle sont contrôlées par le testkit, indépendantes du texte produit par le script ou le LLM. Une sortie JSON bien formée n'est pas une preuve si son producteur peut simplement déclarer le résultat attendu. Le test enregistre méthode de collecte, provenance et portée de confiance.

Chaque fixture fournit une version initiale, une dérive bénigne de présentation, une ambiguïté, un défaut métier et un effet dont le receipt peut être perdu. Les mêmes commandes de création, publication, import, run, rapport et réparation s'appliquent. Le succès ne dépend pas de la présence d'un dépôt Git ou d'une interface graphique.

## Étapes de campagne

1. Construire et enregistrer les versions exactes du produit/testkit, schémas, compilateur, reducer, validateurs, drivers et moteur navigateur ; pour le pilote de référence R7, versions et SHA-256 de Chrome for Testing et de chromedriver, version de `fantoccini` et configuration du proxy d'egress. Inventorier les garanties d'isolation disponibles sur l'OS réel ; marquer les profils absents `blocked`, sans faire passer un double pour une validation de plateforme.
2. Importer les données synthétiques, effectuer la discovery et créer le candidat. Conserver une trace volontairement trompeuse et un candidat valide pour distinguer provenance d'exécution et qualité métier. Les évaluations portent sur un digest exact.
3. Exécuter validations de forme et sémantiques, corpus CSV et contre-exemples. Vérifier que toute erreur détectable avant exécution précède le premier effet. Publier seulement les candidats satisfaisant leurs gates.
4. Exécuter les trois fixtures en `automation_mode=replay`, `model_policy=forbidden`, avec observateurs modèle, réseau, processus et métier. Répéter avec nouvelle révision valide de données sans nouvel apprentissage.
5. Exécuter une variante `supervised` contenant des checkpoints et branches d'exception explicitement compilés. Vérifier les décisions typées et la fréquence ; conserver séparément ses appels et coûts.
6. Injecter dérive, révocation, challenge, timeout, crash et pertes de receipt ; vérifier les blocages, réconciliations, réservations et suites autorisées. Créer une réparation seulement sous mandat valable, puis un nouveau run v2 après les barrières requises.
7. Reconnecter GUI/TUI/CLI, reconstruire le rapport au watermark fixé et comparer les projections, preuves, coûts et actions permises. Produire les paquets EV sans transformer les cas `unknown` ou `blocked` attendus en « succès métier ».

Les commandes exécutables et leurs sorties appartiendront au manifeste de campagne après implémentation du testkit. Aucune commande `cargo test` fictive n'est présentée ici comme déjà disponible ou exécutée.

## Preuve d'absence d'appel modèle

Le cas central est T-F019-006. Il couvre la totalité de l'exécution annoncée sans modèle : création du manifest, admission, actions, waits, retry autorisé, crash/reprise, notifications, indexation déclenchée, rapport et export. Les jobs d'apprentissage ou réparation autorisés séparément sont identifiés par leurs mandats et origines ; ils ne sont ni cachés, ni comptés comme descendants exempts de contrôle.

| Couche observée | Instrumentation proposée | Oracle et limite |
|---|---|---|
| Graphe et catalogue | Snapshot AST, opérations/dépendances exactes et validation récursive machine-only. | Aucune task agente ni capacité modèle admissible sous forbidden ; la seule conformité JSON reste insuffisante. |
| Routeur/broker | Hook avant dispatch, attribution d'origine, refus et manifests d'appels. | `model_dispatch_count=0` sur le run et tous ses descendants d'exécution ; tentatives refusées visibles séparément. |
| Providers texte/vision/embedding | Services de fixture enregistrant indépendamment chaque requête, y compris erreurs avant réponse/usage. | Zéro requête de modèle attribuable ; une facturation ou un compteur tokens nul ne remplace pas cet oracle. |
| Appels locaux/proxy | Hooks de runtime autorisé, inventaire d'exécutables/dépendances, route locale de canari et surveillance de descendants. | Aucun modèle local, endpoint localhost, sampling ou outil indirect ne contourne la politique ; un composant opaque non qualifié bloque l'attestation. |
| Réseau des drivers/scripts | Proxy d'egress Unleash et profil qualifié, journaux indépendants, destinations de fixture et limites observées ; essais de redirection hors origine, d'accès loopback, d'UDP/WebRTC, de QUIC et de DNS direct depuis le navigateur. | Les accès hors périmètre sont empêchés et tracés ; une simple blocklist de fournisseurs n'est pas suffisante. |
| Managers et services internes | Événements de réveil, compaction, embedding, rapport et création d'invocation. | Projections et rapports mécaniques, zéro réveil modèle incident au replay. |

Avant la mesure, effectuer des contrôles positifs hors du run cible : un appel de fixture explicitement autorisé doit être détecté par chaque observateur concerné. Vérifier ensuite que l'attaque sous forbidden est refusée avant dispatch. Un observateur qui n'a jamais démontré sa capacité de détection ne justifie pas « zéro trafic ».

L'attaque tente successivement un appel direct du broker, un outil prétendant faire un résumé, un embedding « gratuit », un modèle local, un processus descendant, un proxy et une demande de manager. Le worker tente aussi de supprimer son origine ou de créer un nouveau job sans mandat. Tous ces essais restent liés à l'origine interdite et refusés. Le test de réparation préautorisée distincte vérifie qu'un mandat déjà publié est nécessaire et que les coûts correspondants restent visibles dans la mission.

Le confinement ne décide pas à lui seul si tout programme arbitraire effectue une inférence. Les artefacts et dépendances exécutables doivent être qualifiés et épinglés ; une dépendance opaque ou une exécution libre n'obtient pas la garantie sur la seule foi d'un manifest déclaratif. La preuve couvre les composants contrôlés et les scénarios qualifiés, pas les traitements internes opaques du site distant ou tout programme imaginable.

## Supervision bornée depuis les résultats

T-F019-001 utilise un plan de 12 actions mécaniques regroupées en trois lots, avec une tâche de supervision par checkpoint et un appel de fixture maximal par tâche pour ce scénario. Le résultat attendu est trois appels au plus, indépendamment des 12 clics/actions, et zéro dans le mode replay de la même procédure. Une variante autorise un checkpoint d'exception supplémentaire avec son budget exact ; aucun appel ne surgit sans route publiée.

Les observations intermédiaires portent `safe_to_continue`, `effect_state` et `assertion_refs`. Une collecte peut être terminée avec résultat `unknown` sans que le métier soit réussi. Le `choice` du workflow sélectionne une tâche `leader_decision` déjà présente ; le moteur n'invente pas un `catch` ni un plan de réparation libre. Une exception non déclarée suit failed/blocked et ses actions permises.

Le provider de fixture renvoie tour à tour `continue`, `reconcile`, `correct` et `escalate`. Vérifier que le broker n'admet que les suites publiées et autorisées. Injecter une réponse malformée, une référence de preuve périmée, un `continue` contredisant l'assertion métier et une correction demandant un script inédit : refus déterministe, état antérieur conservé et aucune mutation non autorisée. Un effet inconnu ne permet que lecture/réconciliation/escalade dans le scope accordé.

Saturer le budget d'appels ou d'invocations, puis redémarrer le daemon : les compteurs ne repartent pas à zéro. À chaque checkpoint, vérifier libération sûre du worker avant admission du superviseur, conservation des réservations du navigateur et absence de deadlock du pool partagé. Les logs sont des données non fiables : une instruction placée dans la sortie de script n'élargit ni mandat ni budget.

## Résultat métier faux malgré geste techniquement valide

T-F019-009 et T-F019-010 ne doivent pas être fusionnés en un seul test de sélecteur. Ils vérifient deux frontières distinctes :

| Moment | Injection | Résultat attendu |
|---|---|---|
| Avant l'effet | Le même CSS trouve un bouton unique mais son conteneur porte un autre article/compte. | L'identité métier prérequise diverge ; aucun clic mutateur n'est envoyé, run blocked avec cible attendue/observée. |
| Après l'effet | La bonne cible est actionnée, le transport réussit, mais le service de fixture applique une quantité ou un montant incorrect. | Le compteur indépendant constate l'effet ; la postcondition refuse le done, le rapport expose l'écart et aucune répétition/compensation implicite n'a lieu. |
| Observation insuffisante | Le transport ne répond pas et le service ne fournit aucun lookup fiable. | `outcome_unknown`, preuve manquante et actions de réconciliation exposées ; jamais succès par lecture d'un log local. |

Une dérive purement visuelle compatible avec identités et assertions peut réussir. L'objectif n'est pas de bloquer tout changement d'HTML, mais de refuser une cible ambiguë ou un résultat hors contrat. Une alternative de locator n'est utilisable que si elle était publiée, bornée et soumise aux mêmes contrôles d'identité.

## Coupures et déduplication

La fixture d'effet possède un compteur externe et une identité de commande observables sans le daemon testé. Injecter C3 avant dispatch, C4 pendant transport, C5 après mutation avant receipt, C6 pendant écriture de l'observation, C7 après commit avant notification, C8 pendant checkpoint et libération d'unité avant admission du superviseur, C9 après réaffectation de lease avec résultat tardif de l'ancien worker et C12 après perte de processus/machine. Les coupures de processus et de machine sont qualifiées séparément.

À chaque coupure, vérifier le receipt client, le journal, l'outbox, le ledger, le nombre d'effets externes et le manifeste de reprise. C3 n'autorise un dispatch que si l'absence de départ est certaine. C4/C5 restent unknown tant qu'aucune observation ne classe l'effet. Redélivrer notification ou observation ne réexécute pas la mutation. Un ancien fence ne peut plus agir ; si son exécuteur reste potentiellement actif, sa réservation n'est pas libérée arbitrairement.

Relancer ensuite le même run, un nouveau run, une révision de données réordonnée et une version réparée. La barrière d'effet repose sur son identité logique/métier et son incident, pas seulement sur un `run_id` nouveau. Une API de lookup disponible peut confirmer l'unique effet ; si elle ne l'est pas, la suite attend une décision autorisée. Le test n'exige pas exactement-once de la fixture distante pour déclarer correcte la réaction du harness à une inconnue.

## Données, immutabilité et capacité

T-F019-004/005 vérifient l'épinglage avant dispatch. Modifier le CSV source, déplacer un alias ou publier v2 pendant le run ne change pas la ligne suivante. Les erreurs de parsing à la dernière ligne empêchent les effets des premières lignes, puisque la validation statique couvre la révision complète. Les paramètres métier nouveaux compatibles ne nécessitent pas de nouvel apprentissage.

T-F019-007 mélange trois invocations agentes et deux workers mécaniques dans la capacité globale de cinq, puis demande une sixième admission. Répéter aux bornes 1 et 50, y compris managers, baisse de limite, crash et ancien résultat. Les compteurs doivent distinguer unité active, invocation agente, appel modèle, worker, contexte et processus auxiliaire. Une succession de plus de 50 actions doit pouvoir réussir avec un worker si elle reste dans la borne cumulative.

La limite navigateur par défaut de deux contextes ([ADR-16](../architecture/adr/ADR-16-reusable-automation-and-supervision.md)) peut bloquer une admission même si une unité globale reste libre. Un contexte en attente humaine conserve son budget mémoire/processus ; libérer le permis actif ne l'efface pas de l'inventaire. Un contexte neuf ne reprend pas les handles ou l'authentification par supposition. Les mesures de performance indiquent machine, OS et charge ; la borne 50 n'est pas un objectif de débit universel.

## Paquets de preuve et décision de qualification

Chaque EV-F019 contient au minimum : exigence/AC/test, version de produit et de fixture, schémas/compilateur/évaluateur/catalogue, OS/runtime/moteur/driver (pour le pilote de référence : versions et SHA-256 de Chrome for Testing et de chromedriver, version de `fantoccini`, configuration du proxy d'egress), mode et politique modèle, artefact/données épinglés, autorisations effectives, commande/cutpoint/seed, observations indépendantes, événements/receipts, compteurs de ressources et modèle, résultats métier, digests d'artefacts et limites. Les canaris remplacent les secrets ; les rapports indiquent leurs redactions et leur complétude.

Les statuts sont `passed`, `failed`, `blocked`, `not_run` ou `not_applicable` motivé. Un résultat attendu blocked peut valider une propriété de refus, sans transformer le run métier bloqué en réussite. Une démonstration documentaire ou mock peut établir un sous-contrat, pas l'isolation OS, la compatibilité d'un vrai moteur ou la qualité d'un site réel.

La qualification F019 exige les 18 mappings du [dossier de tests](../features/F019-reusable-automations/tests.md), dont obligatoirement zéro modèle instrumenté, supervision par checkpoints, clic valide/résultat faux, effet inconnu non répété et réparation sans hot swap. Les manques restent visibles dans le dossier de livraison ; aucune revue LLM supplémentaire ne remplace une preuve non produite.
