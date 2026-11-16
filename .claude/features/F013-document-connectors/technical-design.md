# F013 — conception technique Rust

## Adaptateurs et modèle commun

DocumentConnector est un port Rust à capacités explicites : browse, search, metadata, fetch, changes et preview_formats. DriveConnector, GraphConnector et NotionConnector convertissent des réponses HTTP vers ResourceRef, ResourceRevision et ResourceSnapshot du [modèle commun](../../specs/connectors.md). Les IDs restent opaques et les curseurs retournés sont retransmis tels quels. Les JSON fournisseurs tolèrent les champs additionnels ; les commandes internes restent strictement validées.

L’index textuel local ne reçoit que des ressources sélectionnées ou dans un scope autorisé. Recherche distante et recherche locale sont distinguées dans le résultat. Le plan d’ingestion comporte limites de profondeur, documents, octets et durée ; un dossier immense ne monopolise pas le scheduler.

## Ingestion et fraîcheur

Chaque récupération produit une copie adressée par hash et une révision observée. Pour une page paginée ou des blocs modifiés pendant lecture, conserver capture_consistency et réessayer de façon bornée ; ne pas annoncer un snapshot fournisseur atomique inexistant. Les formats texte/Markdown/JSON et exports documentaires supportés sont privilégiés, les binaires non parsables gardent une référence et une limite visible.

Le Watch persiste les curseurs et son dernier passage. Après indisponibilité, il reprend et réconcilie les tombstones ; un curseur invalide reconstruit seulement le scope sélectionné. Une notification ne remplace jamais la lecture de contenu. Les changements invalident le cache avant d’être proposés pour une prochaine invocation.

## Authentification et droits

Les flux Google/Microsoft utilisent des clients publics d’application native (Q-08) : navigateur système, PKCE S256, `state` et `nonce` de 256 bits, écouteur loopback `127.0.0.1` à port éphémère, à usage unique et expiré après 10 minutes. `client_profile` vaut `ovpt_official`, dont les identifiants sont lus depuis le profil de distribution des seuls artefacts officiels signés, ou `bring_your_own`, défaut des builds communautaires et de développement. Le `public_client_credential` de Google n’est ni un secret confidentiel ni une preuve d’identité. Scopes V1 : Google `openid` + `drive.readonly` ; Microsoft `openid`, `offline_access`, `Files.Read`, puis `Files.Read.All` incrémental. L’id_token est validé (JWKS, `aud`, `nonce`, expiration) avant toute ProviderPrincipalBinding ; les crates `oauth2` et `openidconnect` sont verrouillées au jalon de build. Le registre account stocke une SecretRef, tenant/workspace et périmètre consenti. Notion utilise l’intégration interne à jeton de l’utilisateur, en lecture seule, sans secret OAuth OVPT embarqué ; l’OAuth public Notion n’est pas fourni en V1 (Q-09) ; un flux public sans secret documenté par Notion s’ajouterait comme méthode supplémentaire. Détail : [connecteurs](../../specs/connectors.md).

Les permissions fournisseur sont un plafond, puis Unleash applique scope projet et assignation. Un cache ne prolonge pas indéfiniment un droit révoqué. Après perte d’accès observée, plus de nouvelle injection dans un modèle ; l’historique de transmission reste exact sous sa politique de conservation.

## Exploitation et erreurs

Les connecteurs ont des files bornées par compte et backoff. Un appel de lecture long ne réveille pas un manager à chaque page. Seules erreur bloquante, modification pertinente à mission active ou décision de remplacement créent une demande de revue. Les erreurs AuthRequired, AccessRevoked, AccessUnverified, AppNotAuthorized, ResourceMissingOrHidden, CursorExpired, UnsupportedFormat, RateLimited et AccountIdentityMismatch sont distinctes, avec la projection wire de [interfaces](interfaces.md) ; PartialCapture est une couverture partielle signalée dans le résultat, pas une erreur. Un 403 est classé par la raison documentée du fournisseur : quota ou débit → RateLimited et attente durable commune ; droit → AccessRevoked ; raison inconnue → AccessUnverified, injections suspendues sans tombstone ni purge jusqu’à revalidation bornée.

L’extension MCP conserve les mêmes contrôles de cible réseau, limites, provenance et classification d’outils. Le serveur n’est pas autorité sur ses propres permissions. Les écritures distantes éventuelles sont des actions séparées du connecteur de contexte, avec intention et reprise adaptées.

## Ordre interne de construction

Figer les DTO puis créer les doubles déterministes ; implémenter la voie nominale ; ajouter persistance et idempotence ; injecter les pannes aux frontières ; qualifier les adaptateurs réels ; raccorder clients et rapports. Les interfaces de feature sont proposées, et les APIs de bibliothèques doivent correspondre aux versions verrouillées au jalon de build.

Ne pas ajouter de service séparé sans besoin démontré. Le daemon possède la coordination ; les adaptateurs ne créent ni workflow parallèle ni gestionnaire de budget caché.

## Réauthentification R6

Le gestionnaire de comptes vérifie ProviderPrincipalBinding avant activation de SecretRef ; connector_account_id ne change jamais de principal par simple reconnexion. Même identité → auth_generation nouvelle et droits revalidés ; mismatch/inconnu → reprise suspendue, nouvelle connexion ou migration explicitement autorisée. Les tâches en vol ne peuvent attribuer les données d’une nouvelle identité à l’ancien compte. Un refresh token renvoyé au rafraîchissement remplace atomiquement l’ancien dans le coffre avant l’usage du nouvel access token ; les access tokens restent en mémoire. Voir [connectors](../../specs/connectors.md).
