# F016 — Interfaces proposées

Les commandes utilisent l'enveloppe IPC commune. Les identités d'acteur sont résolues serveur ; `command_id` fournit l'idempotence, et `expected_revision` protège les agrégats modifiables. Les lectures paginées n'écrivent pas artificiellement une nouvelle observation métier à chaque affichage.

## Enregistrement des catégories

`RecordClaim`, `RecordObservation` et `RecordJudgment` passent par l’enveloppe IPC commune sur le canal worker/harness authentifié et figurent au [registre des opérations](../../architecture/client-operation-catalog.md) avec F008, hors de tout groupe PAR ; GUI, TUI et CLI ne les appellent pas. Leur émetteur et son autorité sont résolus par ce canal ([F008/interfaces](../F008-contract-engine/interfaces.md)) : tentative du worker pour un claim, adaptateur authentifié pour une observation, décideur autorisé pour un jugement.

`RecordClaim` suit [contract-claim.v1](../../schemas/contract-claim.schema.json) et [F008/interfaces](../F008-contract-engine/interfaces.md) : sujet `SubjectRef` exact, texte explicatif, proposition `observable` (`process.exit_code` ou `test.expectation_satisfied`, version 1) ou `qualitative`, et `observation_ids` candidats. `claim_id` et `author_ref` sont attribués par le serveur depuis le canal authentifié ; le contexte invocation/mission est dérivé de ce canal. `RecordObservation` est réservé aux adaptateurs habilités du harness ; un worker LLM ne peut pas se déclarer adaptateur de test. `RecordJudgment` reçoit contrat, décisions, observations et claims considérées, motif explicite et autorité requise. Les trois utilisent des types distincts et des permissions séparées.

```json
{
  "command_type": "RecordClaim",
  "payload": {
    "schema_version": "unleash.contract-claim.v1",
    "subject": {
      "subject_id": "candidate-7",
      "subject_revision": 7,
      "scope_id": "scope-accounts",
      "scope_revision": 3,
      "snapshot_ref": {
        "kind": "source_snapshot",
        "id": "snapshot-candidate-7",
        "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
      }
    },
    "statement": "La validation demandée a terminé avec succès sur le candidat 7.",
    "proposition": {
      "kind": "observable",
      "predicate_ref": {"id": "test.expectation_satisfied", "version": 1},
      "expectation_ref": {"id": "workspace-tests", "version": 1, "sha256": "71488d164fd911e6ffb9f65f7542827eb26286f62fa5f2dedd8fa2f971c0fe77"},
      "execution_binding_id": "binding-7",
      "asserted_value": true
    },
    "observation_ids": ["test-observation-51"]
  }
}
```

Le payload est l’objet contract-claim.v1 sans `claim_id` ni `author_ref`. Ses empreintes sont illustratives et respectent le motif `^[a-f0-9]{64}$` du schéma : `snapshot_ref` reprend celle de l’[exemple contract-claim](../../examples/contract-claim.json) ; `expectation_ref` porte l’empreinte RFC 8785 de l’[attendu exemple](../../examples/expected-test-execution.json), comme l’[ExpectedExecutionBinding exemple](../../examples/expected-execution-binding.json). Le rapport restitue séparément l’intégrité des liens (`claim_evidence_links`) et la vérité observable `claim_truth` pass/fail/unknown de F008 ; `ClaimContradicted` ne concerne qu’un prédicat fermé et conserve les deux objets. Le texte de claim reste conservé avec son auteur même si sa vérité est fail.

## Rapport déterministe

```json
{
  "command_type": "CreateReportSnapshot",
  "payload": {
    "subject": {"kind": "mission", "id": "mission-accounts"},
    "boundary": "current_transactional_snapshot",
    "sections": ["outcome", "deliverables", "evidence", "discovery", "uncertainties"],
    "include_model_summary": false,
    "format": "json"
  }
}
```

La réponse comporte report_id, schema_version, as_of_cursor {stream_id, sequence}, access_policy_revision, sections, artifact_refs et missing_sources. La frontière courante est choisie par le serveur dans la même vue transactionnelle que les données capturées ; le client ne demande pas une séquence historique arbitraire. En V1, boundary ∈ {current_transactional_snapshot} ; toute autre valeur est InvalidArgument. La seule lecture historique est GetReportSnapshot(report_id), qui relit un rapport déjà scellé : un rapport absent ou purgé donne HistoryUnavailable (code NotFoundOrHidden, reason_code HISTORY_UNAVAILABLE), des entrées purgées donnent missing_sources, sans reconstruire fictivement des champs supprimés. Les sections de validation distinguent `requested`, `executed`, `not_requested`, `not_observed`, `failed`, `passed_for_snapshot` et `unknown`. La conclusion de mission conserve son propre statut ; elle ne se confond pas avec celui d'une seule validation.

## Manifestes et événements

`GetInvocationManifest` retourne l'InvocationBundle : agent, role_version, session, parent, workflow/run/cycle/step, configuration modèle demandée et politique de modèles autorisés, skill_versions, scope, document_refs, event_refs, action_refs, decision_refs, deliverable_refs, evidence_refs et la liste ordonnée des manifest_id de ses appels. `GetContextManifest` restitue chaque [ContextManifest v2](../../schemas/context-manifest.schema.json) : `segments` (kind, source_ref, content_hash, selection, availability, reason), model_profile_ref, provider_profile_ref et model_id effectifs. La transmission et la réception proviennent du ModelCallRecord et restent distinctes du placement `selection`. Un champ de raisonnement privé n'existe pas dans le contrat.

`SubscribeEvents` reçoit stream_id, after_sequence et filtres autorisés, conformément au contrat IPC commun. Le serveur retourne events, next_cursor, watermark avec le même stream_id, et éventuellement ResyncRequired. `GetActivitySnapshot` sert de base cohérente après perte de fenêtre. Les clients dédupliquent par stream_id/event_id/sequence ; une même séquence dans une nouvelle lignée n'est pas le même événement. Ils montrent l'état de connexion du client séparé de l'état du daemon.

## Preuves, export et erreurs

`EvidenceRef` contient id, kind, source_snapshot_id optionnel, digest, size_bytes, completeness, access_policy_ref, retention_class et availability. `ExportReport` est une commande qui reçoit report_id, format, périmètre et destination.kind=client_stream. Elle prépare un artefact autorisé et retourne le reçu commun ; ReadArtifact en lit ensuite les octets. Le chemin de fichier choisi est traité par l’adaptateur client de confiance, pas comme chemin arbitraire du daemon. L'export inclut un manifeste de fichiers et leurs digests ; les sources absentes ou masquées sont listées avec motif. Un export n'obtient pas plus d'accès que l'utilisateur qui le demande.

| Nom de domaine | Interprétation | Comportement attendu |
|---|---|---|
| `evidence_missing` | Référence inconnue ou octets supprimés | Rapport avec lacune ; pas de preuve simulée |
| `evidence_stale` | Source différente du sujet actuel | Montrer ancienne portée et exiger décision de réemploi |
| `journal_unavailable` | Écriture durable indisponible | Bloquer nouveaux effets nécessitant intention |
| `resync_required` | Curseur hors fenêtre | Recharger snapshot, reprendre au watermark |
| `access_revoked` | Autorisation retirée | Retirer contenus des nouvelles vues et contextes |
| `payload_conflict` | command_id réutilisé pour une autre commande | Refuser sans nouvel effet |
| `outcome_unknown` | Effet possible sans observation complète | Conserver l'incertitude et proposer observation |

Les noms snake_case de ce tableau sont des noms de domaine. Sur le wire, ils emploient l’Error commun ([IPC](../../architecture/ipc-and-apis.md)) : classe commune, reason_code UPPER_SNAKE et retry_class. F016 enregistre ses raisons dans le format de l’IPC :

| Nom de domaine | Wire (classe/reason_code) | retry_class |
| --- | --- | --- |
| `evidence_missing` | NotFoundOrHidden/EVIDENCE_MISSING | never |
| `evidence_stale` | ContractViolation/STALE_EVIDENCE (raison transverse déjà enregistrée par l’IPC, non redéfinie) | after_refresh |
| `journal_unavailable` | InternalFailure/JOURNAL_UNAVAILABLE | after_backoff |
| `access_revoked` | ScopeDenied/ACCESS_REVOKED | after_human_decision |
| HistoryUnavailable | NotFoundOrHidden/HISTORY_UNAVAILABLE | never |

Si l’existence de la ressource doit rester masquée, la réponse est NotFoundOrHidden avec reason_code null, jamais ACCESS_REVOKED. `resync_required`, `payload_conflict` et `outcome_unknown` ne sont pas des raisons propres : ce sont respectivement la classe ResyncRequired (retry_class=after_refresh), IdempotencyConflict et OutcomeUnknown/UNRESOLVED_EFFECT. Dans un rapport produit, une preuve manquante ou périmée reste un état de section ou une entrée de missing_sources, pas une Error.

Les événements `ClaimRecorded`, `ObservationRecorded`, `JudgmentRecorded`, `EvidenceAvailabilityChanged`, `ReportSnapshotCreated` et `ContextManifestSealed` portent séquence et causalité. ResyncRequired n'est pas un événement du journal : c'est la réponse Error.code=ResyncRequired de SubscribeEvents ou de la lecture concernée ; sa fréquence peut être mesurée par un compteur de télémétrie non durable. Les gros contenus restent hors du bus et soumis à contrôle d'accès lors du téléchargement.

## Discovery et temporalité R6

Les rapports exposent séparément DiscoveryReceipt immuable, DiscoveryUseObservation de la création, éventuelle completion_revalidation et fraîcheur courante. Les sources et origines typées suivent [F009](../F009-knowledge-engine/interfaces.md), y compris humain/replay et fichiers/documents sans Git. Une modification ultérieure pertinente réévalue l’applicabilité, sans effacer la preuve d’une recherche antérieure réellement observée. Un simple commit de son propre résultat identique ne transforme pas cette preuve en violation automatique. Les projections restent sous ACL et indiquent absence/inconnu/périmé.

La lecture `GetDiscoveryReceipt` suit la projection paginée et autorisée de [F009](../F009-knowledge-engine/interfaces.md#lecture-bornée-getdiscoveryreceipt) : le rapport distingue le reçu historique, ses états courants au watermark et les références d’invalidations et de validations visibles. Il ne déduit aucune absence globale d’une liste vide ni ne restitue les détails F002 sans autorisation. Une lacune ou une rétention insuffisante reste visible.
