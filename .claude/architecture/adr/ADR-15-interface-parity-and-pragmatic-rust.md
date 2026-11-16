# ADR-15 — Rust pragmatique et parité fonctionnelle GUI/TUI

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : architecture et bibliothèques adoptées ; voir la section « Amendement R7 ». Les contraintes de langage et de parité restent confirmées par Louis (révision R2 du 16 novembre 2026). Sources contractuelles : INV-001, INV-041/042, INV-054/056/057 ; H-06/H-16 ; risque R-15. Q-01 et Q-17 sont résolues par R2 ; Q-02 est résolue par [ADR-03](ADR-03-rust-gui.md) en R7.

## Contexte

La formulation initiale « exclusivement Rust » pouvait interdire les ressources web ou conduire à réimplémenter des moteurs déjà disponibles. Louis autorise explicitement bibliothèques, HTML/CSS et choix intelligents. Il précise également qu’un terminal limité aux commandes essentielles ne satisfait pas Unleash : toutes les fonctions de la GUI doivent être accessibles par une interface terminal soignée.

La [politique d’implémentation](../../brief/implementation-policy.md) est la référence. La révision ne modifie ni API-only, ni SQLite, ni les responsabilités hiérarchiques, ni la V1 complète. Le framework ne possède aucune règle d’orchestration.

## Options et objections

| Option | Avantage | Risque et coût | Position |
| --- | --- | --- | --- |
| Tous moteurs réécrits en Rust | Homogénéité apparente | Coût disproportionné, périmètre impossible et contradiction avec la clarification | Rejetée comme interprétation du besoin |
| Cœur Rust, bibliothèques réutilisées, présentation adaptée | Autorité unique, choix visuel ouvert, maintenance concentrée | Frontière de présentation, dépendances et build à qualifier | Retenue comme politique confirmée |
| Deux backends métier GUI et terminal | Équipes UI initialement autonomes | Dérive de permissions, transitions et preuves, doubles corrections | Rejetée |
| Backend commun et terminal limité au pilotage | Livraison terminal plus rapide | Retire éditeurs et fonctions avancées de la V1 demandée | Rejetée |
| Backend commun, GUI et TUI complètes, CLI scriptable | Même produit avec navigation adaptée ; automatisation possible | Deux rendus à maintenir et tests de parité nécessaires | Retenue |

## Décision

Un seul daemon Rust possède les cas d’usage. Les clients partagent protocole, schémas, catalogue d’opérations, identités, erreurs, révisions, reçus et reprise du flux. Le renderer GUI est une surface de présentation, pas une autorité. Les brouillons locaux sont identifiés comme tels jusqu’à l’enregistrement backend.

La GUI suit [ADR-03](ADR-03-rust-gui.md) : Tauri 2 et renderer React/TypeScript, sans mécanisme métier hors Rust. HTML/CSS sont autorisés. Ratatui/Crossterm sont adoptés pour la TUI ; les sources et alternatives figurent dans [la conception terminal](../../design/cli-tui-design.md). ncurses reste une option de réexamen sur incident démontré, non une obligation de réécrire une couche bas niveau.

La [matrice de parité](../../design/gui-tui-parity.md) est un contrat V1. Elle couvre lecture, création, modification, validation, publication, configuration, inspection et interventions selon les mêmes droits. Le terminal adapte graphiques, prévisualisations et interaction souris au clavier et au texte. Un visualiseur externe peut ouvrir l’original ; il ne remplace pas un éditeur ou réglage absent. OAuth peut utiliser le navigateur système depuis chaque interface, conformément au fournisseur.

## Amendement R7

1. Un seul daemon Rust, `unleashd` (crate `unleash-daemon`), possède cas d’usage, autorité, révisions et état ; aucun client ne contient de règle métier.
2. La crate `unleash-client` (transport Unix, handshake, corrélation des `command_id`, reprise du flux) est la seule voie d’accès au daemon pour la CLI, la TUI et l’hôte GUI `unleash-gui`.
3. Binaire terminal unique `unleash` : sous-commandes clap 4 (API derive), version figée au M0 ; `unleash tui` ouvre la TUI plein écran ; aucune autre commande ne passe en mode brut ni n’attend stdin sans option explicite.
4. TUI : Ratatui 0.30 et Crossterm 0.29 (feature `event-stream`), versions exactes figées par Cargo.lock avec une seule version de Crossterm dans le graphe. Un seul lecteur d’événements, l’`EventStream` de Crossterm sous Tokio, et une seule tâche de rendu ; l’I/O métier tourne hors du rendu sur files bornées ; restauration du terminal par les helpers de Ratatui et un hook de panique.
5. Composants terminal, règle de choix : `tui-textarea` pour l’édition et `vt100` + `tui-term` pour le rendu PTY si leur compatibilité avec Ratatui 0.30, leur licence et leur maintenance sont démontrées au M0 ; sinon un module interne `unleash-tui::editor` fondé sur `ropey` et `unicode-segmentation` et un rendu PTY interne sur `vt100`.
6. Parité : `catalog/operations.json` est la source de couverture. Un test CI échoue si une opération rattachée à un groupe PAR n’a pas de commande GUI générée, d’action TUI enregistrée et de sous-commande CLI ; seuls les gestes de présentation locaux nommés dans la [matrice](../../design/gui-tui-parity.md) sont exemptés. Les opérations sans groupe PAR sont hors de ce test : `Hello`, conduit par `unleash-client`, et `RecordClaim`, `RecordObservation` et `RecordJudgment`, réservées au canal worker/harness et jamais exposées à la GUI, la TUI ni la CLI ([ADR-03](ADR-03-rust-gui.md)).
7. La qualification terminal (claviers internationaux, Unicode large et combiné, lecteurs d’écran, restauration sur sortie, panique et signaux interceptables) porte sur la matrice d’[ADR-14](ADR-14-local-data-oss.md).

## Sécurité et pannes

Les commandes du renderer passent par un adaptateur Rust Tauri restreint puis l’IPC authentifié. Les commandes d’application ne sont pas supposées protégées par la seule présence de Tauri : capabilities, périmètres et commandes accessibles sont explicitement configurés et testés. Le daemon réévalue toujours l’autorité. Les documents et logs restent inertes ; les secrets sont traités par une surface de confiance ou un flux externe qualifié et ne deviennent pas données de la WebView. La TUI neutralise les séquences de contrôle non autorisées dans ses panneaux ordinaires ; son mode PTY est distinct.

Perte de fenêtre, de terminal ou de transport ne signifie pas annulation de mission. Après reconnexion, snapshot et watermark rétablissent l’état, et une commande à issue inconnue est recherchée avant tout nouvel envoi. Le rendu terminal restaure ses modes lors des sorties gérables et documente les limites d’un arrêt non interceptable.

## Conséquences sur la V1 et critères de décision

F001 précise la frontière de langage ; F014 et F015 fournissent le même catalogue complet. L’ajout d’une fonction exige ses parcours GUI/TUI et son accès scriptable lorsque pertinent, sans nouvelle implémentation métier. Le coût de deux rendus reste réel : il est planifié dès M0, intégré dès M1 et qualifié entièrement en M5–M7.

Acceptation : AC-F001-008 ; AC-F014-009 à 014 ; AC-F015-001 à 017 ; scénarios O et U. Les preuves comparent les états, autorisations, reçus et événements sur backend réel, montrent les dix espaces et les éditeurs en terminal, exercent déconnexion, redimensionnement, saisie, charge, restauration et accès aux paramètres sensibles. Une bibliothèque sélectionnée n’est pas réputée avoir passé ces tests.

## Sources primaires et limites

[Tauri architecture](https://tauri.app/concept/architecture/), [configuration frontend](https://tauri.app/start/frontend/), [capabilities](https://tauri.app/security/capabilities/), [Ratatui](https://ratatui.rs/), [Crossterm 0.29.0](https://docs.rs/crossterm/0.29.0/crossterm/) ; consultation du 16 novembre 2026. Ces sources établissent les briques disponibles ; leur intégration Unleash, performances et qualité visuelle restent à construire et vérifier.
