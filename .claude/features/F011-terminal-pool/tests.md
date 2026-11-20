# Tests F011

**Statut : not_run.** Les fixtures de supervision sont de petits exécutables Rust, tandis que la matrice projets appelle aussi leurs outils externes réels. Aucun programme interne Python/JS n'est requis pour produire les tests.

| Mapping | Test et oracle |
|---|---|
| T-F011-001 → AC-F011-001 → EV-F011-001 | Deux acquisitions à barrière ; compteur spawn du host, leases et discovery_trace avec décision reuse/new. |
| T-F011-002 → AC-F011-002 → EV-F011-002 | Injecter réutilisation de PID/PGID avant contrôle puis entre dernière observation et signal, perte d’ancrage/host et handle indisponible ; aucun signal au témoin, refus journalisé. Qualifier handles individuels et contrôle de groupe séparément sur chaque OS. |
| T-F011-003 → AC-F011-003 → EV-F011-003 | SIGKILL daemon pendant sortie PTY, relance même lignée ; hashes/offsets et epoch ; ancien token rejeté, autre authority_generation refusée quel que soit son epoch. |
| T-F011-004 → AC-F011-004 → EV-F011-004 | Test long puis checkpoint, saturation hosts/PTy ; ledger séparé et capture réseau/broker prouvant absence d'appel modèle autonome. |
| T-F011-005 → AC-F011-005 → EV-F011-005 | Barrière après write PTY avant receipt ; fixture compteur d'effet, reprise et zéro répétition automatique. |
| T-F011-006 → AC-F011-006 → EV-F011-006 | Fixture enfant/grand-enfant/détaché ; arbre observé, signaux et rapport final sans kill arbitraire. |
| T-F011-007 → AC-F011-007 → EV-F011-007 | Concurrent bind entre discovery et lancement ; identité du détenteur inconnu conservée et échec readiness. |
| T-F011-008 → AC-F011-008 → EV-F011-008 | Flood stdout/PTY, tokens synthétiques et ANSI malveillant ; bornes, redaction, gap et absence d'effet backend issu du rendu. |
| T-F011-009 → AC-F011-009 → EV-F011-009 | Builds/tests représentatifs Python/TS/Java/Go/Rust/multi ; argv/cwd/env exacts, ToolUnavailable explicite si fixture absente. |
| T-F011-010 → AC-F011-010 → EV-F011-010 | Matrice fermeture GUI, stop mission, delete worktree, crash host, reboot ; état terminal et nombre de générations. |

Le test de PID réutilisé combine un modèle déterministe de l'adaptateur et, lorsqu'obtenable, une observation OS réelle ; il ne dépend pas d'une allocation aléatoire heureuse de PID. Les tests groupes/PTY s'exécutent dans un environnement isolé et ciblent seulement leurs fixtures.

La voie de lancement de T-F011-003 vérifie l’identité d’un processus réellement observé, y compris s’il sort après la capture, et la conservation de sa sortie PTY après le handshake. Dans les contrôles Store, chaque cas fournit un objet de preuve déjà publié dont le digest, le contexte et le dispatch courant correspondent. Un écart positif contrôlé entre capture et reçu, par exemple 25 ms, garde les deux instants. Une capture antérieure au dispatch ou postérieure au reçu doit échouer sur l’ordre temporel. `processes` et `terminal_sessions` restent vides et aucun événement de succès n’est écrit. Le test de reprise après arrêt du daemon reste distinct de ces contrôles de lancement.

La lecture causale est testée après acquisition et après transfert sur le vrai daemon. Comparer la réponse typée à la preuve Store et au watermark du snapshot. Vérifier la lecture ordinaire du reçu, puis les refus sur autre acteur, référence étrangère, mauvais projet, ancien bail, forme invalide et ancienne preuve sans séquence native. Conserver le bail et le journal, et observer une réponse stable sur rejeu de lecture. La preuve de perte d’accusé de T-F011-005 exige encore l’écriture PTY réelle et le compteur d’effets.

Le contrôle de forme de T-F011-005 couvre les charges de 1 et 65 536 octets, les valeurs 0 et 255, le projet obligatoire et les six champs fermés. Vérifier le refus des dépassements, du payload vide, des valeurs hors octet et des champs d’autorité ajoutés. Le banc du daemon vérifie séparément le rapprochement de l’agrégat, de la génération, du bail, du fence et de la séquence avec l’état réobservé.

Les EV contiennent OS/version, gestionnaire de service, identités avant/après, commande normalisée, logs redigés, reservations et résultat. Le test ne déclare pas une sandbox complète ; il démontre exactement quels descendants ont été observés et arrêtés. Toute impossibilité sur un OS reste un blocage de qualification identifié.

Pour la tranche Release de T-F011-010, acquérir un vrai PTY puis retirer son
contrôle avec les acteur, projet, génération, bail et fence exacts. Lire le
résultat causal et le reçu initial, rejouer la commande puis drainer le writer.
Exiger une seule observation et un seul dispatch. Les anciens canaux et une
nouvelle connexion avec l'ancien secret doivent recevoir un refus typé avant
effet, sans accepter EOF comme preuve d'absence d'effet. Les anciennes commandes
publiques d'entrée et de redimensionnement sont refusées, même avec la révision
de contrôle courante. Conserver processus, host, génération, PTY, échéance et les
trois réservations physiques. Une acquisition explicite distincte doit ensuite
obtenir un nouveau bail et un fence supérieur sur le même PTY, sans spawn.

Tester séparément mauvaise portée, acteur usurpé, génération et fence divergents,
puis un marqueur d'effet retenu à C4 et une issue inconnue après restart. Vérifier
zéro reçu Release et l'intégrité de l'action, du dispatch, du contexte, des preuves
et de la barrière initialement inconnus. Tester la perte de l'ACK natif de retrait
sans seconde rotation, son inspection exacte et les refus de preuve absente ou
divergente. Une libération logique de bail seule ne qualifie pas ce parcours.
