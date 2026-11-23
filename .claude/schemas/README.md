# Schémas et fixtures de conception

Ces JSON Schema 2020-12 décrivent des **contrats Unleash de conception**, à implémenter à partir de M0 ; ils ne sont pas des APIs de bibliothèques ou des endpoints AI-Guard existants. Les modèles Rust devront produire/consommer ces formes et un validateur de domaine assurera les contraintes inter-objets.

## Autorités

- ipc-command : enveloppe commune de commande et payloads fermés, dont `TestProviderProfile` et [`PublishWorkflowVersion`](../features/F006-workflow-compiler/interfaces.md#publication-dune-version). Le serveur vérifie l’acteur authentifié et la révision attendue.
- ipc-receipt / ipc-error : réponse commune, sous-résultat métier référencé, erreurs normalisées.
- event : enveloppe wire de fait engagé, cause typée et curseur stream_id/sequence.
- workflow : AST fermé ; graphe, domination, bornes, registre et permissions nécessitent une passe sémantique.
- skill / role / contract-profile : bundles, entrée principale et profils exacts ; aucune compilation de prose en preuve.
- discovery-receipt : decision reuse/create distincte de strategy ; identité, fraîcheur et provenance vérifiées par le domaine.
- context-manifest : manifeste immuable d'un appel API précis, lié au bundle d'invocation. Le statut de transmission appartient à ModelCallRecord, pas à une mutation du manifeste.
- ipc-query / ipc-query-response : enveloppe de lecture et réponse avec watermark transactionnel optionnel. La variante `terminal_acquisition_response` décrit le [résultat causal F011](../features/F011-terminal-pool/interfaces.md#lecture-causale-de-lacquisition). La variante `workflow_run_list_response` impose un watermark pour la [page de runs F005](../features/F005-workflow-engine/interfaces.md#liste-des-runs-par-projet).
- ipc-hello : trames de négociation Hello client-serveur (HelloRequest et HelloResponse, livrable M0).
- ipc-subscribe-events : abonnement aux événements et trames de flux/reprise (SubscribeEventsRequest et SubscribeEventsFrame, livrable M0).
- ipc-client-upload-segment : trame de segment pour l'import de données client_upload (ClientUploadSegment, livrable M0).

Un digest se compare après normalisation vers le couple (algorithme, octets) : un champ nommé `sha256` en hexadécimal nu, la forme `sha256:<hex>` et l'objet `{algorithm: "sha256", hex}` sont équivalents. Toute nouvelle version de schéma adopte la forme `sha256:<hex>`.

Le profil de validation active explicitement les formats UUID/date-time ; JSON Schema n'impose pas à tous les validateurs de les vérifier. Détecter aussi les clés dupliquées pendant décodage, avant un parseur qui les écraserait. Interdire résolution réseau de $ref ; résoudre uniquement le catalogue local versionné autorisé.

## Limites qui doivent rester visibles

Un schéma valide ne prouve pas l'existence d'un role_ref, l'autorité d'un actor_ref, la fraîcheur d'un snapshot, le respect d'un plafond ou la véracité d'une claim. Les passes de domaine comparent digests exacts, grants, versions et états. Une chaîne dans un champ description ne crée aucune règle exécutable.

Les fixtures [examples](../examples/workflow-fixtures.md) couvrent workflow normal et merge-only, skills, rôles et profil. Les autres exemples couvrent IPC, découverte et contexte. Ils constituent des données de test proposées. Les validations réellement exécutées pendant cette mission sont consignées dans [document-validation](../testing/document-validation.md) ; elles ne remplacent pas les tests Rust du futur produit.

Les structures Rust et migrations physiques peuvent utiliser d'autres représentations internes, à condition que leur conversion vers ces enveloppes soit explicite, sans perte des champs requis, et testée par round-trip. Tout changement incompatible augmente la version de protocole/schéma et met à jour les fixtures et consommateurs. Exception : un contrat cible jamais publié par une release peut être corrigé sous sa version tant qu'aucune release ne l'a publié ; DiscoveryReceipt v1 désigne ainsi la forme corrigée en R6 et resserrée en R7 (`stream_id` UUID, `namespace_ref` égal à `scope_owner_key`) ; la forme R5 reste une archive. Le registre local résout chaque URN dans la révision du paquet importé.


## Profil workflow v1.1 R3–R5

[workflow-v1.1.schema.json](workflow-v1.1.schema.json) conserve le schéma v1 et ses fixtures séparés, ajoute des actions sans agent, model_policy et un sous-plafond de workers d’action. Son sous-ensemble récursif machine-only interdit structurellement les tâches agentes et budgets modèle positifs sous forbidden. La [fixture CSV](../examples/automation-reconciliation-workflow.json) est documentaire, sans adaptateur réel ni fichier privé. Les opérations/version, grants, dépendances et effets nécessitent toujours une validation sémantique.

[automation-observation.schema.json](automation-observation.schema.json) formalise le rapport de bloc commun et les incohérences interdites entre effet inconnu et poursuite. Les fixtures d’observation et de supervision sont dans [workflow-fixtures](../examples/workflow-fixtures.md) ; schémas de rôle/skill existants réutilisés.

## Contrats R6

[workflow-v1.2.schema.json](workflow-v1.2.schema.json) ajoute for_each sans changer v1/v1.1 ; voir [sémantique](../architecture/workflow-data-iteration.md) et [fixture](../examples/automation-rows-workflow.json). Les schémas design-catalog, design-operations, design-traceability et design-package-manifest décrivent le [package de conception](../catalog/README.md), pas le runtime. Leur champ `revision` admet toute révision `R<n>` ; le registre de l'importeur liste les révisions acceptées ([ingestion](../specs/document-package-ingestion.md)). [operation-name.schema.json](operation-name.schema.json) valide les noms déclarés ; il ne valide pas les payloads métier ni les permissions.

L’enveloppe de commande cible passe en unleash.ipc.v2 pour imposer authority_generation, immuable lors d’une retransmission. Les clients v1 sont refusés pour mutation, sans migration silencieuse de leur outbox. Les formats de receipt/event restent leurs contrats propres. Le manifeste de contexte agent est unleash.context.v2 pour ses politiques/filiations obligatoires ; aucun produit v1 existant n’est présumé déployé.

Les formats [contract-claim](contract-claim.schema.json), [expected-test-execution](expected-test-execution.schema.json), [expected-execution-binding](expected-execution-binding.schema.json) et [test-execution-observation](test-execution-observation.schema.json) décrivent les comparaisons observables de F008. Leurs URN sont résolues dans le registre local fermé, jamais téléchargées. Les anciennes fixtures de skill/profil restent valides ; les nouvelles test-verification-* exigent le catalogue de validateurs v2 et des attentes scellées avant effet.
