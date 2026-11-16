# Connecteurs documentaires — Explore, Pin, Scope, Watch

Statut : contrat de V1, Google Drive, OneDrive et Notion tous requis ; ce document est l’autorité détaillée de Q-08 et Q-09, résolues R7 par arbitrage délégué ([registre](../brief/open-questions.md)). [F013](../features/F013-document-connectors/README.md), INV-038/039. Sources vérifiées dans [security-sources](../research/security-sources.md) ; la qualification sur comptes réels reste la gate EXT-DOC.

## Modèle commun

ResourceRef = connector_account_id + provider_kind + resource_kind + provider_object_id (+ container_id éventuel, par exemple drive_id OneDrive) ; l’URL d’affichage n’est pas l’identité. ResourceRevision est la version observée chez le fournisseur : etag/version quand disponible, observed_at, parent_refs et permission_snapshot_ref. ResourceSnapshot est la copie matérialisée d’une révision : snapshot_id, revision_ref, content_hash, extraction_version, capture_consistency (enum `immutable_version_coherent`/`unverified`/`inconsistent` de [discovery-receipt](../schemas/discovery-receipt.schema.json)), byte_count et evidence_ref. Un connecteur ne promet pas un historique de versions que son API ne fournit pas.

DocumentAssignment relie ressource ou scope à un projet, mission, agent, rôle/skill ou workflow et précise politique latest_at_activation ou pinned_revision. Une invocation active conserve sa révision assemblée ; un changement de source produit une notification et un nouveau choix, pas une modification rétroactive du contexte.

Explore recherche/navigue dans les ressources autorisées et indique la portée réelle. Pin référence un document précis, éventuellement une révision matérialisée. Scope sélectionne des racines et filtres de ressources, bornes profondeur/nombre/taille et règles d’inclusion. Watch suit des changements ; il ne donne aucun droit supplémentaire ni inclusion automatique de tout document dans le contexte.

## Capacités par service

Google Drive : fichiers/folders par IDs, documents Google exportés dans un format textuel supporté, fichiers binaires téléchargés selon droits. Scopes V1 (Q-08) : `openid` et `drive.readonly`, avec `access_type=offline` ; ce scope restreint exige la vérification Google pour le profil `ovpt_official`. `drive.file` n’est pas utilisé en V1 : il n’accorde l’accès qu’aux fichiers créés par l’application, ouverts avec elle ou choisis par l’utilisateur dans le Google Picker, et ne permet donc pas de lire un fichier existant désigné par ID ou lien. Aucun picker n’est introduit en V1, ni intégré à la WebView, ni par le parcours Picker desktop documenté par Google (navigateur système, `trigger_onepick=true`, drive.file seul et non combinable) ; ce parcours n’entrerait que par réexamen de Q-08. Saisir un ID ou un lien ne vaut jamais autorisation : un refus est classé selon la règle d’échec ci-dessous, jamais présumé révocation. Les Pin, Scope et affectations d’Unleash réduisent toujours ce plafond fournisseur. Les shared drives ont leur suivi propre.

OneDrive : identités drive_id (container_id) + item_id (provider_object_id), navigation children, lecture métadonnées/contenu, eTag et delta quand endpoint et compte le permettent. Les liens nextLink/deltaLink sont opaques. Scopes V1 (Q-08) : `openid`, `offline_access` et `Files.Read` délégué par défaut ; `Files.Read.All` par consentement incrémental à la première sélection d’une ressource partagée ou SharePoint ; les permissions `*.Selected` sont hors V1. Le connecteur expose les différences compte personnel / tenant professionnel / SharePoint.

Notion : pages, blocs, databases/data sources selon version API épinglée, partage explicite à l’intégration. La recherche distante sur titres n’est ni exhaustive ni full-text. La recherche locale du contenu est limitée aux ressources sélectionnées et ingérées. Le rendu conserve les blocs non pris en charge comme marqueurs explicites ; les fichiers signés expirants sont récupérés via une référence à jour. Ne pas inventer un endpoint d’historique complet de page.

## Authentification

Google et Microsoft (Q-08) : clients publics d’application native (RFC 8252). Navigateur système lancé depuis la GUI ou la TUI ; code d’autorisation avec PKCE S256 (verifier de 64 caractères) ; `state` et `nonce` aléatoires de 256 bits ; redirection loopback sur l’IP littérale `127.0.0.1` à port éphémère ; écouteur à usage unique, fermé au premier retour valide ou après 10 minutes (tentative `expired`) ; page de retour statique sans script ; vérification du fournisseur et de l’URI.

Le profil de client est choisi par compte dans Settings et porté par ConnectAccount (`client_profile`) :

- `ovpt_official` : clients enregistrés par OVPT (Google, type *Desktop app* ; Microsoft Entra multilocataire et comptes personnels, plateforme *Mobile and desktop*, client public sans secret ni certificat). Leurs identifiants figurent uniquement dans le profil de distribution des artefacts officiels signés, absent du dépôt et des builds communautaires. Ce profil n’est activé dans une version publique qu’après vérification Google de l’application et des scopes, et vérification d’éditeur Microsoft ; à défaut, la release est livrée avec `bring_your_own` seul, sans perte de capacité.
- `bring_your_own` : client fourni par l’utilisateur ou son organisation (Google Desktop, Entra public) ; client_id par SetSettings, valeur fournie par Google par BeginSecretImport. C’est le défaut des builds communautaires et de développement, et une option toujours disponible. Le guide, identique en GUI et en TUI, demande le statut de publication « En production » côté Google, le statut « Test » limitant la durée des refresh tokens.

La valeur que Google nomme `client_secret` pour un client Desktop est classée `public_client_credential` : non confidentielle selon la documentation Google des applications installées, jamais preuve d’identité de l’application et hors REQ-DOC-008. En `ovpt_official`, elle est lue depuis le profil de distribution, hors SecretRef ; en `bring_your_own`, elle suit BeginSecretImport comme toute valeur fournie par l’utilisateur. Les refresh tokens sont au coffre OS par SecretRef ; un refresh token renvoyé lors d’un rafraîchissement remplace l’ancien dans le coffre avant l’usage du nouvel access token ; les access tokens restent en mémoire. L’adaptateur Rust utilise les crates `oauth2` et `openidconnect`, versions verrouillées au jalon de build ; aucun SDK fournisseur n’est supposé. Les processus projet ne reçoivent pas ces secrets.

Notion (Q-09) : intégration interne à jeton créé dans Notion par l’utilisateur ou par le propriétaire du workspace, en lecture seule, stocké comme SecretRef. Aucun OAuth public Notion, aucun secret client OVPT ni broker OAuth cloud opéré par OVPT en V1 ; un client secret OAuth commun n’est jamais embarqué dans une application open source. Parcours identique en GUI et en TUI :

1. ouvrir la page des intégrations Notion dans le navigateur système ;
2. créer une intégration interne avec la seule capacité de lecture du contenu, sans insertion, modification, commentaires ni informations utilisateur ;
3. saisir le token par BeginSecretImport (saisie masquée en TUI, fenêtre native hors WebView en GUI) ;
4. TestConnectorAccount appelle `GET /v1/users/me` et crée la ProviderPrincipalBinding de type `integration` (subject_id = identifiant du bot, workspace s’il est retourné) ;
5. l’utilisateur partage les pages par le menu Connexions de Notion ; Explore n’affiche que les ressources partagées et le dit.

Token régénéré pour le même bot : nouvelle `auth_generation`. Autre bot ou autre workspace : AccountIdentityMismatch. Token révoqué ou intégration supprimée : AuthRequired (AC-F013-008). Le serveur MCP hébergé de Notion n’est pas le connecteur nominal ; il peut être ajouté comme adaptateur MCP qualifié sous REQ-DOC-012. Réexamen seulement si Notion documente, pour son API REST, un flux OAuth de client public sans secret (PKCE) : il s’ajouterait comme seconde méthode de connexion, sous les mêmes règles d’identité, sans retirer le jeton d’intégration.

## Synchronisation et échecs

Le Watch local utilise polling avec curseur quand disponible, calendrier borné et backoff. Les webhooks publics sont optionnels : laptop fermé ou non exposé n’empêche pas la reprise par polling. Déduplication par ressource + révision/hash ; une notification invalide d’abord l’index puis déclenche récupération. Un curseur expiré lance une réconciliation limitée aux scopes autorisés.

401 → besoin de réauthentification (AuthRequired). 403 → classé selon la raison documentée par le fournisseur (Drive : `error.errors[].reason`) :

- raison de quota ou de débit (`rateLimitExceeded`, `userRateLimitExceeded`, `dailyLimitExceeded`, `sharingRateLimitExceeded`) → RateLimited, avec attente durable commune ;
- raison de droit (`insufficientFilePermissions`, `domainPolicy`, Notion `restricted_resource`) → AccessRevoked ;
- `appNotAuthorizedToFile` → « non autorisé pour l’application » (AppNotAuthorized), pas une révocation : aucune injection, aucun tombstone ;
- raison inconnue → nouvelles injections suspendues (AccessUnverified), sans tombstone ni purge, jusqu’à une revalidation bornée.

404 → ressource absente ou masquée (ResourceMissingOrHidden), sans prétendre distinguer ce que le fournisseur ne distingue pas. 429/529 ou indisponibilité → attente durable selon headers. Format non supporté → preview partielle signalée. La projection wire de ces erreurs est celle de [F013](../features/F013-document-connectors/interfaces.md). Les opérations de lecture sont rejouables dans leur politique ; les futures écritures documentaires sont des actions distinctes sous autorisation et idempotence.

Une révocation retire immédiatement l’accès nouveau dès qu’observée, invalide les caches de recherche et empêche de construire de nouveaux contextes depuis la copie. L’historique de ce qui a déjà été envoyé reste sous rétention/audit ; il est impossible de « désenvoyer » au fournisseur. Le mode offline, s’il est autorisé, utilise une copie étiquetée dernière autorisation connue avec date d’expiration, jamais une validation fictive en ligne.

Les copies de documents connectés suivent la politique `unleash.retention.default.v1` ([modèle de stockage](../architecture/storage-model.md), Q-12) : conservées tant que l’affectation existe, puis 30 jours après retrait, déconnexion ou révocation observée, sans réinjection entre-temps.

## Contextualisation et extension MCP

Les outils communs de contextualisation sont BrowseResources, SearchResources, PreviewResource, AssignResource, AssignDocumentScope, RemoveAssignment, RefreshResource et ConfigureWatch ; la liste wire normative est celle du [catalogue commun](../architecture/client-operation-catalog.md). Les réponses indiquent source, pagination, couverture, fraîcheur et limites. Les extraits sélectionnés sont liés à leur révision ; l’agent peut demander plus, dans son scope.

MCP ajoute des adaptateurs de ressources/outils sans changer les autorités. Les schémas et annotations du serveur sont données non fiables jusqu’à qualification. Pas de token passthrough arbitraire, de suivi d’URL vers réseaux interdits ni de promotion automatique d’un outil « readOnly » en action réputée sûre.

## Liaison d’identité lors d’une réauthentification — R6

connector_account_id est lié à une `ProviderPrincipalBinding` immuable : provider_kind, issuer, subject_type (user/integration/service), subject_id stable, tenant_or_workspace_id si pertinent, et binding_digest. Ni adresse email affichée ni nom de compte ne sont des identifiants suffisants. L’identité provient du mécanisme décidé par fournisseur : pour Google et Microsoft, l’`id_token` OpenID Connect validé (signature JWKS, `aud`, `nonce`, expiration), avec Google `iss` + `sub` et Microsoft `tid` + `oid`, l’issuer multilocataire étant validé par substitution du `tid` ; pour Notion, l’identifiant du bot retourné par `GET /v1/users/me` (Q-09). La qualification réelle de ces claims relève d’EXT-DOC ; aucun utilisateur n’est déduit arbitrairement d’un token opaque.

Avant d’activer de nouveaux credentials, comparer la liaison attendue à celle observée et contrôler scopes/consentements actuels. Même principal : rotation autorisée sous mandat existant, nouvelle `auth_generation`, revalidation des droits et reprise des curseurs dont le contrat le permet. Principal différent ou inconnu : suspendre Watch/refresh/contextes concernés avec AccountIdentityMismatch ; ne pas remplacer la SecretRef sous le même connector_account_id. Créer un compte distinct, ou une migration d’affectations explicitement autorisée et tracée qui conserve les références historiques et revalide accès/caches. Aucun consentement humain répété n’est imposé si cette opération est déjà mandatée.

Chaque collecte conserve connector_account_id, binding_digest, auth_generation et autorité observée. Une réponse en vol d’une génération révoquée reste preuve historique mais ne réactive pas l’accès courant. Les tokens restent secrets ; l’incident ne publie que références expurgées attendu/observé. La rotation d’un token d’intégration Notion se vérifie par l’identité du bot et du workspace (Q-09), sans présumer qu’elle représente la personne ayant cliqué.
