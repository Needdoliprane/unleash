# Qualification sécurité : mécanisme, frontière et preuve

Tests futurs en Rust ; aucun scan ni exploit produit exécuté pendant cette conception. Chaque cas utilise des secrets canaris, comptes de test et artefacts expurgés. Les garanties dépendent du modèle de menace arrêté par Q-05 ([frontières de sécurité](../architecture/security-boundaries.md) ; analyse historique dans l’[audit menaces](../audits/security-threat-model.md)), de l’`isolation_mode` (`standard` par défaut, `strict` explicite) et de la frontière AI-Guard retenue par Q-04 ([ai-guard](../specs/ai-guard.md)).

| Test | Critères liés | Attaque / panne | Oracle et preuve attendue |
| --- | --- | --- | --- |
| T-SEC-01 | AC-F017-001/004, AC-F001-004 | Faux actor_ref admin, socket accessible hors répertoire prévu, credential worker sur canal admin | Identité résolue serveur, refus avant lecture/mutation, journal sans secret ; EV-SEC-01 |
| T-SEC-02 | AC-F017-003, AC-F012-009 | Secret canari demandé dans prompt, log, rapport et export | Valeur absente des destinations interdites ; SecretRef seulement ; limites transformation arbitraire documentées ; EV-SEC-02 |
| T-SEC-03 | AC-F017-005 | Traversal, symlink swap, chemin hors worktree, écriture hors scope | Contrôle au point d'effet sur cible réelle ; refus ou mécanisme OS qualifié ; EV-SEC-03 |
| T-SEC-04 | AC-F017-005 | Shell/build hook/proc-macro tente accès socket/clé/réseau hors mandat | Mode standard (défaut) montre ses limites et s’affiche non confiné ; mode strict bloque l’accès au point d’effet, ou refuse l’action avant exécution (IsolationUnavailable) si le confinement qualifié est absent ; EV-SEC-04 |
| T-SEC-05 | AC-F017-007, AC-F008-003 | Révocation entre acceptation et dispatch, vieille capability/fence après reassignment | Aucune nouvelle action autorisée par ancienne identité ; effets déjà dispatchés restent suivis ; EV-SEC-05 |
| T-SEC-06 | AC-F017-003/004, AC-F013-007 | OAuth state faux, callback rejoué, code PKCE invalide, redirect inattendu, nonce ou aud de l’id_token invalide, tentative expirée après 10 min | Échange refusé, token non conservé, navigateur externe et callback borné ; EV-SEC-06 |
| T-SEC-07 | AC-F009-004, AC-F013-009 | Retrait ACL, cache chaud, compteurs de recherche et contexte reconstitué | Aucun nouveau contenu/nom/compteur interdit ; sources déjà transmises marquées irréversibles ; EV-SEC-07 |
| T-SEC-08 | AC-F017-006, AC-F009-004 | Prompt injection dans README, document, stdout ou annotations MCP | Texte ne crée aucune permission ; commande hors grants refusée par broker ; EV-SEC-08 |
| T-SEC-09 | AC-F017-011, AC-F016-008 | Purge historique, rapports conservés, export ancien et backup | Suppression dans périmètre annoncé, tombstones, sauvegardes gérées concernées listées (octets au plus 35 jours), limites APFS/Time Machine, exports et tiers explicites ; EV-SEC-09 |
| T-SEC-10 | AC-F017-008/009/010/012 | AI-Guard timeout/identité inconnue/retour malformé/décision expirée ; repli guard_optional face à un Deny ou à une obligation non imposable | Fail-closed pour opérations qui exigent décision ; journal cause et issue ; EV-SEC-10 |
| T-SEC-11 | AC-F018-005/007 | Restaurer ancien snapshot avec ancien host et token valides auparavant | Nouvelle authority_generation, refus anciennes lignées, aucune admission avant neutralisation ; EV-SEC-11 |
| T-SEC-12 | AC-F007-005/008/009 | Skill importé contient script, référence réseau, élévation permission ou règle retirée | Donnée non exécutable, profil fermé, grants intersection, publication/binding refusé ; EV-SEC-12 |

## Mise en place et observations indépendantes

Le banc de test exécute le daemon sous utilisateur de test, crée une autre identité locale quand la plateforme le permet, et capture uniquement les destinations de réseau/fichier prévues. Il vérifie l'absence d'effet avec un oracle distinct du rapport LLM. Une simple réponse « denied » ne prouve pas qu'aucun fichier n'a été lu avant le refus.

La campagne same-UID utilise explicitement un shell non confiné pour montrer la limite du mode standard. Il serait trompeur d'annoncer cette campagne passée parce que le chemin API typé refuse l'action alors que le shell peut lire le même répertoire. Pour le mode strict, archiver le mécanisme OS effectif (profil Seatbelt, ou bubblewrap/seccomp/Landlock), version/ABI, configuration, proxy d’egress, périmètre, contournements tentés et canaris Keychain, data_root et socket. Si la frontière ne peut être qualifiée, l'acceptation attend IsolationUnavailable avant exécution.

## Secrets et confidentialité

Le test différencie secret structuré, sortie arbitraire et transformation du secret. La redaction déterministe de valeurs connues ne garantit pas la détection de tout encodage ou exfiltration. Le code de transport reçoit le secret au dernier moment et aucun artefact de test ne garde de vraie clé. Les connecteurs et modèles sont des destinations distinctes, soumises à politiques de données et rétention propres.

## Preuves externes

T-SEC-10 doit être exécuté contre un simulateur et contre le contrat réel AI-Guard, avec résultats séparés. OAuth doit être testé avec les clients enregistrés des profils `ovpt_official` et `bring_your_own` et les scopes effectivement consentis (EXT-DOC). Les permissions d'un compte de développement ne prouvent pas celles d'un utilisateur distribué. Les résultats marquent version, compte/catégorie sans secret, limites et autorité ayant accepté le risque résiduel.

## Automatisations et modèles locaux — tests négatifs R3–R5

| Test | Critères liés | Contre-exemple | Oracle et preuve attendue |
| --- | --- | --- | --- |
| T-SEC-13 | AC-F019-008/014, AC-F017-005 | Script autorisé sur un domaine tente paiement sur ce même domaine ; outil générique exécute du code arbitraire | Le profil médié bloque au point d’effet ou refuse la garantie si la frontière est insuffisante ; un filtre d’origines seul ne constitue pas une réussite ; EV-SEC-13 |
| T-SEC-14 | AC-F019-006, AC-F012-018 | Driver, rapport, manager, MCP sampling ou embedding demande un modèle durant forbidden | Refus avant dispatch, compteur de tentatives conservé, instrumentation indépendante ; aucune destination locale ou distante cachée ; EV-SEC-14 |
| T-SEC-15 | AC-F012-015 | Endpoint localhost relaie vers cloud ou machine non autorisée, redirection modifie destination | Contrôle de placement/transport refuse avant contexte ; incapacité de vérifier reste unknown, pas local ; EV-SEC-15 |
| T-SEC-16 | AC-F019-009/010/011 | Injection dans DOM/logs prescrit nouvel outil ; clic valide mauvais produit ; receipt perdu | Autorité inchangée, postconditions fausses visibles, aucun retry aveugle, old script figé ; EV-SEC-16 |
| T-SEC-17 | AC-F014-015, AC-F015-017, AC-F004-012 | Template novice demande droits élargis, double Apply ou projet différent après preview | Diff de mandat visible, intersection droits, conflit de révision et idempotence ; aucun effet à l’aperçu ; EV-SEC-17 |

Les fixtures utilisent un site de test contrôlé, des fournisseurs factices puis les profils réels qualifiés. Les données de connexion navigateur, cookies, tokens et secrets restent hors des bundles exportés. Un test de refus API ne prouve pas qu’un script libre ne peut contourner la frontière : le test instrumente le processus et sa sandbox réelle. Le modèle qui lit des logs n’est jamais l’autorité qui autorise ses propres outils.

## Complément T-SEC-10 — R6

Une réponse Allow syntaxiquement valide doit encore appliquer toutes ses obligations. Tester inconnue, conflit, impossibilité d’enforcement et capacité perdue avant/pendant effet. L’oracle extérieur au mapper mesure dispatch/secrets/egress et corrèle les obligations appliquées. Une réponse archivée seule ne valide pas ce test. Sous le profil initial `obligations=none_accepted`, l’obligation entièrement applicable est elle aussi refusée ; elle ne devient un cas accepté qu’avec un `guard_mapping_profile` qualifié. La forme externe demeure à qualifier sur le contrat AI-Guard réel (EXT-GUARD).
