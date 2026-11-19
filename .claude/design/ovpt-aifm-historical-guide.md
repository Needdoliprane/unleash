# Annexe A — Guide historique complet (29/10/2026)

# OVPT / AIFM — Guide du design

Une lecture éditoriale. Un parcours guidé. Des repères qui restent en place.

Ce guide formalise le design du parcours pédagogique ovpt.ai : son identité, ses couleurs, sa typographie, ses composants et ses règles de navigation. Il peut servir de référence à un designer ou de brief à un développeur pour prolonger le site.

Référence vérifiée : base visuelle de la version 8 du site, code `4ad18c9d424918f22741f667364150fdae952a01`, état du 29 octobre 2026. L’addendum final documente les composants d’approfondissement ajoutés ensuite et relus dans le code le même jour ; il ne présume pas de leur numéro de publication. Les valeurs chiffrées viennent du code. Les sections « Règle de continuité » formulent des recommandations pour les évolutions futures ; elles ne décrivent pas des fonctionnalités supplémentaires déjà livrées.

## 1. Direction visuelle

### L’idée

Une présentation qui emprunte ses codes à l’édition : de grands titres, beaucoup d’espace, une composition nette et des textes qui expliquent réellement quelque chose. La sophistication vient de la hiérarchie, du rythme et de la précision, plutôt que d’une accumulation d’effets.

Le site accompagne une découverte. Son action principale est comprendre et avancer dans le parcours. Il ne doit pas prendre la forme d’une page de vente.

| Principe | Traduction visuelle |
| --- | --- |
| Lisibilité | Fond blanc cassé, texte sombre, titres courts et hiérarchie marquée. |
| Orientation | Navigation sombre visible dès l’arrivée, compteurs explicites et destination suivante annoncée. |
| Stabilité | Commandes fixes ; leur position ne dépend pas de la longueur du contenu. |
| Exploration | Onglets numérotés, vues comparables, carte interactive et explications consultables à la demande. |
| Retenue | Palette neutre ; accent citron réservé principalement à l’action et à la vue active. |
| Crédibilité | Sources documentaires, exemples identifiés comme fictifs, limites visibles. |

**Règle de continuité.** Pour enrichir une page, ajouter de la matière pédagogique avant d’ajouter de la décoration : mécanisme, exemple, décision, conséquence ou référence.

## 2. Identité AIFM / OVPT

### Le mot-symbole AIFM

L’identité principale est typographique : AIFM, en sans sérif très grasse, avec un espacement serré. Le mot-symbole est volontairement surdimensionné sur l’accueil. Il donne une signature forte sans nécessiter d’illustration centrale.

La forme orbitale antérieure n’est plus utilisée dans l’accueil. La composition actuelle associe le grand mot-symbole AIFM à une signature OVPT plus discrète, séparée par un trait vertical.

| Élément | Traitement observé |
| --- | --- |
| Marque dans le header | Arial / Helvetica, 29px, graisse 900, lettres -1.8px ; 28px sur mobile. |
| Mot-symbole d’accueil | Graisse 900, interligne 0.8, lettres -0.092em. |
| Taille du mot-symbole | `clamp(230px, 33vw, 575px)` sur grand écran ; 33vw sous 700px. |
| Signature OVPT d’accueil | Sans sérif grasse, trait vertical ; petit complément en monospace sur grand écran. |
| Signature de fin de page | Fond noir, grand OVPT blanc et devise à côté ; composition verticale sur mobile. |

### La devise

Omnia Vincit Progressus Technologicus

Le progrès technologique vient à bout de tout.

La devise s’utilise en signature, avec une taille plus modeste que le nom de la marque. Elle accompagne l’identité sans concurrencer le contenu pédagogique.

**Règle de continuité.** Préserver les lettres et l’espace autour d’elles. Éviter les effets 3D, les formes orbitales, les pictogrammes évoquant une autre marque et les détails décoratifs qui rendent la signature ambiguë.

## 3. Palette

### Couleurs principales

| Nom d’usage | Code hexadécimal | Usage observé |
| --- | --- | --- |
| Blanc cassé | `#FAFAF8` | Fond principal et surfaces claires. |
| Noir de lecture | `#101010` | Texte principal. |
| Noir d’identité | `#111111` | Plusieurs boutons, diagramme d’accueil, signature et composants sélectionnés de la carte. |
| Noir de navigation | `#161714` | Navigation fixe du parcours. |
| Citron | `#E2F471` | Bouton Suivant, onglet interne sélectionné et progression active. |
| Gris de panneau | `#F0F0ED` | Fond des blocs interactifs. |
| Gris de trait | `#D4D4CF` | Bordures et séparateurs. |
| Gris de texte | `#666662` | Texte secondaire et métadonnées. |
| Gris de titre | `#8A8A84` | Seconde partie de nombreux grands titres. |

### Couleurs de support

| Code | Fonction |
| --- | --- |
| `#E4E7DA` | Bandeau des exemples interactifs. |
| `#EFFFA1` | Survol du bouton principal citron. |
| `#171B07` | Texte sur le bouton citron et sur l’onglet sélectionné. |
| `#C5C7B8` | Compteur de chapitre sur fond de navigation sombre. |
| `#EFEFEB` | Panneau de la carte d’architecture. |
| `#E1E1DA` | Composant voisin de la sélection dans la carte. |
| `#5555FF` | Contour de focus général, hors commandes du parcours. |

### Couleurs des résultats illustratifs

Ces couleurs servent aux verdicts du simulateur. Elles ne doivent pas devenir des couleurs de marque secondaires.

| État visuel | Fond | Bordure |
| --- | --- | --- |
| Résultat favorable sous conditions | `#E4EBE5` | `#86998D` |
| Refus | `#F3E4DF` | `#A57B6B` |
| Confirmation / obligations | `#F2ECDF` | `#A39575` |

**Règle de continuité.** Le citron attire le regard sur la prochaine action ou sur la sélection courante. Éviter de l’étendre à tous les titres, à de grandes surfaces de contenu ou à des décorations : sa valeur vient de sa rareté.

## 4. Typographie

### Familles

```css
/* Texte, titres, boutons et identité */
font-family: Arial, Helvetica, sans-serif;

/* Repères, compteurs, sources et légendes */
font-family: "Courier New", monospace;
```

Ces piles utilisent les polices disponibles sur l’appareil. Leur rendu précis dépend donc de la police effectivement sélectionnée par le navigateur.

### Échelle observée

Les tailles ci-dessous sont des pixels CSS, pas des points d’impression.

| Usage | Grand écran | Mobile / adaptation | Traitement |
| --- | --- | --- | --- |
| Titre de chapitre | `clamp(36px, 4.2vw, 62px)` | 36px sous 700px | Graisse 500, interligne 1, lettres -0.06em sur grand écran. |
| Titre de l’accueil | `clamp(32px, 3.6vw, 56px)` | 38px sous 700px, 34px sous 400px | Interligne 1.02, lettres -0.055em. |
| Titre de la carte d’architecture | `clamp(52px, 7vw, 100px)` | 65px sous 900px, 54px sous 520px | Interligne 0.95. |
| Titre de section | 25px | Ajustements locaux selon le composant | Graisse 500, interligne 1.15, lettres -0.8px. |
| Sous-titre | 18px | Hérite du composant | Graisse 500, interligne 1.3. |
| Texte courant | 16px | 16px par défaut | Interligne 1.55. |
| Introduction des chapitres | 16px | 16px | Largeur maximale 880px. |
| Repères et petites légendes | Souvent 11px | Souvent 10px | Courier New, interligne 1.4 à 1.5. |
| Action principale du parcours | 16px, graisse 700 | 14px sous 1000px, 13px sous 700px, 11px sous 400px | Ligne de destination plus petite en dessous. |

### Rythme rédactionnel

Un grand titre exprime une idée complète, souvent sur deux lignes. La première partie est sombre ; la seconde peut passer en gris. Le gris n’est pas une indication d’inactivité : il crée un second niveau de lecture.

Les petites capitales ou la monospace servent aux repères. Les paragraphes explicatifs gardent une casse normale. Les libellés des commandes doivent rester directs : « Commencer le parcours », « Étape suivante », « Vue suivante », « Chapitre suivant ».

**Règle de continuité.** Ne pas mettre de longs paragraphes en capitales ou en monospace. Réserver l’espacement très serré aux grands titres et aux mots-symboles.

## 5. Grille, espace et surfaces

| Paramètre | Valeur observée |
| --- | --- |
| Largeur maximale du conteneur | 1800px |
| Marge latérale fluide | `clamp(20px, 3.2vw, 56px)` |
| Bordure standard | 1px |
| Rayon des principaux panneaux | 4px à 5px |
| Rayon du bouton Suivant | 7px |
| Rayon du bouton d’entrée dans le parcours | 6px |
| Boutons secondaires et sélecteur de langue | Forme pilule : 999px |
| Espace au-dessus d’un bloc interactif de chapitre | 27px |
| Grille explicative interne | Deux colonnes sur grand écran ; une colonne sous 700px. |

La page alterne trois types de surfaces :

1. Le fond clair, pour les titres, la narration et les références.
2. Le panneau neutre, pour une interaction ou un exemple délimité.
3. Le fond sombre, pour les commandes principales et la signature.

**Règle de continuité.** Utiliser l’espace et les traits fins pour structurer la page. Éviter de transformer chaque paragraphe en carte : les surfaces doivent avoir une fonction identifiable.

## 6. Navigation du parcours

### Deux niveaux distincts

Le parcours sépare explicitement :

- Le chapitre : « Chapitre 3 sur 10 » et son nom.
- La position interne : « Étape 2 sur 5 » pour un processus, ou « Vue 2 sur 3 » pour des éléments à comparer.

Le bouton principal avance d’abord à l’intérieur du chapitre. Après la dernière vue, il devient « Chapitre suivant ». La ligne secondaire annonce la destination. Le dernier chapitre propose « Recommencer ».

Les dix repères numérotés restent des raccourcis directs vers les chapitres. Leur état « avant la position actuelle » ne constitue pas une preuve de lecture ou de validation.

### Position et dimensions

Le header et le bloc de navigation sont fixes en haut de la fenêtre. Le contenu réserve leur hauteur avec un padding-top équivalent, afin de ne pas débuter derrière eux.

| Situation | Header | Navigation | Suivant |
| --- | --- | --- | --- |
| Largeur supérieure à 1000px | 100px | 166px | 280 × 65px |
| Largeur de 701px à 1000px | 90px | 166px | 250 × 65px |
| Largeur ≤ 700px | 120px | 194px | Largeur disponible, hauteur 58px |
| Hauteur ≤ 600px, largeur > 700px | 64px | 96px | Format grand écran ; bande des dix chapitres masquée. |

### États visuels

| Élément | État courant |
| --- | --- |
| Bouton Suivant | Fond citron, texte sombre et flèche vers la droite. |
| Bouton Précédent | Fond sombre transparent, contour visible, texte clair. |
| Précédent indisponible | Opacité 0.35 ; bouton désactivé. |
| Chapitre courant | Tuile claire, texte sombre. |
| Autre chapitre | Tuile sombre avec contour discret. |
| Étape interne courante | Petit badge citron et segment de progression citron. |

### Entrée dans le parcours

Le bouton « Commencer le parcours » est présent dans le bloc de navigation et dans l’introduction de l’accueil. La version dans le contenu est noire, avec une hauteur minimale de 62px et une flèche. Elle rend l’action évidente sans demander au lecteur de chercher un footer.

**Règle de continuité.** Ne pas déplacer Suivant quand le texte change, quand une vue devient plus longue ou quand le visiteur arrive à la fin d’un chapitre. Conserver le bouton, remplacer son libellé et annoncer la destination.

## 7. Anatomie d’un chapitre

### Composition observée

| Niveau | Rôle |
| --- | --- |
| Repère supérieur | Situer le lecteur dans le parcours. |
| Petit intitulé de chapitre | Nommer le thème et son numéro. |
| Grand titre | Exprimer le point à comprendre. |
| Introduction | Expliquer pourquoi le sujet compte et ce que l’on va explorer. |
| Corps du chapitre | Cartes explicatives, panneau interactif ou simulateur selon le sujet. |
| Complément dépliable | Présenter une nuance ou un cas particulier sans interrompre le fil principal. |
| Référence | Indiquer la documentation source et le périmètre de l’explication. |

### Panneau interactif

Le panneau combine un en-tête, un compteur local, des onglets numérotés et la vue sélectionnée. Le contenu interne place généralement une explication à gauche et un objet ou résultat illustratif à droite.

| Composant | Valeur / comportement |
| --- | --- |
| Fond du panneau | `#F0F0ED` |
| En-tête du panneau | Fond `#E4E7DA`, texte `#303526` |
| Onglet inactif | Fond `#FAFAF8`, bordure neutre |
| Onglet actif | Fond `#E2F471`, bordure `#849642`, texte `#171B07` |
| Hauteur minimale d’un onglet | 54px ; 50px sur mobile |
| Rayon d’un onglet | 5px |
| Organisation des onglets | Grille sur grand écran ; défilement horizontal sur mobile |
| Révélation de la sélection | La rangée défile horizontalement pour garder l’onglet actif visible. |

**Règle de continuité.** Pour ajouter de la profondeur, privilégier une réponse à chacune de ces questions : « Que se passe-t-il ? », « Qui décide ? », « Qu’est-ce qui change ? », « Que se passe-t-il si cela échoue ? ». Distinguer les exemples illustratifs des affirmations vérifiées dans la documentation.

## 8. Carte d’architecture et parcours interactifs

La carte reprend les mêmes familles typographiques, les panneaux clairs et les traits sobres. Sa sélection utilise le noir plutôt que le citron du lecteur principal.

| Composant | Traitement observé |
| --- | --- |
| Fond de la carte | `#EFEFEB`, bordure `#D5D5CF` |
| Composant normal | Fond `#FAFAF8`, bordure `#C3C3BB`, rayon 4px |
| Composant sélectionné | Fond `#111111`, texte blanc, contour renforcé à l’intérieur |
| Composant voisin | Fond `#E1E1DA` |
| Composant atténué | Opacité 0.68 |
| Liaison | Trait noir de 1.4px |
| Liaison active | Trait discontinu animé ; animation supprimée en mode de mouvement réduit |
| Panneau de détail | Colonne séparée, hauteur réservée et contenu défilant à l’intérieur |

La carte passe de quatre colonnes à deux sous 520px. Sous 900px, le panneau de détail passe sous la carte. Les commandes de lecture restent dans une zone propre, séparée de l’explication.

**Règle de continuité.** La mise en évidence doit expliquer une relation. Garder les autres composants suffisamment visibles pour préserver le contexte ; éviter les déplacements de nœuds au clic.

## 9. Mobile, langues et accessibilité

### Mobile

Le mobile conserve les commandes essentielles et réduit la densité de la navigation secondaire. La bande des chapitres affiche des numéros de 44 × 44px, avec des libellés accessibles complets. Les titres et les panneaux reviennent à une colonne ; les onglets internes peuvent défiler horizontalement.

**Règle de continuité.** Tester au minimum à 320px et à 390px de largeur, ainsi qu’en paysage. Aucun contenu principal ne doit imposer un défilement horizontal de la page entière.

### Français et anglais

Le sélecteur Français / English reste dans le header. La langue active est visible par un état clair dans une pilule sombre. Le code conserve le choix dans le stockage local du navigateur ; le français est utilisé par défaut lorsqu’aucun choix anglais n’est enregistré.

**Règle de continuité.** Traduire le contenu, les commandes, les compteurs et les libellés accessibles. Les variations de longueur ne doivent ni déplacer Suivant ni masquer la destination.

### Mécanismes présents

- Lien d’accès rapide au contenu.
- Focus visible : contour général bleu `#5555FF`, contour blanc dans le bloc de navigation.
- Chapitre courant annoncé avec `aria-current="step"`.
- Boutons des vues internes avec `aria-pressed`.
- Statut de navigation avec `aria-live="polite"`.
- Progression globale avec rôle `progressbar` et valeur courante.
- Navigation au clavier avec les flèches gauche et droite, hors contrôles que le code exclut de ce raccourci.
- Prise en compte de `prefers-reduced-motion`.

Ces mécanismes sont confirmés dans le code. Ils ne constituent pas, à eux seuls, une déclaration de conformité WCAG ou un audit d’accessibilité complet.

## 10. Extrait CSS de référence

Cet extrait reprend les valeurs structurantes du style publié. Il sert de base de reprise ; il ne remplace pas les feuilles complètes, qui contiennent aussi les composants et les états.

```css
:root {
  color-scheme: light;
  --bg: #fafaf8;
  --panel: #f0f0ed;
  --line: #d4d4cf;
  --muted: #666662;
  --text: #101010;
  --gutter: clamp(20px, 3.2vw, 56px);
}

body {
  background: var(--bg);
  color: var(--text);
  font: 16px / 1.55 Arial, Helvetica, sans-serif;
}

body:has(.journey-dock) {
  --accent: #e2f471;
  --reader-header: 100px;
  --reader-height: 166px;
  padding-top: calc(var(--reader-header) + var(--reader-height));
  padding-bottom: 0;
}

body:has(.journey-dock) > header {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: var(--reader-header);
}

body:has(.journey-dock) > .journey-dock {
  position: fixed;
  top: var(--reader-header);
  left: 0;
  right: 0;
  height: var(--reader-height);
  background: #161714;
  color: #fff;
}

@media (max-width: 1000px) and (min-width: 701px) {
  body:has(.journey-dock) {
    --reader-header: 90px;
  }
}

@media (max-width: 700px) {
  body:has(.journey-dock) {
    --reader-header: 120px;
    --reader-height: 194px;
  }
}

@media (max-height: 600px) and (min-width: 701px) {
  body:has(.journey-dock) {
    --reader-header: 64px;
    --reader-height: 96px;
  }

  .journey-dock #journey-steps {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  * {
    animation: none !important;
    transition: none !important;
  }
}
```

## 11. Vérification d’une évolution

Cette grille est une recommandation pour les futures modifications.

| Test | Critère d’acceptation |
| --- | --- |
| Arrivée sur l’accueil | Un bouton d’entrée dans le parcours est immédiatement identifiable. |
| Avancement dans un chapitre | Le compteur, la vue active et la destination suivante restent cohérents. |
| Dernière vue | Le bouton annonce explicitement le passage au chapitre suivant. |
| Commandes et défilement | Les coordonnées du bouton Suivant ne changent pas pendant l’avancement. |
| Contenu long | Les explications restent accessibles sans recouvrir les commandes. |
| Mobile | Pas de débordement horizontal global ; onglet courant visible. |
| Français / anglais | Aucun libellé coupé de façon gênante ; même navigation dans les deux langues. |
| Clavier | Focus visible, ordre de lecture compréhensible et accès aux commandes. |
| Mouvement réduit | Les animations n’empêchent pas la compréhension et sont désactivables par la préférence système. |
| Erreurs et limites | Un résultat incomplet, un refus ou une limite documentaire est nommé clairement dans le contenu. |
| Exactitude éditoriale | Toute affirmation technique nouvelle est reliée à une source vérifiée. |

## 12. Sources et périmètre

Les spécifications de ce guide ont été relues dans les fichiers suivants, au commit de référence :

| Fichier du site | Ce qu’il confirme |
| --- | --- |
| `dist/style.css` | Palette, typographie, surfaces, navigation fixe et adaptations d’écran. |
| `dist/architecture.css` | Carte, composants, sélection et panneau de détail. |
| `dist/index.html` | Structure du parcours, header, commandes, attributs accessibles et signature. |
| `dist/app.js` | Chapitres, vues internes, avancement, compteurs et repères actifs. |
| `dist/language.js` | Sélecteur de langue et conservation du choix. |

Confirmé : valeurs et comportements présents dans cette version du code.

Recommandé : règles de continuité et grille de vérification.

À revérifier lors d’une reprise : cohérence avec la version alors publiée, nouveaux contenus, contrastes de toute nouvelle combinaison de couleurs et comportement sur les appareils réellement ciblés.

Ce guide décrit l’interface. Il ne valide pas les affirmations techniques sur AIFM, qui doivent rester vérifiées dans sa documentation d’architecture.

## 13. Addendum — Donner plus de matière aux chapitres

Observation confirmée dans le code ajouté : `dist/chapter-depth.css` et `dist/chapter-depth.js`, relus le 29 octobre 2026. Ce complément conserve la palette, la typographie et les commandes du parcours ; il ajoute une seconde profondeur de lecture à l’intérieur des chapitres.

### Composition de l’approfondissement

| Composant | Rôle et style observés |
| --- | --- |
| Ouverture « Le mécanisme, en détail » | Séparation haute par un trait noir ; marge supérieure 56px, retrait intérieur 36px. Titre fluide de 30 à 48px, avec introduction à côté. |
| Cartes de mécanisme | Trois colonnes, écart 26px. Fond `#F0F0ED`, bordure `#D4D4CF`, rayon 5px, espace intérieur 24px. Numéro en monospace, titre 22px, texte 15px avec interligne 1.65. |
| Mise en situation | Panneau sombre `#171914`, rayon 6px, espace intérieur 30px. Deux colonnes : contexte à gauche, choix et résultat à droite. L’exemple est explicitement nommé fictif. |
| Choix du cas | Boutons d’au moins 45px de haut, rayon 5px. La sélection reprend le citron `#E2F471` ; les autres options restent sombres. |
| Résultat du cas | Carte claire `#FAFAF8`, rayon 4px, espace intérieur 25px, hauteur minimale 170px. Le résultat change sans quitter le chapitre. |
| Synthèse | Bloc « À retenir » avec texte 18px, séparé par un trait discret. |
| Références et portée | Complément dépliable ; libellé en monospace 11px, sources en 12px. |

### Explication au niveau d’une étape

Une vue peut être complétée par un bloc en trois colonnes : À l’entrée / Le contrôle clé / À la sortie. Chaque colonne présente un libellé monospace de 10px et un texte de 14px. Un trait vertical `#B7C474` rattache visuellement le contenu au repère.

Le point de vigilance est isolé dans un panneau `#E6E9DC`, avec un titre court et une explication. Les sources de cette étape restent disponibles dans un complément dépliable distinct.

### Adaptation et interaction

Sous 700px, le titre, les cartes, la mise en situation, les choix et les explications d’étape passent en une colonne. Les choix mesurent au moins 44px de haut. La sélection d’un cas utilise `aria-pressed` et indique le résultat contrôlé avec `aria-controls`. Le résultat est annoncé avec un statut `aria-live="polite"` et `aria-atomic="true"`.

**Règle de continuité.** Une nouvelle carte doit apporter une explication distincte. Un cas doit montrer une conséquence compréhensible. Réserver le panneau sombre à la mise en situation pour conserver son rôle visuel ; maintenir les références près du contenu qu’elles justifient. Les styles documentés ici ne constituent pas une validation des affirmations techniques qu’ils présentent.
