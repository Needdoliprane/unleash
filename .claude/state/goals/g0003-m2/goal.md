# g0003-m2 — Exécutions, modèles et processus

- Statut : in_progress
- Rattaché au programme `g0000-v1` et au Goal Codex existant `01a1eeb2-a996-7291-9f6d-1e99f6534683`
- Branche : `goal/g0003-m2`, préparation issue de `dev@ab2b7ba`, socle publié `dev@8598866` intégré en `cdb16bd`
- Options : merge=oui, dry-run=non

## Objectif

Raccorder l’admission durable, les adaptateurs de modèles et le process-host à l’autorité du daemon. Démontrer les parcours avec SQLite et des processus ou serveurs locaux réels avant la qualification externe.

## Critères de terminé

- [ ] F003. À la limite de cinq unités globales, vingt managers logiques et une charge mixte agents, replay et probes restent sous le plafond. Une admission concurrente ne réserve jamais deux fois la dernière place. Les attentes durables rendent la place après checkpoint sûr et neutralisation. Les anciennes tentatives et lignée sont rejetées.
- [ ] F012. Le chemin produit traverse le writer et le broker avant réseau. Les dialectes OpenAI Responses et Anthropic Messages sont qualifiés séparément, avec usage, 429, flux interrompu, timeout et résultat tardif. Un dispatch ambigu garde sa réservation et ne se répète pas automatiquement.
- [ ] Q-19. Les preuves distinctes LM Studio, Ollama et llama.cpp portent les tuples et versions épinglés, les capacités absentes, le refus du proxy distant en `local_only`, le défaut d’une génération locale active par serveur, OOM, annulation et configuration devenue périmée.
- [ ] F011. Le vrai daemon se rattache au même hôte PTY vivant dans la même lignée après crash. Ancienne capacité, identité non vérifiée et autre lignée sont refusées. Un accusé ambigu après écriture PTY ne cause aucune répétition. La qualification indique précisément OS, gestionnaire de service et couverture du cycle de vie.
- [ ] Vérifications des exigences de F003, F011 et F012, revue indépendante et rapport `qualification/m2/exit.md`. Gates sur la branche du jalon puis sur `dev`, publication et CI vérifiée au SHA publié.

## Plan

| Lot | Résultat et fichiers exclusifs | Dépendance | Vérification | Statut |
| --- | --- | --- | --- | --- |
| f003-policy | Décision Q-15 pure dans `crates/unleash-scheduler` | — | T-F003-005 sur fixtures, 11 cas | done |
| f012-stream | Décodeurs SSE et banc HTTP local de `crates/unleash-model-router` | — | 26 cas, notamment EOF et 429 | done |
| t01-admission | Port transactionnel d’admission dans `crates/unleash-store`, projection des trois types d’unités et décompte durable | Scheduler et Store M1 | 14 cas SQLite, 4 105 entrées avec contrôle admis, concurrence sur la dernière place et checkpoint avant libération | done, intégré en `c9c030c` |
| t02-model-http | Transport OpenAI Responses de `crates/unleash-model-router` derrière le broker, TLS loopback | Décodeurs F012 et frontière `ActionExecutorPort` | 11 essais TLS, 417 tests workspace à `7730f59` | done, intégré en `b8c4314` |
| t03a-host-ack | Test F011 de reprise sur le même host après perte de l’acquittement de rotation | Host et PTY réels, daemon simulé | T-F011-003 ciblé et 35 tests de la crate à `c16e230` | done |
| t03-terminal | Service de terminal et raccord IPC pour le vrai daemon, réutilisation de `unleash-process-host` | Port Store des terminaux et journal broker | T-F011-003 (crash daemon, rotation, même PTY) et états d’issue inconnue | pending |
| t04a-probe-store | Préparation d’un probe sans session, reçu, contexte et lien model call/action dans `crates/unleash-store` | Port V008 et contrat F012 | Migration et rollback SQLite, rejeu du reçu, propriétaire et contexte exacts | in_progress, `/root/m1_store` |
| t04b-probe-wire | Payload fermé de `TestProviderProfile` et décodage dans `crates/unleash-protocol` | Interfaces F012 R6 | Cas phase/limites et refus d’extras, origine et acteur établis par le service | in_progress, `/root/m2_stream` |
| t04-probe | Raccord `TestProviderProfile` authentifié avec propriétaire réel du probe, admission, budget, auteur du dispatch et résultat corrélé | t01, t02, t04a et t04b | Deux dialectes en essai local de bout en bout, refus sans connexion, coupure après dispatch | pending |
| t05-qualification | Gates M2, preuves F003/F011/F012 et matrice Q-19 applicables | t01 à t04 | Résultats réels, limites et publication | pending |

Le Store a un seul écrivain par phase. Les connexions F011 et F012 au daemon se coordonnent sur les mêmes points IPC et de composition. Les appels payants, environnements externes et validations d’OS restent soumis au mandat et aux preuves distinctes du programme.

## Résultat acquis

`5a16c4a` intègre la politique F003 et les flux F012. Leur campagne combinée a passé 249 tests Rust. La fusion `cdb16bd` réunit ces lots et le socle M1 publié. Cargo accepte le verrou hors ligne et le dossier validé compte 0 erreur et 0 avertissement. Le rejeu de la fusion passe avec 406 tests Rust (doctest compris). Les raccords à SQLite, à l’IPC et au vrai daemon restent ouverts.

`b8c4314` intègre le transport TLS OpenAI Responses et reproduit exactement l’arbre vérifié `7730f59`. Les 417 tests Rust comprennent les onze cas de transport sur de vrais sockets TLS avec journal de fixture. L’admission, le budget et la persistance de l’usage sont prévus dans le raccord t04.

`c9c030c` combine le transport et l’admission SQLite V008. Les contrôles du workspace passent avec 431 tests Rust et 0 erreur du dossier. Le Store exécute une décision pure fournie au writer, avec contrôle de l’époque, du jeton et des observations avant engagement. Le raccord de `decide` au daemon, les producteurs d’observations et la réconciliation après changement d’époque restent à tester dans le runtime.

Les raccords t03 et t04 partagent la préparation des actions dans le writer. Pour le terminal, le reçu, la lease et la capacité auxiliaire précèdent le lancement. La transaction `Authorizing` peut alors créer l’action et ses lignes dépendantes. Pour le probe, cette préparation doit relier profil, contexte sans session, model call, référence de clé opaque et budget avant dispatch. Le Store et le broker partagés sont sérialisés avant le raccordement des handlers et des tests propres à chaque parcours.
