# F013 — interfaces et contrats

## DTO et opérations

ResourceRef porte connector_account_id, provider_kind, resource_kind, provider_object_id et container_id éventuel, selon le [modèle commun](../../specs/connectors.md). ResourceDescriptor ajoute titre, type, parent_refs, display_url, permissions_observed_at, revision_hint et capabilities. ResourceRevision porte etag/version quand disponible, observed_at, parent_refs et permission_snapshot_ref. ResourceSnapshot, copie matérialisée d’une révision, porte snapshot_id, revision_ref, content_hash, extraction_version, capture_consistency (enum du schéma discovery-receipt), byte_count et evidence_ref.

Commands : ConnectAccount, CancelAccountConnection, DisconnectAccount, TestConnectorAccount, AssignResource, AssignDocumentScope, RemoveAssignment, ConfigureWatch et RefreshResource. Queries : BrowseResources, SearchResources, GetResource, PreviewResource, ListAssignments, ListConnectorAccounts, GetConnectorAccount et GetAccountConnection, conformément au [catalogue commun](../../architecture/client-operation-catalog.md). ConnectAccount porte `client_profile` (`ovpt_official`, ou `bring_your_own` avec `profile_ref`) et le niveau de scope demandé (Google `drive.readonly` ; Microsoft `Files.Read`, puis `Files.Read.All` par consentement incrémental), selon Q-08. Les arguments d’affectation portent owner_type/owner_id, access_mode, revision_policy et expected_revision.

## Réponses et pagination

Une liste retourne items, opaque_next_cursor, coverage, observed_at et source_kind remote/local. Preview retourne texte ou représentation supportée, références de blocs/pages, avertissements de contenu manquant et taille originale. Aucun résultat ne révèle le nom d’une ressource hors scope.

DocumentChanged, DocumentAccessRevoked, DocumentSnapshotCreated et DocumentAssignmentChanged fournissent la cause et les missions touchées. Les sources du contexte référencent snapshot_id ; une URL humaine seule ne permet pas de reproduire la sélection.

## Erreurs et idempotence

L’ajout d’une affectation dupliquée conserve une seule identité métier. La modification attend sa révision. La déconnexion désactive l’usage futur et propose une suppression des copies selon politique ; elle ne réécrit pas les manifests d’invocations passées.

Un curseur expiré retourne une action de resynchronisation précise, pas un retour silencieux à tous les documents de l’entreprise. Une recherche partielle conserve ses limites même si le modèle souhaite une réponse exhaustive.

Le transport commun et les codes d’erreur sont définis dans [ipc-and-apis](../../architecture/ipc-and-apis.md). Les noms de DTO ci-dessus sont des propositions Unleash, pas des APIs de bibliothèques ou services supposées existantes.

Projection wire des erreurs F013, enregistrée par le propriétaire dans le format commun d’[ipc-and-apis](../../architecture/ipc-and-apis.md) (nom de domaine, classe/reason_code, retry_class). AuthenticationFailed reste réservé à l’authentification IPC ; une réauthentification fournisseur n’est jamais projetée ainsi.

| Nom de domaine | Wire (classe/reason_code) | retry_class |
| --- | --- | --- |
| AuthRequired | CapabilityDenied/CONNECTOR_AUTH_REQUIRED | after_human_decision |
| AccessRevoked | CapabilityDenied/CONNECTOR_ACCESS_REVOKED | never |
| AccessUnverified (403 de raison inconnue) | CapabilityDenied/CONNECTOR_ACCESS_UNVERIFIED | after_refresh |
| AppNotAuthorized (Drive `appNotAuthorizedToFile`) | CapabilityDenied/CONNECTOR_APP_NOT_AUTHORIZED | after_human_decision |
| ResourceMissingOrHidden | NotFoundOrHidden/RESOURCE_MISSING_OR_HIDDEN | never |
| CursorExpired (`opaque_next_cursor` d’une requête paginée qui n’est plus servi ; jamais en réponse à une commande) | ResyncRequired/CURSOR_EXPIRED | after_refresh |
| UnsupportedFormat | ContractViolation/UNSUPPORTED_FORMAT | never |
| RateLimited (429/529 ou 403 de quota) | RateLimited/PROVIDER_RATE_LIMITED | after_backoff |
| AccountIdentityMismatch | CapabilityDenied/ACCOUNT_IDENTITY_MISMATCH | after_human_decision |

PartialCapture n’est pas une Error : c’est une couverture partielle signalée dans le résultat. Conformément à [ipc-and-apis](../../architecture/ipc-and-apis.md), ResyncRequired ne répond qu’à une lecture incrémentale. L’expiration d’un curseur fournisseur du Watch, y compris pendant ConfigureWatch ou RefreshResource, déclenche une réconciliation bornée interne et n’est jamais renvoyée comme refus d’une commande mutante.

## Contrat compte/identité R6

ProviderPrincipalBinding (provider_kind, issuer, subject_type user/integration/service, subject_id, tenant_or_workspace_id éventuel, binding_digest) est lié au connector_account_id. Toute activation de credentials porte auth_generation et résultat de vérification d’identité/scopes. Claims décidés par fournisseur : Google `iss` + `sub` et Microsoft `tid` + `oid` issus de l’id_token validé ; Notion identifiant du bot (Q-09). La [règle de réauthentification](../../specs/connectors.md) interdit le remplacement implicite par un autre principal. Refresh/Watch et ResourceSnapshot conservent cette provenance ; la qualification réelle des claims relève d’EXT-DOC.
