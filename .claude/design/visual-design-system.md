# Design system OVPT — application à Unleash

**Référence d’implémentation adoptée sur consigne de Louis.** Ce document décline l’identité OVPT pour Unleash. Les écarts du produit, le rendu des écrans et les contrastes employés restent à qualifier. Invariants INV-054/056/057 et critères AC-F014-009/010/011/012 et AC-F015-009/013/015. [ADR-03](../architecture/adr/ADR-03-rust-gui.md) fixe la GUI Tauri 2 avec renderer React/TypeScript. [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) fixe la TUI Ratatui/Crossterm.

La base OVPT/AIFM documente le code de `ovpt.ai` version 8 au 29 octobre 2026, référence `4ad18c9d424918f22741f667364150fdae952a01`. L’[annexe historique intégrale](ovpt-aifm-historical-guide.md) conserve les valeurs, composants et sources observés à cette date. Les prescriptions Unleash ci-dessous sont l’adaptation de Louis pour l’écosystème OVPT. Les assets, thèmes, polices effectivement rendues et conventions restent à confirmer dans chaque dépôt.

## Direction

Une interface de travail éditoriale, moderne et sobre. La hiérarchie vient de la typographie, du contraste, de l’espace et des séparations fines. La page mission reçoit un titre court, une phrase d’objectif et une ligne d’état, puis une zone de travail lisible. Les panneaux secondaires s’ouvrent sans effacer la position dans la mission. Les grands titres peuvent avoir une seconde partie grise lorsque sa taille et son contraste le permettent. Les visuels expliquent une information ou une relation utile. Préférer listes, espaces et traits aux cartes répétées. Réserver ombres, icônes et animations aux usages fonctionnels.

Conserver le nom **Unleash** dans l’identité **OVPT**. Utiliser l’asset de marque officiel si sa provenance est établie. Si aucun asset officiel n’est confirmé, conserver la signature textuelle du produit. La devise publique est *Omnia Vincit Progressus Technologicus*. Sa traduction est « Le progrès technologique vient à bout de tout. » Préférer la devise seule pour une signature. Les icônes utiles proviennent d’un ensemble cohérent dont licence et génération sont vérifiées.

La palette, le rythme éditorial et les composants génériques sont communs à l’écosystème. Les chapitres, le lecteur de parcours, le simulateur et les cartes d’architecture de l’annexe décrivent AIFM. Les écrans Unleash découlent de [leurs parcours produit](gui-screen-specifications.md) et de [la parité d’opérations](gui-tui-parity.md).

## Tokens OVPT

| Token de référence | Valeur | Rôle |
| --- | --- | --- |
| `--ovpt-bg` | `#FAFAF8` | Fond clair |
| `--ovpt-text` | `#101010` | Texte principal |
| `--ovpt-ink` | `#111111` | Identité et boutons sombres |
| `--ovpt-nav` | `#161714` | Navigation sombre |
| `--ovpt-accent` | `#E2F471` | Action primaire, sélection importante et progression |
| `--ovpt-accent-hover` | `#EFFFA1` | Survol de l’action citron |
| `--ovpt-accent-text` | `#171B07` | Texte sur le citron |
| `--ovpt-panel` | `#F0F0ED` | Panneaux neutres |
| `--ovpt-border` | `#D4D4CF` | Traits et séparateurs |
| `--ovpt-muted` | `#666662` | Texte secondaire et métadonnées |
| `--ovpt-title-muted` | `#8A8A84` | Seconde partie des grands titres |
| `--ovpt-focus` | `#5555FF` | Focus de référence sur surface claire |

Centraliser ces valeurs dans la feuille de tokens existante. Relier les tokens sémantiques consommés par les composants aux rôles de cette palette au moment de leur adaptation. Préserver le thème sombre et le choix de thème. Adapter ses valeurs à ses surfaces et mesurer les couples réellement rendus dans chaque thème. Conserver des rôles distincts pour succès, attente, erreur et issue incertaine. Ajouter un libellé ou un autre repère perceptible à la couleur. Les résultats illustratifs AIFM restent propres au simulateur.

Vérifier les contrastes selon le référentiel [WCAG 2.2](https://www.w3.org/TR/WCAG22/) AA adopté par [ADR-14](../architecture/adr/ADR-14-local-data-oss.md). Viser au moins 4,5:1 pour le texte normal et 3:1 pour le grand texte et les éléments concernés. Vérifier particulièrement le gris de titre, les contours, l’accent sur clair et le focus sur fond sombre. Utiliser un contour de focus validé sur la surface de navigation sombre.

Utiliser `Arial, Helvetica, sans-serif` pour texte, titres et actions et `"Courier New", monospace` pour les repères et légendes. Choisir des piles locales et vérifier ce qui est effectivement rendu sur les plateformes visées. Le texte courant de référence est de 16 px avec un interligne de 1,55. Respecter les choix de taille et de densité de l’utilisateur. Donner accès aux valeurs essentielles lorsque l’espace manque.

## Composition

La grille utilise des espacements de 4, 8, 12, 16, 24 et 32 unités logiques. Sur les pages de contenu pouvant s’étendre, la largeur maximale de référence est de 1800 px, avec marges `clamp(20px, 3.2vw, 56px)` adaptées à la densité d’un espace de travail. Les panneaux ont des traits de 1 px et des rayons de 4 à 5 px. Une seule action primaire domine chaque zone de décision.

Sur une fenêtre large, prévoir la navigation latérale (220–260 unités), une zone principale flexible et l’inspecteur (320–440 lorsqu’ouvert). En dessous d’environ 1100 unités, utiliser un inspecteur superposé ou une vue dédiée et préserver la lecture des tableaux. À 800 unités de large, maintenir le parcours desktop complet avec une navigation compacte. Dans le renderer aux largeurs étroites, empiler les zones de contenu, préserver les commandes et limiter tout défilement horizontal nécessaire au composant concerné.

Les statuts comportent icône, libellé et description accessible. Une tâche « tests exécutés, revue en attente » reste distincte de « validée ». Les coûts montrent estimé, engagé et confirmé ; une incertitude n’est pas dissimulée sous un total précis.

Le workflow peut être affiché en étapes horizontales courtes ou verticales si nombreuses. Les nœuds parallèles sont groupés ; l’utilisateur peut ouvrir le détail. L’arbre d’agents est une vue, pas la structure de navigation exclusive.

## Interaction

Animations fonctionnelles de 120–180 ms proposées pour ouverture/transition, désactivables avec réduction des mouvements. Aucun clignotement de token ou repositionnement permanent d’une ligne active. Le focus reste stable lors d’une mise à jour événementielle ; un nouveau blocage apparaît sans voler le clavier.

Les actions courantes restent proches de leur cible. Un menu contient les actions secondaires ; les opérations à effets irréversibles montrent ressource et portée. Les demandes de permission ne sont pas universelles : les mandats préautorisés sont respectés, avec un reçu visible.

## Règle d’implémentation et de remise

1. Avant toute modification, inventorier la stack, les feuilles de style et tokens, les composants, les assets de marque, les thèmes, les polices rendues, les routes, les langues et les tests réellement présents. Signaler les écarts avec la référence historique et proposer une adaptation motivée à partir du code courant.
2. Réutiliser les composants et tokens existants. Centraliser les variables nouvelles dans la structure du dépôt. Préserver les opérations, les routes et les langues existantes. Garder des libellés y compris accessibles dans chaque langue prise en charge. Suivre les règles du [renderer GUI](../rules/gui-renderer.md) et du [design TUI](cli-tui-design.md).
3. Préserver sémantique HTML, ordre clavier et focus visible, contraste, étiquettes et préférence `prefers-reduced-motion`. Rendre les états inactif, chargement, vide et erreur compréhensibles là où ils s’appliquent. Pour la TUI, adapter la hiérarchie et les rôles sémantiques aux couleurs disponibles et garder libellés et sélection perceptibles en mode sans couleur.
4. Contrôler le renderer à 320, 390, 768 et 1440 px de large, dans les deux thèmes applicables. Vérifier l’absence de débordement horizontal global, les commandes et les libellés. Vérifier le clavier dans les flux modifiés. Conserver les essais TUI aux dimensions en cellules prévues par F015.
5. Actualiser les tests existants utiles et exécuter les commandes disponibles de lint, typage, tests et build. Remettre les fichiers modifiés, les décisions d’adaptation, les captures lorsqu’elles sont possibles, les commandes avec résultats observés, les échecs et les limites. Une consigne appliquée au dossier reste distincte d’une qualification du rendu.

Consigne réutilisable pour un agent d’interface.

> Lis ce design system et inventorie le code et les assets du dépôt. Applique l’identité OVPT aux écrans du mandat en réutilisant les composants, en centralisant les tokens et en préservant les comportements, les routes et l’accessibilité. Vérifie les dimensions et les états applicables avec les outils du projet. Dans la remise, distingue les observations du code, les adaptations choisies et les résultats réellement vérifiés.

### Écart observé dans Unleash lors de l’adoption

Le renderer centralise déjà ses tokens dans `apps/gui-renderer/src/styles/tokens.css`. Le clair emploie un fond `#F5F6F8`, un accent `#4D46C8`, un corps de 14 px et une pile système démarrant par `-apple-system`. Les panneaux utilisent un rayon de 12 px. Un thème sombre est présent. Le dépôt contient une icône d’application `crates/unleash-gui/icons/icon.png` dont l’inventaire ne confirme pas la provenance comme logo officiel OVPT. La migration visuelle relève des écrans effectivement traités et de leurs tests.

La fenêtre Tauri a une largeur minimale de 800 unités. Les mesures à 320, 390 et 768 px se font dans le renderer jusqu’à une évolution explicite de cette limite. Le document HTML indique le français et certains libellés sont en anglais. L’inventaire de ce renderer n’a pas identifié de sélecteur ou de catalogue de traductions. La TUI actuelle utilise des couleurs terminal dont le cyan et le jaune et dispose de modes `NO_COLOR`, ASCII et lecture linéaire.

## Qualification visuelle

Qualifier le même écran mission sur la pile adoptée (ADR-03), puis le même parcours dans la TUI Ratatui/Crossterm adoptée ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)). Appliquer la politique R2, sans interdire HTML/CSS ni réimplémenter un moteur existant. Utiliser contenus réalistes : trois équipes, une intégration bloquée, un document périmé, un audit long et un coût incertain. Revue notée qualitativement : hiérarchie immédiatement comprise, cohérence des composants, lisibilité des textes longs, absence de surcharge, alignements, états d’erreur et qualité clavier.

Le gate exige aucun défaut critique de lisibilité ou d’accessibilité, une navigation goal→cause→preuve en trois changements de vue au plus et une revue esthétique explicite du porteur. La revue ne doit pas être remplacée par une affirmation « framework natif donc beau ». Les preuves sont captures, parcours et mesures réelles ; elles sont futures.

