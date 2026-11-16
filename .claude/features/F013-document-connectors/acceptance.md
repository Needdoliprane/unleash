# F013 — critères d’acceptation

Tous les critères sont **à exécuter** sur l’implémentation future. Les preuves documentaires actuelles ne sont pas des résultats de recette produit.

## AC-F013-001 — Trois services

Exigence : REQ-DOC-001.

- Étant donné : Trois comptes de test avec dossiers/pages autorisés.
- Lorsque : Explorer, sélectionner et récupérer une ressource de chaque service.
- Alors : Les trois snapshots sont identifiés et inspectables, limites fournisseur explicites.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-002 — Explore

Exigence : REQ-DOC-002.

- Étant donné : Une ressource récemment partagée est absente de l’index distant.
- Lorsque : Rechercher puis ouvrir par référence directe autorisée.
- Alors : Absence de recherche n’est pas présentée comme inexistence ; accès direct essayé selon droits.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-003 — Pin

Exigence : REQ-DOC-003.

- Étant donné : Deux documents au même titre.
- Lorsque : Épingler par identité puis construire le contexte.
- Alors : Le document choisi uniquement est inclus avec provenance.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-004 — Scope

Exigence : REQ-DOC-004.

- Étant donné : Dossier avec sous-dossiers et ressources hors scope.
- Lorsque : Lancer une ingestion avec quotas.
- Alors : Aucune ressource hors scope ; dépassement expliqué et reprenable.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-005 — Watch

Exigence : REQ-DOC-005.

- Étant donné : Machine hors ligne puis modifications/suppression.
- Lorsque : Reprendre le Watch.
- Alors : Curseurs persistés, changements récupérés et index invalidé.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-006 — Révisions

Exigence : REQ-DOC-006.

- Étant donné : Document change pendant une invocation.
- Lorsque : Synchroniser puis lancer une invocation suivante.
- Alors : Ancien manifest inchangé ; nouveau snapshot proposé et provenance préservée.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-007 — OAuth

Exigence : REQ-DOC-007.

- Étant donné : Flux avec mauvais state, redirect ou code verifier.
- Lorsque : Traiter les retours.
- Alors : Aucun token accepté sur mismatch ; secret absent des logs.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-008 — Notion local

Exigence : REQ-DOC-008.

- Étant donné : Token d’intégration utilisateur et pages partagées.
- Lorsque : Connecter puis révoquer le token.
- Alors : Lecture autorisée puis AuthRequired ; binaire sans secret partagé.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-009 — Révocation

Exigence : REQ-DOC-009.

- Étant donné : Copie locale existante puis 403 de droit qualifié, 404 ou tombstone.
- Lorsque : Demander contexte et recherche.
- Alors : Aucun contenu exposé à nouveau ; historique marqué et préservé selon rétention.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-010 — Formats

Exigence : REQ-DOC-010.

- Étant donné : Page contenant blocs et fichier non parsable.
- Lorsque : Prévisualiser et sélectionner.
- Alors : Parties disponibles lisibles, omissions identifiées ; pas faux contenu.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-011 — Quotas

Exigence : REQ-DOC-011.

- Étant donné : 429/529 ou 403 de quota Drive simulé avec reprise permise.
- Lorsque : Lancer plusieurs missions sur le même compte.
- Alors : Backpressure commune, aucun réveil manager par page.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

## AC-F013-012 — MCP

Exigence : REQ-DOC-012.

- Étant donné : Serveur MCP annonce readOnly mais demande effet réseau.
- Lorsque : Exposer son outil dans un contexte.
- Alors : Policy locale décide ; aucune promotion de droit par annotation.
- Limite de la preuve : le test doit préciser environnement, versions et scope ; un résultat de double ne vaut pas un succès de service réel.

