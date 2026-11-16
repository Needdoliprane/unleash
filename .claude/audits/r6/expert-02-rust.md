# R6 — Expert 02 Rust Systems Architect

Audit de conception indépendant, 16 novembre 2026. Périmètre : workspace Rust, lifecycle, IPC, supervision et F001/F003/F011. Aucun code produit, compilation ou test OS exécuté. Cette intervention est portée par un sous-agent distinct ; la rédaction groupée décrite dans le protocole initial ne décrit pas cette revue R6.

## Constats

La séparation domaine/protocole/adaptateurs, le writer propriétaire, les capacités restreintes et les réservations durables donnent une base cohérente. Le cœur Rust pragmatique, les moteurs externes et les clients GUI/TUI complets restent compatibles avec cette architecture. Les limites déjà déclarées concernant `spawn_blocking`, logout, PTY perdu, descendants détachés et qualification des versions ne sont pas de nouveaux défauts. Le rejet des clés JSON dupliquées existe dans `schemas/README.md` ; F002 précise déjà les champs du digest de commande.

## Risques et objections

### R6-E02-01 — BLOCKING_INVARIANT : autorité verrouillée au mauvais périmètre

**Fichiers/sections.** `specs/scheduling.md`, ouverture : un seul pool global utilisateur. `features/F001-core-daemon/requirements.md`, REQ-CORE-001, et `acceptance.md`, AC-F001-001 : unicité limitée au couple UID/racine de données. `architecture/runtime-lifecycle.md`, « Machine d’état du daemon », exige un verrou sans fixer son indépendance de cette racine.

**Scénario.** Deux démarrages sous le même UID choisissent deux racines locales distinctes. Chaque instance possède son verrou, sa base et ses cinquante réservations. Chaque test d’unicité actuel peut réussir, alors que l’utilisateur obtient deux autorités et cent exécutions admises.

**Conséquence.** Le plafond partagé agents/replay et l’autorité unique deviennent des propriétés d’instance, contrairement au mandat utilisateur.

**Correction concrète.** Fixer un verrou d’autorité par UID, indépendant de toute racine de données ou de profil configurable ; préciser la sélection unique de son emplacement par l’adaptateur OS. Garder éventuellement un verrou supplémentaire par base. Une autre racine demandée pendant l’activité reçoit un conflit explicite ; son activation exige arrêt et neutralisation des exécuteurs précédents. Aligner REQ/AC/T-F001-001 et H-01.

**Alternative.** Un daemon unique héberge plusieurs profils et conserve un seul ledger d’admission global. Cela ajoute du routage interne ; plusieurs daemons sans arbitre commun ne satisfont pas l’invariant.

### R6-E02-02 — BLOCKING_INVARIANT : observation répétée assimilable à un ciblage sûr

**Fichiers/sections.** `architecture/process-supervision.md`, « Identité et propriétaire », autorise une « observation répétée conservatrice » lorsque le handle stable manque, puis la signalisation du groupe après revalidation. F011 REQ-TERM-002/006 exige pourtant le refus du contrôle lorsque l’identité est insuffisante.

**Scénario.** Après la dernière observation correcte, le leader termine, son identité numérique devient réutilisable, puis l’adaptateur signale le PID/PGID mémorisé. Répéter la lecture ne ferme pas l’intervalle avant le signal. La présence d’un host ne le ferme pas non plus sans contrat de propriété supplémentaire.

**Conséquence.** Un processus étranger peut recevoir un signal malgré une génération initialement validée. C’est une course de ciblage, distincte du risque déjà documenté des descendants échappant au groupe.

**Correction concrète.** Rendre explicite qu’une observation, même répétée, n’autorise jamais seule le signal. Exiger un mécanisme qualifié qui empêche la réutilisation de la cible jusqu’au signal, ou une ancre de groupe effectivement possédée pendant cette opération ; sinon `IdentityUnverified`, état partiel et aucune signalisation numérique de secours.

**Alternative.** Signaler uniquement les individus possédant un handle stable ; rapporter les autres descendants non contrôlables. Linux expose également une portée groupe dans `pidfd_send_signal` depuis 6.9, à qualifier par capacité/version, sans extrapolation macOS. La [documentation primaire](https://www.man7.org/linux/man-pages/man2/pidfd_send_signal.2.html) décrit la course PID et les portées disponibles ; elle ne prouve aucune implémentation Unleash.

## Hypothèses

Le périmètre « global utilisateur » désigne un UID sur une machine. Les racines personnalisées sont possibles, conformément au lifecycle. Aucun besoin multi-hôte ni isolation absolue d’un même UID hostile n’est supposé.

## Recommandations

Corriger ces deux contrats avant délégation du packaging et des adaptateurs OS. Aucune nouvelle bibliothèque ni réorganisation des crates n’est nécessaire pour lever les ambiguïtés documentaires.

## À confirmer

Emplacement effectif du verrou par OS ; mécanisme exact de ciblage du leader et du groupe ; comportement lorsque l’ancre meurt. Ces preuves appartiennent aux futurs builds et essais OS, pas à cette revue.

## Acceptation

- **R6-E02-01 :** démarrages concurrents même UID/deux racines ; une seule autorité admet, et la somme agents/replay respecte 5/50. Ajouter migration de racine avec ancien host vivant.
- **R6-E02-02 :** barrière déterministe entre dernière observation et signal, disparition/réutilisation simulée et témoin étranger ; zéro signal au témoin. Couvrir cible individuelle et groupe sur chaque backend qualifié. L’absence de mécanisme sûr produit un refus observable.

Avis : deux corrections de spécification requises ; aucune certification d’exécutabilité du produit.
