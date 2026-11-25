# Retours d’expérience Unleash

Le [skill `retex`](../skills/retex/SKILL.md) utilise cette base pour retrouver les corrections qui s’appliquent au cas rencontré. À [G7](goal-protocol.md#7-clôture-et-rapport), `retro` reprend les mêmes entrées pour comparer leurs effets. Le journal du goal conserve les observations et la fiche donne l’état courant de la livraison.

## RETEX-001 — Raccorder le parcours avant de compter un lot clos

Sur M2, les intégrations ont ajouté le démarrage du run, la création d’agent, des commandes de profils et la lecture de la préférence du skill. Le premier `AssignTask` vérifié consommait encore un bundle publié par le banc. Le suivi répétait `13/16` et détaillait les sous-lots sans identifier assez tôt le premier raccord qui permettrait d’accepter l’un des trois lots ouverts.

Le [protocole de goal](goal-protocol.md#6-boucle-et-conditions-darrêt) demande désormais un résultat d’acceptation, une action et un responsable pour chaque lot ouvert. Dans la [fiche M2](../state/goals/g0003-m2/goal.md#actions-de-clôture-ouvertes), le premier contrôle F003 suit `StartRun`, `CreateAgent` et `AssignTask` sur l’IPC public. L’assertion vérifie que le daemon produit le bundle à partir de la politique autorisée et des sources de contexte, puis observe l’admissibilité. Le rattachement de ce contrôle aux lots est appliqué au suivi. La preuve produit reste à obtenir.

La [rétro des trois lots M2](../state/goals/g0003-m2/journal.md#rétrospective-des-trois-lots-ouverts-de-m2) retrouve le même écart pour la disposition terminal F011 et la génération durable F012. Le [découpage](goal-protocol.md#2-plan-et-découpage) part désormais du plus petit test au point d’entrée produit. Les bancs existants doivent permettre d’observer le bundle émis par le daemon, la disposition et le devenir des ressources, puis l’adoption d’un résultat de génération après consommation unique du permis. Le premier rouge fonctionnel et le vert sur le candidat mesureront l’effet de cette correction pour chaque raccord.

## RETEX-002 — Rejouer le bon binaire

Pendant la qualification de Settings, une réextraction a conservé les dates Git et Cargo a réutilisé le binaire de référence. Le journal attribué au candidat contenait encore les deux échecs de la référence.

Les [règles de test](../rules/testing.md) demandent d’actualiser les dates, de comparer les empreintes et d’établir la provenance du binaire. Le banc IPC a déjà appliqué cette correction. Après recompilation attestée des deux révisions, il a observé deux échecs attendus sur `fcf5db4` et deux succès sur `0350f90`. Voir la [preuve RETEX-002](../state/goals/g0003-m2/journal.md#preuve-retex-002).

## RETEX-003 — Vérifier l’identité réellement engagée

La première revue de la préférence du skill a trouvé deux cas absents du banc. Un rôle de la même portée provenait d’une autre affectation. La dépréciation d’un skill encore épinglé était traitée comme sa révocation.

Pour une lecture issue d’un plan engagé, reprendre le lecteur de l’affectation effective et ajouter les contre-cas d’identité et de cycle de vie au test du contrat. Les deux cas ont échoué sur `9699167`, puis réussi sur `8acdc07`. Le candidat corrigé a été intégré dans `f0c0a20` après deux nouvelles revues. Voir la [preuve RETEX-003](../state/goals/g0003-m2/journal.md#preuve-retex-003).

## RETEX-004 — Vérifier le banc avant de recompiler

La compilation de composition M2 a échoué sur des répertoires en lecture seule. À la reprise, Docker Desktop s’annonçait démarré alors que son API expirait sans répondre. L’état de l’application indiquait seulement que son service hôte était lancé.

Au redémarrage du 22 novembre, les écritures du banc et deux tests du Store ont réussi. Au blocage suivant, l’API a répondu HTTP 200 alors qu’une commande dans le conteneur expirait. Les [règles de test](../rules/testing.md) vérifient maintenant une sortie dans le conteneur avant de tester ses écritures. Le premier effet est observé sur le banc F003. La sonde a isolé l’étape en défaut en 12 secondes et le test IPC est resté non exécuté. La [reprise Docker de V021](../state/goals/g0003-m2/journal.md#reprise-docker-et-validation-rust-v021) conserve les deux résultats verts, et la [rétro](../state/goals/g0003-m2/journal.md#rétrospective-des-trois-lots-ouverts-de-m2) donne le contrôle de reprise du test suivant.

Le 23 novembre, ROOT a essayé plusieurs volumes avant de lire le journal de lancement macOS. Le journal retrouve la demande d’autorisation de Lima, du prévol Virtualization et de llama.cpp, sur SMB et sur les volumes de comparaison. Une expiration seule ne permettait pas de localiser la cause. Les règles de test placent maintenant cette lecture avant un changement de volume ou la préparation de l’invité. La [preuve de lancement](../state/goals/g0003-m2/journal.md#preuve-retex-004--autorisation-de-lancement-macos) situe le blocage au premier prévol et conserve les conditions de reprise.

Sur la fixture Ollama de Q-19, le port était en écoute pendant la détection du GPU. La première sonde a interrompu ce démarrage. Après correction du banc, deux sondes ont expiré puis l’API a répondu sur le même PID, avant l’échéance de démarrage. Après un autre arrêt, la lecture du manifeste v2 a retrouvé l’import engagé et permis de reprendre avec le modèle existant. Voir la [reprise de la fixture Ollama](../state/goals/g0003-m2/journal.md#reprise-de-la-fixture-ollama).

## RETEX-005 — Valider les expressions du workflow avant intégration

La rétro G7 du cycle Settings a retrouvé trois conditions `jobs.*.if` invalides dans le workflow publié avec M1. Elles utilisaient `hashFiles`, disponible au niveau des étapes après le checkout.

Les trois manifestes sont requis et suivis. Le correctif retire ces conditions de job et prend la version de pnpm dans le manifeste du renderer. `actionlint` relève trois erreurs sur le workflow publié et passe sur le candidat. Les [règles de test](../rules/testing.md) prévoient ce contrôle pour chaque workflow modifié. Le résultat de la CI distante reste à observer. Voir la [preuve du correctif CI](../state/goals/g0003-m2/journal.md#preuve-du-correctif-ci).

## RETEX-006 — Identifier le chargement natif bloqué sur SMB

Le banc Settings est resté muet avant les tests. Un échantillon a situé l’attente dans `dlopen`. Après le remplacement des compilateurs natifs par leurs distributions WebAssembly, le même type d’attente a persisté et `lsof` a identifié le fichier facultatif `fsevents.node` sur SMB.

Le banc ponctuel redirige esbuild et Rollup vers les distributions de la même version et écarte le watcher facultatif. Le build Vite se termine et Vitest exécute les quatre cas ciblés de validation de configuration. Les [règles du renderer](../rules/gui-renderer.md#contrôles-ponctuels-sur-smb) donnent la condition d’emploi. La [preuve RETEX-006](../state/goals/g0003-m2/journal.md#preuve-retex-006) indique la portée qualifiée.

## RETEX-007 — Qualifier un script de banc par sa première exécution réelle

Le 24 novembre, le script de reprise du banc Docker s’est arrêté à son contrôle d’inactivité : `docker top -eo comm=` est refusé par le démon (« Couldn’t find PID field »), qui exige une colonne PID dès que des options `ps` sont fournies. Le script avait passé `sh -n` et ShellCheck, jamais son mode `run`. Quatre autres défauts des scripts du banc ne sont apparus qu’à l’exécution.

Le contrôle d’inactivité demande maintenant `-eo pid,comm`, en-tête ignoré, et refuse un banc occupé. Les [règles de test](../rules/testing.md) demandent d’exécuter chaque chemin du banc sur un cas réel avant une longue série. Appliquée à la promotion du banc dans `tools/bench/` : chaque chemin exécuté et relu avant le commit `58ae0f2` (codes `0`, `1`, `101`, `2` et `3`, banc occupé, réseau absent), puis sur cette révision `cargo fmt`, l’installation hors ligne, `run build` et les cinq contrôles renderer de `gates.sh`, tous code `0`. Le même contrôle a rencontré une saturation d’entrées-sorties de la machine virtuelle Docker : un appel de 15 secondes a expiré pendant un transfert, sans message, et le répertoire vide laissé derrière a été retiré avant de le rejouer, vert.

Le même soir, une instance de `bench.sh` a rendu une erreur de syntaxe parce que son fichier avait été modifié en place pendant son exécution : le shell lit un script commande par commande. Les scripts de `tools/bench/` se relisent donc une fois en mémoire à leur lancement, chacun pour soi, y compris celui qu’un autre lance. Dans le banc, une instance en cours a fini avec son marqueur après une réécriture de `bench.sh`. Lancé dix fois, réécrit entre 100 et 500 ms : neuf contrôles normaux, contre six sorties à `0` sans contrôle et quatre erreurs de syntaxe sans la garde. Non couvert : une réécriture qui chevauche la lecture initiale ; réécrit 3 à 80 ms après le lancement, le script gardé a encore été faussé sept fois sur huit, dont quatre sans bruit.

## RETEX-008 — Ne pas partager un volume de compilation entre arbres différents sans verrou ni marque d’arbre

Le 25 novembre, pour éviter des compilations à froid sur un disque de VM lent, six bancs de rédacteurs ont monté le volume de compilation du banc M2. Les noms d’artefacts des crates du workspace ne dépendent pas du chemin des sources : un seul `libunleash_store-<hash>.rlib` servait plusieurs arbres, et la fraîcheur repose sur les dates. Un rédacteur a lié du code d’un autre arbre (erreurs `E0432` et `E0425`, journal `evidence/g0003-m2/settings-policy-20261125/red-existing-cbf99d4.log`) ; un autre a trouvé à l’emplacement partagé un `unleash-process-host` venu d’un banc voisin. Les contrôles obtenus sur ce volume de 00:17 à 01:09 UTC ne prouvent rien.

`bench.sh` impose désormais un verrou global (un seul contrôle à la fois : `unleashd` et `unleash-process-host` déversés sont des chemins uniques) et une marque d’arbre (si un autre arbre a compilé depuis le dernier contrôle de celui-ci, ses sources sont rendues plus récentes que les artefacts partagés, ce qui force la recompilation des crates du dépôt ; les dépendances du registre restent réutilisées). Un vert n’est accepté que si le journal montre `Compiling unleash-<crate>` pour les crates concernés. Rejoué le même jour sur un second banc, avec deux arbres compilés tour à tour : `Compiling unleash-domain` au troisième contrôle, et une commande `pnpm` laisse la marque inchangée.

Second manquement, du même auteur : ce correctif a été déployé chez des rédacteurs actifs sans être qualifié par exécution réelle, à la différence du verrou. Deux défauts (délai du contrôle écrasé par `dock` ; sous-commande `renderer` en échec avant `pnpm`) ont été relevés par deux rédacteurs, puis corrigés et qualifiés. Un troisième correctif, préventif : une commande `pnpm` n’écrit pas la marque. Voir la preuve dans le [journal du programme](../state/goals/g0000-v1/journal.md), section « 25 novembre — deux défauts de l'outil de banc sur volume partagé, et première exécution de `cargo deny check` (00:45–01:15 UTC) ».
