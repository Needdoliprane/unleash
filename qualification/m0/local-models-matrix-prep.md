# Préparation Q-19 — Matrice de qualification des modèles locaux (M0)

**Date : 17 novembre 2026 — Statut : préparation documentaire et inventaire machine (`not_run`, aucun modèle chargé ni testé).**
Références : `.claude/specs/local-models.md`, `.claude/testing/local-model-tests.md`, `.claude/architecture/adr/ADR-17-local-model-providers.md`, `.claude/research/local-model-sources.md`.

---

## 1. Inventaire réel du poste de développement (sans chargement de modèle)

Vérification effectuée en lecture seule sur le poste hôte sans télécharger ni charger de poids en mémoire :

| Élément inspecté | État observé | Commande / Source |
|---|---|---|
| Système hôte | macOS `27.2` (`BuildVersion 26B123`), architecture `arm64`, `Apple M4 Max`, `137 438 953 472` octets (128 GiB mémoire unifiée) | `sw_vers`, `uname -m`, `sysctl -n machdep.cpu.brand_string hw.memsize` |
| Ollama | **Présent** : `/Applications/Ollama.app` (`0.40.13`) et `/usr/local/bin/ollama` (`ollama version is 0.40.13`) | `defaults read /Applications/Ollama.app/Contents/Info.plist`, `ollama --version` |
| Poids Ollama locaux | Aucun manifeste de modèle présent (`~/.ollama/models/manifests` vide) | `ls -la ~/.ollama/models/manifests` |
| LM Studio | **Absent** (`/Applications/LM Studio.app` absent, `lms` introuvable dans `PATH`, `~/.lmstudio` et `~/.cache/lm-studio` absents) | `ls`, `which lms` |
| llama.cpp (`llama-server`) | **Absent** (`llama-server` introuvable dans `PATH`) | `which llama-server` |

Écart constaté : seul Ollama (`0.40.13`) est installé sur ce poste à l'ouverture de M0 ; LM Studio (`≥ 0.4.0`), le binaire `llama-server` (`llama.cpp`) ainsi que les poids GGUF/Ollama des modèles de référence `M1` et `M2` restent à installer/épingler explicitement avant le gel M2 (`GATE-LOCAL` / `EXT-MODEL-LOCAL`).

---

## 2. Matrice de référence Q-19 (`ADR-17` / `local-model-tests.md`)

Cette matrice qualifie des tuples précis sans restreindre les modèles configurables librement par l'utilisateur. Tout tuple non éprouvé demeure `non qualifié` et ses capacités non testées restent indisponibles.

### 2.1 Backends et dialectes de première classe

| Profil | Version / Build cible (gel M2) | Voie d'inférence | Voie d'inventaire | Exigences de localité et d'auth |
|---|---|---|---|---|
| `lmstudio` | Version `≥ 0.4.0` figée au gel M2 | Chat Completions compatible (`/v1/chat/completions`) | API native LM Studio (`/api/v1/models`, instances chargées) | Modes `none` (loopback qualifié) et `token` ; contrôle LM Link distant (`local_only` refusé si placement distant ou non vérifiable) |
| `ollama` | Release stable figée au gel M2 (sur poste : `0.40.13` observée) | API native `/api/chat` (profil compatible OpenAI distinct si livré) | `/api/tags` et `/api/show` | Local sans secret ; relais cloud explicitement désactivé (`disable_ollama_cloud` / `OLLAMA_NO_CLOUD=1`) et vérifié après redémarrage |
| `llama_cpp` | `llama-server` au build figé au gel M2 | Chat Completions compatible, templates Jinja activés | Endpoint modèles / `/health` | Modes sans et avec clé API ; contrôle des options RPC / calcul déporté pour `local_only` |

### 2.2 Modèles de référence Q-19

| ID | Modèle | Format / Quantification | Capacités à qualifier | Contrainte de provenance |
|---|---|---|---|---|
| `M1` | **Qwen3-8B instruct** | GGUF `Q4_K_M` (même fichier GGUF partagé entre LM Studio et llama.cpp) | Texte, streaming fragmenté, appel d'outil unique, sortie JSON contrainte, limite de contexte (`8 192`), annulation en cours de stream | Digest SHA-256 du fichier GGUF et digest Ollama consignés au gel M2 |
| `M2` | **nomic-embed-text v1.5** | Embeddings locaux (GGUF / Ollama) | Génération d'embeddings et contre-tests d'interdiction sous `model_policy=forbidden` | Digest, dimensions et absence totale d'appel sous `forbidden` |

### 2.3 Paramètres d'admission et délais figés V1 (`ADR-17` / `local-models.md`)

| Paramètre | Valeur de référence V1 | Notes |
|---|---|---|
| `max_inflight` par serveur local | `1` | Défaut V1 ; relèvement au-dessus de `1` interdit sans preuve de benchmark par tuple ; s'ajoute au pool global `5` / `50` (`ADR-13`) |
| Fenêtre de contexte configurée | `8 192` tokens | La limite configurée prime sur le maximum théorique de la famille du modèle |
| Plafond de sortie des fixtures | `≤ 1 024` tokens | Borne de qualification déterministe |
| Délai connexion loopback | `2 s` | Connexion TCP vers `127.0.0.1` uniquement (HTTP loopback qualifié, sans redirection) |
| Délai chargement à froid (`cold_start`) | `180 s` | Séparé du délai de premier token et de génération |
| Délai premier token (`TTFT`) | `60 s` | Distingue chargement et démarrage d'inférence |
| Délai génération totale | `300 s` | Timeout global d'une requête bornée |

### 2.4 Plateformes de la matrice Q-11

- **`H1` (bloquant)** : Mac Apple Silicon, 16 Go de mémoire unifiée (macOS 15/26/27). Le poste actuel (`Apple M4 Max`, 128 Go, macOS 27.2) satisfait et dépasse `H1` ; un profil de test contraint à 16 Go reste requis en qualification M2.
- **`H2` (bloquant)** : Linux `x86_64` (Ubuntu 24.04 / 26.04 LTS), CPU seul, 16 Go de RAM.
- GPU dédié NVIDIA : extension non bloquante hors socle minimal V1.

---

## 3. Plan de couverture et contre-tests obligatoires (prévu M2)

Couverture des chaînes `REQ-MODEL-013` à `REQ-MODEL-018` (`T-F012-013` à `T-F012-018`) :

1. **`T-F012-013` (Transport, auth et secrets)** : modes `none` et `token` valide/invalide par backend, refus HTTP non loopback, refus redirection vers origine tierce, canaris secrets absents des logs/erreurs/renderer.
2. **`T-F012-014` (Capacités et handshake)** : texte, stream fragmenté, outil complet vs tronqué, schéma invalide, contexte > `8 192`, champ accepté mais ignoré détecté, `CapabilitySnapshotStale` sur changement d'alias ou template.
3. **`T-F012-015` (Localité stricte `local_only`)** : proxy loopback vers serveur distant, relais Ollama cloud activé (`InferenceLocalityUnknown` / refus) puis désactivé, LM Link distant, cible LAN refusée sous `local_only`, interdiction de fallback cloud silencieux.
4. **`T-F012-016` (Ressources, OOM et annulation)** : file `max_inflight=1`, serveur froid vs chaud, OOM simulé puis réel borné, annulation pendant stream conservant l'état incertain sans libération optimiste de capacité.
5. **`T-F012-017` (Manifests et honnêteté des métriques)** : digest/build/usage absent conservé comme inconnu (jamais `0` ou hash inventé), immuabilité du `ModelCallManifest` entre préparation et dispatch.
6. **`T-F012-018` (Parité GUI/TUI et interdiction `forbidden`)** : configuration/inspection identiques en GUI et TUI, refus de toute requête de chat, préchauffage ou embedding `M2` sous `model_policy=forbidden`.

**Statut global Q-19 à la sortie de `t09` : `not_run` (fiche préparatoire complète ; aucun test local exécuté en M0).**
