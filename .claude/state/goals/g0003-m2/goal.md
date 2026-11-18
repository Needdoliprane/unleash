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
| t01-admission | Port transactionnel d’admission dans `crates/unleash-store`, projection des trois types d’unités et décompte durable | Scheduler et Store M1 | SQLite, mélange 5/6, concurrence sur dernière place, checkpoint avant libération | in_progress, `/root/m1_store` |
| t02-model-http | Transport borné de `crates/unleash-model-router`, premier dialecte testé avec TLS loopback | Décodeurs F012 et frontière `ActionExecutorPort` | Certificat valide, refus avant réseau, 429/EOF sans second POST | in_progress, `/root/m2_stream` |
| t03-terminal | Service de terminal et raccord IPC pour le vrai daemon, réutilisation de `unleash-process-host` | Port Store des terminaux et journal broker | T-F011-003 (crash daemon, rotation, même PTY) et états d’issue inconnue | pending |
| t04-probe | Raccord `TestProviderProfile` authentifié avec propriétaire réel du probe, admission, budget, auteur du dispatch et résultat corrélé | t01 et t02 | Deux dialectes en essai local de bout en bout, refus sans connexion, coupure après dispatch | pending |
| t05-qualification | Gates M2, preuves F003/F011/F012 et matrice Q-19 applicables | t01 à t04 | Résultats réels, limites et publication | pending |

Le Store a un seul écrivain par phase. Les connexions F011 et F012 au daemon se coordonnent sur les mêmes points IPC et de composition. Les appels payants, environnements externes et validations d’OS restent soumis au mandat et aux preuves distinctes du programme.

## Résultat acquis

`5a16c4a` intègre la politique F003 et les flux F012. Leur campagne combinée a passé 249 tests Rust. La fusion `cdb16bd` réunit ces lots et le socle M1 publié. Cargo accepte le verrou hors ligne et le dossier validé compte 0 erreur et 0 avertissement. Le rejeu Rust combiné et les raccords à SQLite, à l’IPC et au vrai daemon restent ouverts.
