# Spike 1 M0 — Persistance SQLite (`unleash-store`)

Date : 2026-11-17  
Branche / worktree : `task/g0001-m0/t04` (`crates/unleash-store`, exemple `spike_sqlite`)  
Exigences couvertes : `REQ-PERSIST-001` à `REQ-PERSIST-010`, jalon M0 (`T6`).

## 1. Critères de qualification annoncés avant mesure

1. **Qualification build SQLite embarquée (`rusqlite` `bundled` + `backup`)** :
   - Version SQLite embarquée sans dépendance système, non retirée (`!= 3.52.0`), options WAL (`OMIT_WAL` absent) et `THREADSAFE=1` (`THREADSAFE=0` absent) actives.
   - `sqlite_source_id()` à l'exécution strictement égal à l'ID attendu à la compilation (`STORAGE_BUILD_UNQUALIFIED` en cas de divergence).
2. **Profil PRAGMA effectif du writer unique (file bornée à 8 demandes)** :
   - `journal_mode = wal`, `synchronous = 2` (`FULL`), `foreign_keys = 1` (`ON`), `trusted_schema = 0` (`OFF`), `secure_delete = 2` (`FAST`), `busy_timeout = 250` ms, `wal_autocheckpoint = 0`.
   - Sur macOS : `fullfsync = 1` et `checkpoint_fullfsync = 1`.
   - Rejet immédiat `ResourceBusy/BACKPRESSURE` au-delà de 8 demandes en file.
3. **Budget de latence transactionnelle sous `fullfsync=ON` sur APFS local (`/tmp`)** :
   - Mesure p50/p95/p99 de 200 `CommitCommand` durables séquentiels (état + reçu + événement + outbox dans la même transaction `BEGIN IMMEDIATE`) avec profil release, cible indicative `p95 < 25 ms` sur SSD APFS local (à défaut, passage en batching/compression ou alerte M0).
4. **Bornes WAL, lecteurs `query_only=ON`, export et sauvegarde** :
   - Rejet de toute transaction dépassant `1 MiB` (`1 048 576` octets) de croissance WAL (`TxWalGrowthExceeded`).
   - Activation des checkpoints `PASSIVE` à `16 MiB` (`16 777 216` octets) / 30 s d'inactivité et refus des écritures non critiques à `32 MiB` (`33 554 432` octets, `WalPressureStop`).
   - Interruption et libération réelle d'une transaction lectrice `query_only=ON` dépassant `5 s` permettant la reprise immédiate du checkpoint.
   - Export NDJSON en continu strictement borné à `8 MiB` (`8 388 608` octets) par fragments `<= 256 KiB` (`262 144` octets) sans laisser de fichier final ou de staging résiduel lors d'un dépassement (`ExportQuotaExceeded`).
5. **Couverture complète de `T-F002-001` à `T-F002-010`** :
   - Exécution sans échec ni test ignoré de toutes les suites d'intégration `F002` sur disque local temporaire (`/tmp`).

## 2. Qualification du build SQLite observée

Commande reproductible (profil `--release`, disque APFS local via `$TMPDIR`) :

```sh
cargo run --release -p unleash-store --example spike_sqlite
```

Métadonnées relevées à l'exécution (`SELECT sqlite_version()`, `SELECT sqlite_source_id()`, `PRAGMA compile_options`) :

- **`sqlite_version()`** : `3.54.1`
- **`sqlite_source_id()`** : `2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76`
- **`expected_source_id`** (constante FFI `rusqlite::ffi::SQLITE_SOURCE_ID`) : identique (`match = true`)

Options de compilation (`51` options exactes observées) :

```text
ATOMIC_INTRINSICS=1
COMPILER=clang-21.0.0
DEFAULT_AUTOVACUUM
DEFAULT_CACHE_SIZE=-2000
DEFAULT_FILE_FORMAT=4
DEFAULT_FOREIGN_KEYS
DEFAULT_JOURNAL_SIZE_LIMIT=-1
DEFAULT_MMAP_SIZE=0
DEFAULT_PAGE_SIZE=4096
DEFAULT_PCACHE_INITSZ=20
DEFAULT_RECURSIVE_TRIGGERS
DEFAULT_SECTOR_SIZE=4096
DEFAULT_SYNCHRONOUS=2
DEFAULT_WAL_AUTOCHECKPOINT=1000
DEFAULT_WAL_SYNCHRONOUS=2
DEFAULT_WORKER_THREADS=0
DIRECT_OVERFLOW_READ
ENABLE_API_ARMOR
ENABLE_COLUMN_METADATA
ENABLE_DBSTAT_VTAB
ENABLE_FTS3
ENABLE_FTS3_PARENTHESIS
ENABLE_FTS5
ENABLE_LOAD_EXTENSION
ENABLE_RTREE
ENABLE_STAT4
HAVE_ISNAN
MALLOC_SOFT_LIMIT=1024
MAX_ATTACHED=10
MAX_COLUMN=2000
MAX_COMPOUND_SELECT=500
MAX_DEFAULT_PAGE_SIZE=8192
MAX_EXPR_DEPTH=1000
MAX_FUNCTION_ARG=1000
MAX_LENGTH=1000000000
MAX_LIKE_PATTERN_LENGTH=50000
MAX_MMAP_SIZE=0x7fff0000
MAX_PAGE_COUNT=0xfffffffe
MAX_PAGE_SIZE=65536
MAX_SCHEMA=10000000
MAX_SQL_LENGTH=1000000000
MAX_TRIGGER_DEPTH=1000
MAX_VARIABLE_NUMBER=32766
MAX_VDBE_OP=250000000
MAX_WORKER_THREADS=8
MUTEX_PTHREADS
SOUNDEX
SYSTEM_MALLOC
TEMP_STORE=1
THREADSAFE=1
USE_URI
```

Profil PRAGMA effectif vérifié sur la connexion writer :

| PRAGMA | Valeur attendue | Valeur observée | Statut |
| --- | --- | --- | --- |
| `journal_mode` | `wal` | `wal` | Conforme |
| `synchronous` | `2` (`FULL`) | `2` | Conforme |
| `foreign_keys` | `1` (`ON`) | `1` | Conforme |
| `trusted_schema` | `0` (`OFF`) | `0` | Conforme |
| `secure_delete` | `2` (`FAST`) | `2` | Conforme |
| `busy_timeout` | `250` ms | `250` | Conforme |
| `wal_autocheckpoint` | `0` | `0` | Conforme |
| `fullfsync` (macOS) | `1` (`ON`) | `1` | Conforme |
| `checkpoint_fullfsync` (macOS) | `1` (`ON`) | `1` | Conforme |
| `page_size` | `4096` | `4096` | Conforme |

## 3. Mesures APFS locales (`fullfsync=ON`), pression WAL et export borné

### 3.1 Latences `CommitCommand` sur APFS local (`/tmp`, `--release`, `N = 200`)

Chaque itération exécute un `CommitCommand` complet : vérification d'autorité, lookup reçu, mutation `daemon_state`, écriture d'un événement dans `events`, d'une entrée `outbox`, et du reçu dans `command_receipts` dans la même transaction `BEGIN IMMEDIATE` avec `fullfsync=ON` (`bench_path=/tmp/spike-sqlite-ben-q6g0KH`).

| Métrique | Valeur mesurée |
| --- | --- |
| Minimum | `3 986 µs` (`3.99 ms`) |
| Moyenne | `5 639 µs` (`5.64 ms`) |
| Médiane (`p50`) | `5 961 µs` (`5.96 ms`) |
| `p95` | `6 046 µs` (`6.05 ms`) |
| `p99` | `6 066 µs` (`6.07 ms`) |
| Maximum | `6 084 µs` (`6.08 ms`) |
| `PrepareBackup` (`step(-1)` + copie + manifeste) | `35 984 µs` (`35.98 ms`) |
| `ValidateBackup` (`integrity_check`, `foreign_key_check`, hashes) | `2 020 µs` (`2.02 ms`, `integrity_ok=true`, `fk_violations=0`) |

Le critère indicatif (`p95 < 25 ms` sous `fullfsync=ON`) est largement respecté (`6.05 ms` observés), soit une capacité soutenue d'environ `165` à `177` transactions durables séquentielles par seconde sur ce poste sans assouplir `synchronous=FULL` ni `fullfsync=ON`.

### 3.2 Pression WAL 16/32 MiB, expiration lecteur 5 s et export 8 MiB

| Scénario mesuré | Observation exacte (`spike_sqlite`) | Verdict |
| --- | --- | --- |
| Croissance WAL par transaction (`> 1 MiB`) | Transaction d'insertion `1 200 000` octets rejetée (`tx_wal_1mib_cap_rejected=true`) | Conforme |
| Seuil checkpoint `PASSIVE` (`16 MiB`) sous lecteur retenu `query_only=ON` | `176` checkpoints `PASSIVE` déclenchés automatiquement sans bloquer ni tronquer tant que le lecteur retient un read-mark | Conforme |
| Seuil d'arrêt `WalPressureStop` (`32 MiB` = `33 554 432` octets) | Déclenché après `175` commits de ~128 KiB à `stop_wal_bytes = 33 648 040` octets (`wal_stop_hit=true`) | Conforme |
| Expiration du lecteur (> 5 s à `t = +6 s`) + ticket maintenance | `reader_aborted=true`, `reader_released=true`, `expired_terminated=1`, `passive_ckpts=177`, `active_wal_after_reap=0` | Conforme |
| Export NDJSON borné (`8 MiB` max = `8 388 608`, fragment `256 KiB` = `262 144`) | Arrêt immédiat sur quota à `bytes_written = 8 449 572` octets (`<= 8 388 608 + 262 144`), `ExportQuotaExceeded`, zéro fichier final ni staging résiduel | Conforme |

## 4. Résultats des suites de tests `T-F002-001` à `T-F002-010` et unitaires

Suite exécutée via `cargo test -p unleash-store --all-features --no-fail-fast` :

| Suite | Cas exécutés | Résultat observé | Objet principal vérifié |
| --- | --- | --- | --- |
| Unitaires `src/lib.rs` (`error`, `qualification`) | 2 | `2 passed, 0 failed` | Alignement exhaustif `error_code()`/`reason_code()`/`IpcError` et vérification PRAGMAs writer/reader |
| `T-F002-001` (`t_f002_001_atomic_commit`) | 3 (`t001_child_crash_entrypoint`, `c0_c1_c2`, `sigkill_at_c1_and_c2_real_child`) | `3 passed, 0 failed` | Atomicité 5 tables + cutpoints `C0/C1/C2` + `SIGKILL` processus enfant réel et oracle disque |
| `T-F002-002` (`t_f002_002_wal_profile_and_build`) | 3 | `3 passed, 0 failed` | Qualification build + rejet `3.52.0`/`OMIT_WAL`/`source_id` + PRAGMAs + `owner_uid` + file `8` (`Backpressure`) + `1/16/32 MiB` WAL + lecteur 5 s + export `8 MiB` (`InvalidArgument/SCHEMA_LIMIT_EXCEEDED`) |
| `T-F002-003` (`t_f002_003_outbox_inbox`) | 1 | `1 passed, 0 failed` | Parallélisme outbox, backoff jittered, dédup `inbox`, `dead_letter` au 8e échec, blocage agrégat, réarmement unique `consumer_version`, fenêtre 7 jours (`ResyncRequired`) |
| `T-F002-004` (`t_f002_004_idempotency_lineage`) | 1 | `1 passed, 0 failed` | Rejet `unleash.ipc.v1`, `command_digest` RFC 8785 v2, réutilisation clé après purge -> `IDEMPOTENCY_WINDOW_EXPIRED`, `STALE_AUTHORITY_GENERATION` avant lookup reçu |
| `T-F002-005` (`t_f002_005_migrations`) | 1 | `1 passed, 0 failed` | Garde sauvegarde pré-migration, interruption `C11`, checksum SHA-256 divergent, refus binaire plus ancien sans écriture ni altération `mtime` |
| `T-F002-006` (`t_f002_006_objects_pins_gc`) | 1 | `1 passed, 0 failed` | Objets adressés par contenu, pins préparation/lecture, vraie course concurrente multi-thread sous barrières `C10b`/`C10c` dans les deux ordres + symétrique `Barrier(2)`, `OBJECT_DELETING`, nouvelle génération sur recréation, quarantaine sur fichier étranger |
| `T-F002-007` (`t_f002_007_storage_faults`) | 2 (`busy_low_space_full_ioerr_and_corrupt` + `maintenance_and_object_unlink_faults`) | `2 passed, 0 failed` | Vraie contention `SQLITE_BUSY`, espace bas < 1 GiB, `SQLITE_FULL` (`max_page_count`), `SQLITE_IOERR` / `FaultTarget::{CommandWrite,Maintenance,ObjectUnlink}`, `SQLITE_CORRUPT`/`NOTADB` non destructif |
| `T-F002-008` (`t_f002_008_budgets_capacity`) | 1 | `1 passed, 0 failed` | Budgets multi-comptes atomiques, `unknown` non libéré au timeout, dépassement réel réglé bloquant l'admission, pool 5/50, réserve contrôle `soft`/`hard`, drainage |
| `T-F002-009` (`t_f002_009_leases_fences`) | 1 | `1 passed, 0 failed` | Fences monotones sans retour à zéro, réattribution, rejet `StaleFence`, rejet lignée étrangère, classement `suspect` au redémarrage même lignée |
| `T-F002-010` (`t_f002_010_backup_restore`) | 2 (`restore` + `proptest`) | `2 passed, 0 failed` | `PrepareBackup` `C11a..d` + conservation pins/barrières après redémarrage + `ValidateBackup` + rotation `7+4` Q-12 + import capsule (`inbox_consumptions`, actions `invocation`, `budget_reservations`) + rollback `C11RestoreBeforeActivate` + `RestorationBlocked` (lisible et `source_db_unreadable` `0600`) + `PurgeHistory` tombstone |

## 5. Architecture retenue et limites constatées

1. **Comptabilisation du WAL après réutilisation de fichier sur `PASSIVE`** : SQLite `PRAGMA wal_checkpoint(PASSIVE)` ne tronque pas la taille physique du fichier `-wal` après écriture des frames vers le fichier `.db` (celle-ci reste au pic atteint jusqu'à un checkpoint `TRUNCATE`), et réinitialise le compteur de frames lors de l'écriture suivante. `writer.rs` interroge le couple `(n_log, n_checkpointed)` retourné par `PRAGMA wal_checkpoint(PASSIVE)` dès que la taille physique approche les seuils configurés afin de lever la barrière `WalPressureStop` dès lors que `n_checkpointed >= n_log`.
2. **Surface des verrous en mode POSIX WAL** : les mesures ont été effectuées sur disque local APFS (`/tmp`), conformément aux consignes du dépôt ; le partage réseau SMB hébergeant le code source n'est jamais utilisé pour ouvrir une base SQLite (verrous POSIX/WAL incompatibles avec un système de fichiers réseau).
3. **Portée M0 du schéma `STRICT`** : toutes les tables cœur du dossier `sqlite-design.md` (y compris la table de manifests runtime renommée en `store_runtime_manifests` car le préfixe `sqlite_*` est réservé par le moteur SQLite) sont créées et appliquées dès `V001`/`V002`. L'absence de colonne `sqlite_source_id()` ou de schéma système sur les versions non supportées bloque l'ouverture avant toute écriture applicative.
