# F016 — Plan de tests de traçabilité

**État : non exécuté.** Le harness de qualification est à développer en Rust. Les fixtures contiennent des secrets synthétiques, jamais des clés réelles. Les observations d'outils et de providers simulés sont marquées comme fixtures ; elles ne deviennent pas des preuves de compatibilité réseau réelle.

| Test | Exigence / acceptation | Méthode | Preuve attendue |
|---|---|---|---|
| T-F016-001 | REQ-OBS-001 / AC-F016-001 | Terminer worker puis redémarrer, modifier versions courantes des registries | EV-F016-001 : manifeste durable conservant les versions effectives historiques |
| T-F016-002 | REQ-OBS-002 / AC-F016-002 | Injecter claim fausse, observation contradictoire et jugement de correction | EV-F016-002 : trois objets distincts et rapport des écarts |
| T-F016-003 | REQ-OBS-003 / AC-F016-003 | Test ancien snapshot, log tronqué, code inconnu puis candidat modifié | EV-F016-003 : périmètre, complétude et refus d'attribution abusive |
| T-F016-004 | REQ-OBS-004 / AC-F016-004 | Matrice créations/receipts, rebase et changement de rôle/skill entre recherche et effet | EV-F016-004 : rapprochement, antériorité et invalidations |
| T-F016-005 | REQ-OBS-005 / AC-F016-005 | Faute entre state/event/outbox puis duplication de livraison | EV-F016-005 : atomicité de transition, déduplication et curseurs |
| T-F016-006 | REQ-OBS-006 / AC-F016-006 | Capturer octets envoyés au provider simulé et comparer au manifeste | EV-F016-006 : digests/ordre et contenu effectif, sans champ CoT privé |
| T-F016-007 | REQ-OBS-007 / AC-F016-007 | Charge mêlant niveaux agents, dormance, auxiliaires, retries et usage absent | EV-F016-007 : compteurs exacts sur fixture, cardinalité bornée et inconnues |
| T-F016-008 | REQ-OBS-008 / AC-F016-008 | Secrets connus, faux positifs, formes transformées, ACL changeante et cache chaud | EV-F016-008 : sorties expurgées, tests d'accès et limites constatées |
| T-F016-009 | REQ-OBS-009 / AC-F016-009 | Rapports répétés à seq/politique fixes puis événement ultérieur et révocation | EV-F016-009 : stabilité bornée, as_of et changement de vue explicite |
| T-F016-010 | REQ-OBS-010 / AC-F016-010 | Clients lents, perte de connexion, doublons et fenêtre dépassée | EV-F016-010 : convergence, resync et absence d'arrêt de mission |
| T-F016-011 | REQ-OBS-011 / AC-F016-011 | Disque plein, writer indisponible et crash après effet Git | EV-F016-011 : refus avant effet ou état incertain après effet |
| T-F016-012 | REQ-OBS-012 / AC-F016-012 | Suppression d'archive, export autorisé/interdit et rétention d'un contrat actif ; balayage Q-12 sous horloge incertaine, réduction de durée avec délai de 24 h, objet protégé par run non terminal ou version publiée, inventaire des sauvegardes après purge | EV-F016-012 : manifeste d'export, tombstones et octets réellement retirés |

## Oracles indépendants

Le test de contexte compare le flux effectivement transmis au provider simulé au manifeste, pas une copie d'un builder exécutée deux fois. L'oracle de test observe directement le processus et ses fichiers de sortie. L'oracle de rapport calcule ses attentes depuis des événements de fixture écrits séparément, afin de ne pas reproduire uniquement l'implémentation du projecteur.

Le test de masquage ne se contente pas d'une absence du secret dans la GUI : il inspecte stockage prévu, logs exportables, métadonnées, métriques, caches, fichiers temporaires et contexte. Les secrets transformés arbitrairement servent à documenter la limite de détection, pas à promettre que tout est couvert. Les champs explicitement secrets ne doivent jamais dépendre d'une expression régulière tardive pour être protégés.

## Scénarios de charge et faute

Une charge synthétique contrôlée mélange événements fréquents de stdout, événements métier peu fréquents et client lent. Le résultat attendu est une réduction ou perte déclarée de diagnostics facultatifs, tout en conservant les événements requis pour la reprise. Les buffers sont bornés ; une mémoire croissante sans limite est un échec même si le rapport final paraît correct.

Le crash après effet avant journal doit produire des conclusions autorisées par les observations de reprise, notamment `desired_state_present` ou `outcome_unknown`. Le test ne considère pas le rétablissement de la même ref comme preuve de l'auteur de l'effet. Les sorties de tests non exécutés, les étapes non demandées et les données de facturation absentes gardent leur état distinct.

## Critères de livraison des preuves

Chaque EV contient version du schéma, build du harness, seed, environnement, chronologie, séquence de rapport, ACL et fichiers pertinents expurgés. Les captures GUI éventuelles complètent les JSON mais ne remplacent pas la preuve des états du backend. Une incompatibilité de format de rapport doit produire une erreur de version explicite, pas un rendu partiellement trompeur.

Les parcours [F, N et O](../../testing/integration-tests.md) exercent cette feature avec Git, knowledge, clients et reprise. Aucune preuve future n'est marquée réalisée dans ce dossier.

## Complément FINAL-08 — historique et restauration

T-F016-009 scelle un rapport à (streamA, 481), modifie la mission à 482 puis relit le rapport : ses entrées conservées restent celles de 481, sous droits actuels. Une demande portant sur l'état non scellé à 480 (boundary autre que courant) est refusée InvalidArgument ; GetReportSnapshot d'un rapport inexistant ou purgé donne HistoryUnavailable, et des entrées purgées donnent missing_sources. Purger une source produit missing_sources, pas une preuve inventée. Restaurer un backup avec streamB et sequence 481 ne permet ni collision de rapport ni déduplication sur 481 seul. T-F016-010 vérifie resync du client et nouveau snapshot/watermark atomiques. Ces assertions étendent EV-F016-009/010.
