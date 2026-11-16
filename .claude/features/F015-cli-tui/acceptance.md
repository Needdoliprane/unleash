# F015 — acceptation

État : critères spécifiés, non exécutés sur le produit. Chaque preuve identifie build, versions, environnement terminal/OS, configuration, identité et limites. Les suffixes 001 à 008 sont conservés ; 009 à 016 étendent la recette (R2), puis 017 (R3–R5).

## AC-F015-001 — Parité complète des dix espaces

Exigence : REQ-CLI-001.

- Étant donné : le catalogue de la [matrice](../../design/gui-tui-parity.md), un backend réel et les mêmes données, révisions, identité et mandats.
- Lorsque : parcourir chaque groupe d’opérations dans GUI, TUI et commandes non interactives applicables, avec succès puis refus/conflit.
- Alors : mêmes commandes métier, effets, reçus, erreurs, données et preuves ; chaque opération et chaque espace possède un parcours TUI complet. Aucun renvoi obligatoire vers la GUI, aucun périmètre limité aux fonctions « essentielles ».
- Preuve attendue : EV-F015-001 avec résultat par ligne PAR, parcours terminaux, comparaison de projections/effets et liste d’écarts vide pour les capacités requises. Un mock seul ne valide pas cette parité.

## AC-F015-002 — JSON et flux

Exigence : REQ-CLI-002.

- Étant donné : sortie scriptable, texte multiligne, gros résultat et erreur fournisseur.
- Lorsque : exécuter lecture, mutation et watch dans un pipeline.
- Alors : stdout parsable, version/IDs/état/pagination présents, aucun ANSI ni diagnostic humain ; stderr et code de sortie cohérents.
- Preuve attendue : EV-F015-002, captures stdout/stderr et assertions de schéma.

## AC-F015-003 — Reçus et issue incertaine

Exigence : REQ-CLI-003.

- Étant donné : mutation acceptée avec coupure avant ou après commit serveur.
- Lorsque : réutiliser command_id puis inspecter reçu/opération depuis un autre client.
- Alors : un seul effet, reçu initial retrouvé, état accepted distinct de completed ; même clé avec payload différent refusée.
- Preuve attendue : EV-F015-003, journal causal, reçus et état durable.

## AC-F015-004 — Versions et autorité

Exigence : REQ-CLI-004.

- Étant donné : protocole majeur incompatible, objet modifié ailleurs et acteur sans permission.
- Lorsque : envoyer l’action depuis CLI et TUI puis le même cas en GUI.
- Alors : refus commun explicite, aucun effet ni accès direct à la base ; aucun droit supplémentaire accordé par un flag ou raccourci.
- Preuve attendue : EV-F015-004, réponses comparées, traces d’autorisation et absence de mutation.

## AC-F015-005 — Quitter et détacher

Exigence : REQ-CLI-005.

- Étant donné : mission et processus actifs avec client connecté ou PTY attaché.
- Lorsque : quitter la TUI, fermer le terminal ou détacher, puis reconnecter.
- Alors : daemon et travail continuent selon leur état réel ; l’attachement est libéré. Seule une commande métier explicite peut demander arrêt/interruption.
- Preuve attendue : EV-F015-005, états avant/après, identités processus et absence de commande d’arrêt implicite.

## AC-F015-006 — Clavier, saisie et focus

Exigence : REQ-CLI-006.

- Étant donné : souris désactivée, fenêtre redimensionnable, conversation et formulaire multiligne.
- Lorsque : parcourir les dix espaces, rechercher, envoyer un message, corriger un champ et revenir à la sélection.
- Alors : toutes les actions sont atteignables au clavier ; focus visible, sélection/brouillon préservés, saisie isolée des raccourcis globaux.
- Preuve attendue : EV-F015-006, parcours clavier et captures avant/après resize, sans dépendance à un éditeur externe.

## AC-F015-007 — Logs, PTY et génération

Exigence : REQ-CLI-007.

- Étant donné : utilisateur lecteur, utilisateur écrivain, PTY actif et PID réutilisé.
- Lorsque : lire/rechercher les logs, tenter d’écrire, obtenir/perdre le lease et demander arrêt.
- Alors : séquences hostiles neutralisées dans les vues texte, refus d’écriture du lecteur, révocation appliquée, génération exacte ciblée, mode PTY et sortie clairement séparés du chat.
- Preuve attendue : EV-F015-007, canaris ANSI/OSC, traces des droits/leases, générations et captures.

## AC-F015-008 — Erreurs et automatisation

Exigence : REQ-CLI-008.

- Étant donné : daemon arrêté, refus de politique, quota, stdin/pipe fermé et absence de TTY.
- Lorsque : exécuter les commandes avec/sans wait et demander tui sans terminal interactif.
- Alors : codes documentés, erreur actionnable, aucun prompt masqué ni raw mode sur pipe ; timeout client sans annulation métier implicite.
- Preuve attendue : EV-F015-008, matrice d’erreurs, codes, sorties et état métier final.

## AC-F015-009 — Application terminal soignée

Exigence : REQ-CLI-009.

- Étant donné : les dix espaces avec jeux vides, remplis, partiels et en erreur.
- Lorsque : utiliser navigation, fil d’Ariane, arborescence/tableaux, inspecteur, palette, formulaires et aide.
- Alors : mêmes actions via menus/palette, contexte conservé, composants cohérents, aucun texte/action essentielle tronqué sans accès au détail ; états compréhensibles sans consulter les logs bruts.
- Preuve attendue : EV-F015-009, captures de référence 120×40 et 80×24, parcours par espace, revue visuelle et ergonomique.

## AC-F015-010 — Éditeurs intégrés complets

Exigence : REQ-CLI-010.

- Étant donné : un skill long, deux versions de rôle et un workflow avec branches, boucle bornée, contrats et bindings.
- Lorsque : créer/éditer dans la TUI, rechercher, annuler/rétablir, comparer, valider, simuler/évaluer si applicable, publier puis déprécier les versions de skill/rôle selon mandat.
- Alors : contenu intégral accessible et modifiable, diagnostics localisés, même AST/versions/contrats que GUI, draft invalide conservable mais non exécutable, conflit de publication explicite ; aucune application graphique ni éditeur externe requis.
- Preuve attendue : EV-F015-010, documents/AST avant/après, diffs, diagnostics, évaluations/simulations marquées et reçus.

## AC-F015-011 — Contexte, provenance et previews

Exigence : REQ-CLI-011.

- Étant donné : contexte à plusieurs appels, compaction, document externe, source non intégrée, contenu supprimé et média sans rendu terminal.
- Lorsque : ouvrir les manifests, naviguer les sources, comparer, prévisualiser et exporter un contenu autorisé.
- Alors : autorisé/sélectionné/transmis et réception inconnue sont distincts ; hash/version, provenance, exclusions et rétention restent visibles ; preview adaptée sans information métier perdue, source/export accessibles, aucun secret exposé.
- Preuve attendue : EV-F015-011, manifests comparés, exports, droits et captures ; une URL seule ne remplace pas les informations inspectables du produit.

## AC-F015-012 — Settings, fournisseurs et authentification

Exigence : REQ-CLI-012.

- Étant donné : nouveau profil, fournisseurs modèles, Google/OneDrive et parcours Notion qualifié, scopes et AI-Guard.
- Lorsque : configurer/tester, gérer secret référencé, initier le consentement depuis la TUI, refuser/expirer/reprendre, ajuster limites et révoquer selon mandat.
- Alors : parcours réalisable sans GUI Unleash ; état réel du flux, erreurs et remédiation visibles ; secret absent d’argv/logs/captures ; mêmes refus et limites backend.
- Preuve attendue : EV-F015-012, parcours services réels autorisés, traces expurgées et vérification du coffre. Les doubles valident les erreurs de présentation, pas la compatibilité fournisseur.

## AC-F015-013 — Capacités et accessibilité

Exigence : REQ-CLI-013.

- Étant donné : terminaux qualifiés, couleur/souris désactivées, glyphes limités, fenêtre étroite et lecteur d’écran.
- Lorsque : naviguer, lire, éditer et configurer dans les modes normal, compact et linéaire.
- Alors : statuts/focus/erreurs ne reposent pas sur couleur seule ; actions atteignables et contenu conservé ; dégradation annoncée, aucune prise de contrôle brut sur non-TTY ; compatibilité d’assistance consignée avec ses limites.
- Preuve attendue : EV-F015-013, matrice terminal/capacité, parcours clavier/assistance et contenu exporté inchangé.

## AC-F015-014 — Restauration terminal

Exigence : REQ-CLI-014.

- Étant donné : terminal dans un état initial mesuré et modes optionnels activés.
- Lorsque : sortie normale, erreur d’initialisation partielle, panique, SIGINT/SIGTERM interceptables, suspension pilotée (Ctrl+Z) et reprise, SIGSTOP/SIGCONT externes ou transfert à l’éditeur.
- Alors : curseur, raw mode, alternate screen, souris/focus/paste restaurés ; reprise redessine proprement ; erreur de nettoyage observable, sans masquer l’erreur initiale. La frontière SIGKILL/SIGSTOP externe/perte terminal/arrêt OS est documentée ; après SIGSTOP, seule la réinitialisation au SIGCONT est attendue.
- Preuve attendue : EV-F015-014, état terminal avant/après, session shell utilisable et captures réelles, sans confusion avec arrêt du daemon.

## AC-F015-015 — Ressources et réactivité

Exigence : REQ-CLI-015.

- Étant donné : charge et budgets définis dans T-F015-015 sur une machine de référence identifiée.
- Lorsque : filtrer/déplier, ouvrir un gros artefact, suivre des logs continus et redimensionner sous rafale.
- Alors : seules les pages/sections nécessaires sont chargées, buffers et files restent bornés, commande et clavier répondent dans les budgets ; backpressure et couverture manquante visibles.
- Preuve attendue : EV-F015-015, mesures p95/mémoire/volumes, instrumentation des pages et absence de perte silencieuse d’événements métier.

## AC-F015-016 — Reconnexion et reprise des interactions

Exigence : REQ-CLI-016.

- Étant donné : message en vol, brouillon local, cursor expiré ou stream_id remplacé.
- Lorsque : perdre la connexion, modifier l’objet via GUI puis reconnecter la TUI.
- Alors : état périmé visible, reçu réconcilié sans doublon, nouveau snapshot et événements ordonnés, brouillon conservé avec conflit explicite ; aucune soumission automatique.
- Preuve attendue : EV-F015-016, curseurs, journal causal, captures et états avant/après reprise.


## AC-F015-017 — Parcours guidé et automatisations au terminal

Exigence : REQ-CLI-017.

- Étant donné : Un utilisateur ouvre une TUI neuve sans agents ni repository.
- Lorsque : Préparer puis exécuter une procédure recommandée, éditer ses paramètres, inspecter résultats et modèle local ; refaire en CLI avec un document de setup.
- Alors : Toutes les fonctions GUI existent sans ouvrir la GUI ; l’interface distingue données/procédure et coûts, fournit reprise des drafts et ne transforme pas un mode sans modèle en supervision implicite.
- Preuve attendue : EV-F015-017, environnement, versions, observations et limites. Statut : non exécuté.
