# Sources sécurité et connecteurs — relevé du 16 novembre 2026

Les conclusions d’architecture sont arrêtées par les ADR et contrats du dossier (arbitrages R7). Les documents ci-dessous établissent des mécanismes externes, pas la conformité d’un produit qui n’existe pas encore.

| Source primaire | Fait retenu | Limite / conséquence |
| --- | --- | --- |
| [RFC 8252, octobre 2017](https://www.rfc-editor.org/rfc/rfc8252) | Applications natives : navigateur externe et flux adaptés aux clients publics | Ne suppose pas qu’un secret distribué reste confidentiel |
| [RFC 7636, septembre 2015](https://www.rfc-editor.org/rfc/rfc7636) | PKCE lie l’échange au client initiateur | state, redirect et stockage doivent aussi être corrects |
| [Google OAuth desktop](https://developers.google.com/identity/protocols/oauth2/native-app) | PKCE et callback loopback documentés | Politique de diffusion décidée (Q-08, [connecteurs](../specs/connectors.md)) ; enregistrement OVPT et vérification Google à obtenir |
| [Drive scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth) | drive.file couvre les fichiers créés ou ouverts avec l’application, ou choisis par l’utilisateur dans le Google Picker ou le sélecteur de l’application ; scopes larges soumis aux règles Google | Ne donne pas automatiquement un browse global du Drive ; connaître un ID ou un lien ne vaut pas autorisation |
| [Google Picker](https://developers.google.com/workspace/drive/picker/guides/overview) | Desktop/mobile : navigateur système, `trigger_onepick=true`, redirect URI, drive.file seul non combinable, pas de webview intégrée | Écarté de la V1 (Q-08 : `openid` + `drive.readonly`, `drive.file` non utilisé) : aucun picker, ni dans la WebView ni par le parcours desktop ; réexamen seulement par Q-08 ([connecteurs](../specs/connectors.md)) |
| [Drive changes](https://developers.google.com/workspace/drive/api/guides/manage-changes) | Cursor de changements, notifications ne contiennent pas le contenu | Polling et réconciliation restent nécessaires |
| [Drive shared changes](https://developers.google.com/workspace/drive/api/guides/about-changes) | Journaux utilisateur et shared drive peuvent être distincts | Ne pas réduire un drive partagé au journal utilisateur |
| [Microsoft auth code](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow) | Auth code + PKCE pour clients natifs | Tenant et consentements dépendent du compte |
| [Graph Selected](https://learn.microsoft.com/en-us/graph/permissions-selected-overview) | Permissions choisies + grants ressources explicites | Pas un équivalent universel du picker grand public |
| [Graph permissions](https://learn.microsoft.com/en-us/graph/permissions-reference) | Files.ReadWrite.Selected historique n’est pas destiné aux appels Graph directs | Ne pas l’utiliser pour contourner le consentement |
| [DriveItem delta](https://learn.microsoft.com/en-us/onedrive/developer/rest-api/api/driveitem_delta) | nextLink/deltaLink opaques et tombstones | Disponibilité endpoint/compte qualifiée |
| [Notion autorisation](https://developers.notion.com/guides/authorization/overview) | Tokens d’intégration et OAuth ; échange OAuth client_secret documenté | Pas de secret OVPT global incorporé au binaire ; V1 : jeton d’intégration interne de l’utilisateur, lecture seule (Q-09) |
| [Notion recherche](https://developers.notion.com/reference/search-optimizations-and-limitations) | Recherche sur titres, index non exhaustif et asynchrone | Pas de promesse full-text globale du workspace |
| [Notion versions](https://developers.notion.com/reference/versioning) | Header version ; exemple actuel 2026-03-11 ; IDs distincts des URLs humaines | Champs additionnels tolérés côté fournisseur |
| [Notion limites](https://developers.notion.com/reference/request-limits) | Limites connexion/workspace et Retry-After, parfois 529 | Pas de constante universelle « 3 requêtes/s » |
| [Notion webhooks](https://developers.notion.com/reference/webhooks) | Endpoint public HTTPS requis ; signature HMAC disponible | Un laptop localhost n’est pas un récepteur public |
| [Apple Keychain](https://developer.apple.com/documentation/security/keychain-services.md) | Service de stockage de secrets du système | Accès daemon et entitlements à qualifier |
| [Apple App Sandbox](https://developer.apple.com/documentation/security/app-sandbox.md) | Confinement et droits déclarés par application | Ce n’est pas une garantie automatique pour tout outil de build |
| [Linux Landlock](https://docs.kernel.org/userspace-api/landlock.html) | Réduction de droits sans privilèges, dépendante des ABI | Tester les droits réellement supportés ; pas simuler capacité manquante |
| [keyring 4.2.0](https://docs.rs/keyring/4.2.0/keyring/) | Séparation keyring-core/stores recommandée pour sélection explicite | Ne pas recopier une API d’une ancienne version |
| [MCP tools 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) | Schémas d’outils et annotations non fiables hors serveur de confiance | Reclassification locale des effets/permissions |
| [MCP sécurité](https://modelcontextprotocol.io/docs/2025-11-25/tutorials/security/security_best_practices) | Confused deputy, SSRF, scopes et sessions | Le protocole n’est pas une sandbox |
| [Open Source Definition](https://opensource.org/osd) | La distribution open source comporte des droits définis | Licence Apache-2.0 et politique de marque décidées (Q-06, [ADR-14](../architecture/adr/ADR-14-local-data-oss.md)) |

Les pages Apple ont été lues via leurs liens Markdown officiels. Aucune interface AI-Guard n’a été fournie ni inventée.

## Sources ajoutées en R7, à relire au gate

Ces lignes n’ont pas été relues lors de la révision R7 ; elles sont datées à leur lecture au gate indiqué. Une source sans URL a une URL officielle à relever au même moment ; aucune n’est inventée.

| Source primaire | Point à vérifier | Décision et gate |
| --- | --- | --- |
| Page de manuel `sandbox-exec(1)` de chaque version macOS qualifiée, lue localement | Présence, statut (dépréciation) et profils Seatbelt applicables | Q-05 ; capacité à qualifier par version d’OS |
| [bubblewrap](https://github.com/containers/bubblewrap) | Espaces de noms utilisateur, `--unshare-net`, montages en lecture seule | Q-05 ; capacité à qualifier par version et distribution |
| [seccomp(2)](https://man7.org/linux/man-pages/man2/seccomp.2.html) | Filtrage des appels système du processus confiné | Q-05 ; capacité à qualifier par noyau |
| ABI Landlock (ligne [Linux Landlock](https://docs.kernel.org/userspace-api/landlock.html) ci-dessus) | Version d’ABI et droits réellement couverts | Q-05 ; capacité à qualifier par noyau, jamais simulée |
| [Google OAuth desktop](https://developers.google.com/identity/protocols/oauth2/native-app) | Le secret d’un client installé n’y est pas traité comme un secret | Q-08 ; EXT-DOC |
| [Google OAuth 2.0](https://developers.google.com/identity/protocols/oauth2) | Expiration à 7 jours des refresh tokens d’un projet au statut de publication « Test » | Q-08 ; EXT-DOC |
| Vérification des scopes restreints Google (URL officielle à relever) | Exigences de vérification et d’évaluation de sécurité des scopes Drive retenus | Q-08 ; EXT-DOC |
| [Vérification d’éditeur Microsoft](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview) | Conditions de vérification de l’éditeur d’une inscription Entra | Q-08 ; EXT-DOC |
| [Redirect URI Entra](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url) | Restrictions `localhost`/`127.0.0.1` et port variable | Q-08 ; EXT-DOC |
| [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) | Contributions (§5) et marque non concédée (§6) | Q-06 ; EXT-OSS |

