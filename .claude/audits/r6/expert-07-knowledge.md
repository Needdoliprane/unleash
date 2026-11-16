# R6 — Expert 07 — Knowledge & Code Intelligence

**16 novembre 2026 — audit documentaire indépendant, avant corrections R6.** Périmètre : [protocole expert](../../agents/knowledge-expert.md), shared-knowledge, knowledge-indexing, traceability et les sept documents F009, puis raccords nécessaires. Aucun runtime Unleash examiné. Trois écarts de contrat sont retenus ; les comportements produit restent à qualifier.

## R6-E07-01 — Le reçu de discovery exige une invocation inexistante

**Priorité haute ; contradiction reproduite dans le schéma.** Le [schéma canonique](../../schemas/discovery-receipt.schema.json) impose `invocation_id`, chaîne non vide, pour tout reçu. Or [F019/interfaces](../../features/F019-reusable-automations/interfaces.md), `CreateAutomationDraft`, exige une décision de discovery sans appel modèle implicite ; le [modèle SQL](../../architecture/sqlite-design.md), origines d'actions, interdit les invocations fictives pour commandes humaines ou mécaniques.

**Scénario / risque.** Un utilisateur crée une procédure depuis un projet sans agent. Sa recherche doit produire un reçu avant création. Contrôle documentaire exécuté avec JSON Schema : la [fixture agente](../../examples/discovery-receipt.json) était acceptée ; remplacer son auteur par `user:louis` et retirer `invocation_id` était rejeté ; `null` également. Les équipes auraient dû bloquer le parcours, contourner le schéma ou inventer une attribution.

**Solution.** Raccorder le reçu à une origine typée existante : principal authentifié et cause durable toujours requis ; invocation seulement pour l'origine agente ; commande de contrôle ou run/worker pour les autres origines. Vérifier les appartenances côté domaine et conserver l'antériorité. **Alternative :** rattacher le reçu à une action durable dont l'origine porte déjà cette union, sans dupliquer les règles.

**AC.** Recherche et création humaines acceptées sans invocation ni modèle ; reçu agent sans invocation rejeté ; cause étrangère au principal/scope rejetée ; retry de commande conservant le même résultat durable.

## R6-E07-02 — La provenance sans Git n'est pas raccordée à F009

**Priorité moyenne ; contrat interdomaines manquant.** [shared-knowledge](../../specs/shared-knowledge.md), « Identité de source », rend `repo_id` obligatoire et définit les champs/états Git. [F009/interfaces](../../features/F009-knowledge-engine/interfaces.md) renvoie `source_snapshot_id` pour ses candidats ; sa [conception](../../features/F009-knowledge-engine/technical-design.md) décrit la reprise par rapprochement avec Git et registries. Pourtant la [carte des dépendances](../../architecture/dependency-map.md) maintient fichiers/documents et discovery sans F010. [F013/interfaces](../../features/F013-document-connectors/interfaces.md) possède déjà `ResourceSnapshot` : le problème est son raccord à F009, pas l'absence de tout modèle documentaire.

**Scénario / risque.** Une mission sans repository recherche un contrat dans un document sélectionné, puis ce document change ou perd son autorisation. Aucun contrat commun ne précise quel type de snapshot le reçu référence, ni quelle révision remplace la génération Git dans sa revalidation. Une équipe peut perdre cette provenance ou marquer la source inconnue uniquement parce que Git est absent.

**Solution.** Définir une référence discriminée commune vers snapshot Git, révision documentaire/fichier et version de registre. Réutiliser les identités existantes ; préciser pour chaque variante digest/révision, ACL, cohérence, invalidation et reprise. Les champs Git restent obligatoires dans leur variante. **Alternative :** conserver des DTO distincts et documenter une projection typée vers les sources de discovery.

**AC.** Projet sans dépôt : document trouvé et cité exactement ; changement/révocation invalidant le reçu concerné ; crash puis réconciliation sans Git ; aucune identité Git fabriquée ; scénarios worktree/nonmergé inchangés.

## R6-E07-03 — Validité préalable et fraîcheur à la clôture se confondent

**Priorité moyenne ; règle temporelle ambiguë.** [shared-knowledge](../../specs/shared-knowledge.md), « Discovery », invalide les reçus après modification pertinente et impose un nouveau reçu pour revalidation. [traceability](../../specs/traceability.md), « Recherche avant création », relie la clôture aux reçus valides. Le [contrat](../../specs/contract-enforcement.md) refuse qu'une recherche postérieure satisfasse normalement l'antériorité. Le schéma ne distingue que `valid/stale/unknown` et avant/après création.

**Scénario / risque.** À la génération G, la recherche préalable autorise un nouveau helper ; sa création ajoute précisément un candidat au périmètre recherché, puis son commit avance la source. Le reçu devient périmé. À la clôture, sa revalidation est nécessairement postérieure à la création. Aucune règle ne dit comment conserver la conformité historique sans accepter un reçu périmé pour une nouvelle action. Deux équipes peuvent donc produire des verdicts opposés sur le même journal.

**Solution.** Enregistrer une validation durable du reçu pour l'opération autorisée, avec révisions observées et séquence préalable. La clôture vérifie cette utilisation historique ; la fraîcheur actuelle gouverne les utilisations futures. **Alternative :** reconstruire explicitement le verdict « valide au moment de l'effet » depuis les événements retenus ; historique insuffisant implique `unknown`.

**AC.** Recherche valide → création contrôlée → commit → clôture conforme à cette obligation ; modification pertinente avant création bloquante ; recherche tardive toujours distincte ; validation historique inutilisable pour autoriser une nouvelle création.

## Désaccords et points ouverts

Je ne retiens pas comme défauts la capture dirty `unverified`, les limites LSP/interlangages, les créations opaques détectées tardivement ou l'absence de métriques : elles sont explicitement assumées. Provenance des worktrees, distinction découverte/disponibilité, faux amis sémantiques et ACL avant classement sont correctement cadrés. Restent à qualifier versions/capacités des cinq profils, corpus de pertinence, budgets et rétention. Les corrections proposées conservent toutes les fonctions V1, sans imposer Git, embeddings ni invocation d'agent.

## Addendum — corrections appliquées

E07-01/02/03 corrigés dans shared-knowledge, knowledge-indexing, F009 et le schéma cible V1 non publié : origine dérivée, sources typées, usage historique et revalidation bornée séparés. Quatre fixtures ajoutées/mises à jour ; métaschéma valide, six cas positifs acceptés et trente-deux négatifs rejetés. Dates contrôlées explicitement après détection du checker optionnel absent ; détails dans [tests F009](../../features/F009-knowledge-engine/tests.md). Tests produit toujours not_run. Raccords F008, F016, traceability et catalogue d'opérations/événements confiés à la consolidation ; aucune preuve d'autorité runtime revendiquée.
