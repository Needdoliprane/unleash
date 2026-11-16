# R6 — Expert 01 — Chief Software Architect

**Date : 16 novembre 2026. Statut : audit de conception indépendant du dossier R5, avant corrections R6.** Aucun produit, binaire ou test produit examiné. Périmètre : protocole chief-architect, README, invariants, révision R3–R5, architecture globale, composants, dépendances, workspace, stockage, IPC, automatisations et contrats F001/F012. Constats fondés sur les fichiers réels ; sources internes ci-dessous.

## Verdict et limites

Le principe d’un seul moteur est cohérent : AutomationRun est une projection du Run, les actions mécaniques emploient le WorkflowReducer, le broker, le writer et le pool communs. Les moteurs navigateur/inférence restent des dépendances externes. Aucune nécessité de nouvel orchestrateur n’est identifiée ([architecture](../../architecture/system-overview.md), [composants](../../architecture/component-model.md), [profil v1.1](../../architecture/automation-execution-profile.md)). Deux raccords documentaires doivent toutefois être fermés avant que des équipes puissent implémenter les contrats indépendamment.

## R6-E01-01 — Probe modèle sans propriétaire d’exécution représentable

**Priorité haute ; défaut documentaire confirmé.** [local-models](../../specs/local-models.md), sections localité/qualification, autorise un test utilisateur indépendant et un probe de génération. [F012/interfaces](../../features/F012-model-api-router/interfaces.md), sections Types/Manifestes, exige pourtant invocation_id, attempt_id et invocation_bundle_digest ; [sqlite-design](../../architecture/sqlite-design.md), table model_calls, impose invocation_id. [scheduling](../../specs/scheduling.md), Identités/Compteurs, attribue chaque appel modèle à une invocation admise et ne connaît que agent_invocation/replay_worker. L’origine control_plane des actions ne ferme pas ces autres contrats.

**Scénario et risque.** À la première configuration, aucun agent n’existe ; TestProviderProfile demande une génération. Une équipe devra inventer une invocation, rejeter le parcours prévu ou contourner le modèle de persistance/admission. Aucun de ces comportements runtime n’est prétendu observé.

**Correction recommandée.** Définir une origine d’appel fermée invocation|provider_probe. Le probe possède un ProviderProbeContext durable : principal authentifié, receipt/cause, profil exact, politique et contexte synthétique épinglés, budgets, limites, état et admission. ModelRequest, ModelCallManifest, contexte et model_calls appliquent cette union ; les références d’invocation sont obligatoires seulement dans leur variante. Ajouter provider_probe aux propriétaires du **même** pool global 5/50 et conserver le quota fournisseur ; aucun agent fictif ni deuxième scheduler.

**Objection à traiter.** Le principal ne peut effacer sa filiation : une demande issue d’un run forbidden reste interdite, même renommée probe. Distinguer validation/connexion/inventaire et génération pouvant charger un modèle. Une annulation sans preuve d’arrêt conserve l’incertitude et les réservations concernées.

**Alternative.** Matérialiser une véritable invocation de maintenance, avec agent/session/bundle et admission existants, explicitement visible ; cette voie demande davantage d’objets et ne doit pas fabriquer des identifiants sans leur sémantique.

**Acceptation attendue.** Sans agent préexistant, génération de test attribuable et bornée ; somme invocations/replays/probes ≤ limite globale ; quota local respecté ; retry de commande sans double probe ; interruption/reprise conservant l’issue inconnue ; probe descendant d’un replay forbidden refusé avant dispatch. Contrats documentaires concordants avant tests produit futurs.

## R6-E01-02 — Catalogue wire sans autorité machine complète

**Priorité moyenne ; défaut de couverture documentaire, pas vulnérabilité démontrée.** Le [catalogue client](../../architecture/client-operation-catalog.md) fournit les noms et annonce GetClientCapabilities, mais aucun registre machine ne relie nom, classe, propriétaire et payload. [ipc-command.schema.json](../../schemas/ipc-command.schema.json) ne valide que l’enveloppe : command_type est une chaîne à motif et payload un objet libre. Contrôle documentaire exécuté avec les formats activés : StartRun avec payload vide et UnknownCommand sont acceptés par ce schéma, conformément à sa portée limitée.

Une divergence réelle existe : [F001/interfaces](../../features/F001-core-daemon/interfaces.md), Opérations, nomme GetDaemonStatus et StopDaemon, absents du catalogue complet ; son GetSnapshot désigne une projection/watermark, tandis que le catalogue l’emploie pour snapshots de travail/connaissance. Le lecteur doit arbitrer les frontières lui-même.

**Scénario et risque.** Une équipe génère un client depuis les schémas, une autre depuis le Markdown : noms disponibles, payloads et capacités peuvent diverger sans contrôle documentaire permettant de le détecter.

**Correction recommandée.** Ajouter un registre machine versionné : nom unique, classe command/query/stream/handshake, propriétaire, contrat normatif et références de payload/résultat. Distinguer explicitement validation d’enveloppe, résolution de commande et validation du payload. Identifier les contrats incomplets, qui ne permettent aucune qualification positive. Raccorder ou distinguer les noms F001 et les deux sens de GetSnapshot. Dériver ou comparer catalogue, capacités et parité depuis cette autorité.

**Objection / alternative.** Un simple enum ne valide pas le métier. Une génération depuis les futurs types Rust est crédible, à condition d’inscrire cette dépendance au jalon M0 et de conserver aujourd’hui un index contractuel exploitable, sans prétendre disposer des handlers.

**Acceptation attendue.** Chaque nom wire normatif possède exactement une entrée ou une exclusion motivée transport/interne ; aucun doublon sémantique implicite ; commande inconnue et payload manquant rejetés par le contrat concerné ; couverture incomplete visible ; égalité vérifiable avec les opérations promises aux clients.

## Risques assumés, hypothèses et vérifications restantes

Le découpage crates/modules et les choix de moteurs demeurent proposés. Isolation réelle, protocoles externes, performance et comportement après crash restent à qualifier ; leur absence de preuve produit ne constitue pas un nouveau défaut documentaire. Les corrections ci-dessus conservent les 19 features, l’autorité du harness et la réutilisation des workflows. Relecture requise après intégration des contrats et de leurs matrices.
