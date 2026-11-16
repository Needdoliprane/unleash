# Design system proposé — OVPT Unleash

Statut : direction visuelle à qualifier, pas maquette déjà approuvée. Invariants INV-054/056/057 ; acceptation AC-F014-009/010/011/012 et AC-F015-009/013/015. La pile adoptée ([ADR-03](../architecture/adr/ADR-03-rust-gui.md) : Tauri 2, renderer React/TypeScript) doit démontrer cette expérience. Kivy et Tauri sont les références corrigées par Louis ; elles ne remplacent pas l’évaluation de l’expérience réelle.

## Direction

Une interface desktop de travail soignée, calme et précise. Fond légèrement teinté, surfaces peu nombreuses, bordures discrètes, typographie nette et accents réservés aux actions utiles. La page mission reçoit un titre court, une phrase d’objectif et une ligne d’état, puis une zone de travail lisible. Les panneaux secondaires s’ouvrent sans effacer la position dans la mission.

Le nom OVPT Unleash est traité typographiquement dans l’en-tête. Aucun logo ni marque graphique n’est inventé comme identité approuvée. Les icônes proviennent d’un ensemble cohérent dont licence et génération sont vérifiées.

## Tokens proposés

| Token | Clair | Sombre | Usage |
| --- | --- | --- | --- |
| background | #F5F6F8 | #11151D | Fond général |
| surface | #FFFFFF | #1A202B | Panneaux |
| surface_subtle | #ECEFF4 | #222B39 | Sélection secondaire |
| text_primary | #182131 | #EDF1F7 | Titres et contenu |
| text_secondary | #4E5A6E | #AAB7CB | Métadonnées |
| border | #D5DBE5 | #354156 | Séparateurs |
| accent | #4D46C8 | #AAA5FF | Action principale / focus à valider |
| success | #216747 | #7ED7AE | Résultat établi |
| attention | #805400 | #EDC66B | Décision ou attente |
| danger | #A62D41 | #FFA0AD | Échec/risque |
| unknown | #5D5575 | #C2B6DC | Issue incertaine |

Les couleurs sont des propositions de style ; mesurer leurs contrastes réels sur chaque couple utilisé. [WCAG 2.2](https://www.w3.org/TR/WCAG22/) niveau AA, consulté le 16/11/2026, est le référentiel de recette du contenu GUI ([ADR-14](../architecture/adr/ADR-14-local-data-oss.md), amendement R7) : 4,5:1 pour texte normal et 3:1 pour grand texte/éléments concernés. Cela ne constitue pas une certification WCAG d’une application native.

Échelle d’espacement proposée : 4, 8, 12, 16, 24, 32 unités logiques. Rayon 8 pour contrôles, 12 pour panneaux, ombres légères seulement sur overlays. Éviter un panneau imbriqué pour chaque donnée : listes et séparateurs suffisent souvent.

Police système sans serif pour texte et interface, police monospace système pour code/IDs. Corps 14–15 unités, libellés 12–13, titre de mission 24–28. Les tailles suivent le réglage utilisateur ; aucun contrôle ne tronque une valeur indispensable sans détail accessible.

## Composition

À largeur large : navigation latérale 220–260 unités, zone principale flexible, inspecteur 320–440 lorsqu’ouvert. En dessous d’environ 1100 unités, l’inspecteur devient panneau superposé ou vue dédiée ; il ne compresse pas les tableaux jusqu’à illisibilité. À 800 unités de large, navigation compacte et parcours complet restent possibles.

Les statuts comportent icône, libellé et description accessible. Une tâche « tests exécutés, revue en attente » reste distincte de « validée ». Les coûts montrent estimé, engagé et confirmé ; une incertitude n’est pas dissimulée sous un total précis.

Le workflow peut être affiché en étapes horizontales courtes ou verticales si nombreuses. Les nœuds parallèles sont groupés ; l’utilisateur peut ouvrir le détail. L’arbre d’agents est une vue, pas la structure de navigation exclusive.

## Interaction

Animations fonctionnelles de 120–180 ms proposées pour ouverture/transition, désactivables avec réduction des mouvements. Aucun clignotement de token ou repositionnement permanent d’une ligne active. Le focus reste stable lors d’une mise à jour événementielle ; un nouveau blocage apparaît sans voler le clavier.

Les actions courantes restent proches de leur cible. Un menu contient les actions secondaires ; les opérations à effets irréversibles montrent ressource et portée. Les demandes de permission ne sont pas universelles : les mandats préautorisés sont respectés, avec un reçu visible.

## Qualification visuelle

Qualifier le même écran mission sur la pile adoptée (ADR-03), puis le même parcours dans la TUI Ratatui/Crossterm adoptée ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)). Appliquer la politique R2, sans interdire HTML/CSS ni réimplémenter un moteur existant. Utiliser contenus réalistes : trois équipes, une intégration bloquée, un document périmé, un audit long et un coût incertain. Revue notée qualitativement : hiérarchie immédiatement comprise, cohérence des composants, lisibilité des textes longs, absence de surcharge, alignements, états d’erreur et qualité clavier.

Le gate exige aucun défaut critique de lisibilité ou d’accessibilité, une navigation goal→cause→preuve en trois changements de vue au plus et une revue esthétique explicite du porteur. La revue ne doit pas être remplacée par une affirmation « framework natif donc beau ». Les preuves sont captures, parcours et mesures réelles ; elles sont futures.

