# R6 — Expert 10 : sécurité et AI-Guard

**Statut : revue documentaire indépendante ; deux raccords à préciser, aucun test produit exécuté.** Périmètre : protocole Expert 10, frontières de sécurité, GuardPort, connecteurs, navigateur, F017 et tests sécurité. Les limites same-UID, les garanties conditionnelles du navigateur et l’absence de contrat externe AI-Guard sont correctement annoncées. Elles ne constituent pas des anomalies supplémentaires. Les sujets déjà confiés aux autres experts ne sont pas recomptés.

## R6-E10-01 — Une décision `Allow` doit rendre ses contraintes exécutoires

**Priorité haute ; lacune du contrat interne.** [AI-Guard, « Port interne » et « Flux d’exécution »](../../specs/ai-guard.md) définit `Allow(decision_ref, constraints, expiry)`, puis demande de persister les contraintes. La vérification avant effet porte explicitement sur fence, mandat, expiration et droits. [F017/interfaces](../../features/F017-ai-guard-integration/interfaces.md) reprend les contraintes sans définir leur consommation obligatoire. [T-SEC-10](../../testing/security-tests.md) couvre réponse malformée et décision expirée, pas un `Allow` valide dont une obligation reste inappliquée.

**Scénario et risque.** Le double de GuardPort renvoie un `Allow` assorti d’une contrainte non reconnue par l’adaptateur, ou connue mais impossible à imposer par l’exécuteur choisi. Enregistrer cette réponse puis lancer l’action satisferait les étapes explicites actuelles, sans établir que l’autorisation conditionnelle est respectée. Il s’agit d’une ambiguïté permettant une implémentation fautive, pas d’un contournement observé.

**Solution proposée.** Le broker transforme toute décision en obligations typées et versionnées, intersectées avec les limites locales. Chaque obligation possède un composant responsable, une frontière et une preuve d’applicabilité ; celles qui doivent durer sont imposées pendant l’exécution. Une obligation inconnue, contradictoire ou non applicable bloque avant dispatch et résolution de secrets. Le journal conserve décision, profil de mapping, obligations appliquées et cause du refus, sans payload sensible. Un éventuel repli optionnel conserve sa politique préautorisée et sa propre décision : il ne consiste pas à ignorer une partie du `Allow`.

**Alternative.** N’accepter dans le premier profil d’intégration que des décisions sans obligations additionnelles ; toute autre réponse reste non prise en charge jusqu’à qualification du mapping. Cela restreint un profil déclaré, sans supprimer l’intégration V1 ni attribuer une API à AI-Guard.

**AC.** Étendre AC-F017-005/008/010 et T-SEC-10 : contrainte inconnue, connue mais non imposable, contradictoire et entièrement applicable. Dans les trois premiers cas, oracle indépendant attestant zéro dispatch et aucune libération de secret ; dans le dernier, preuve du contrôle effectif et corrélation avec la décision. Tester aussi la perte de capacité entre évaluation et lancement.

**Non vérifié.** Forme et sémantique des contraintes du véritable AI-Guard, capacités de ses exécuteurs et profil de correspondance : gate [Q-04](../../brief/open-questions.md), puis recette réelle F017. Aucune structure externe n’est présupposée ici.

## R6-E10-02 — Réauthentifier un connecteur ne doit pas remplacer implicitement son identité

**Priorité moyenne ; frontière compte/source.** [Connecteurs](../../specs/connectors.md) inclut `connector_account_id` dans l’identité des ressources et reprend après `401` par réauthentification. [F013/conception](../../features/F013-document-connectors/technical-design.md) stocke SecretRef, tenant/workspace et périmètre consenti ; [ses interfaces](../../features/F013-document-connectors/interfaces.md) conservent affectations et curseurs par compte. Aucun de ces contrats ne précise la comparaison obligatoire du principal fournisseur attendu avec celui de la nouvelle connexion. AC-F013-007 vérifie la transaction OAuth, sans scénario de compte différent.

**Scénario et risque.** Le compte A expire. Le navigateur reconnecte B, éventuellement dans le même tenant, et une implémentation remplace la SecretRef sous l’ancien `account_id`. Watch, cache et affectations peuvent alors reprendre avec des droits de B tout en attribuant les nouvelles observations à A. Le risque concerne notamment une ressource partagée accessible aux deux comptes ; il ne suppose pas que le fournisseur réutilise un identifiant de document.

**Solution proposée.** Le gestionnaire de comptes lie l’identité Unleash à une identité fournisseur qualifiée et stable, incluant tenant/workspace lorsque pertinent. L’adaptateur vérifie cette liaison avant activation des nouveaux credentials. Un changement crée un compte distinct ou une migration explicitement autorisée ; il ne réactive pas les anciennes autorisations de cache. Une génération d’authentification rattache collectes, droits observés et curseurs. Un mismatch suspend la reprise et laisse un événement expurgé identifiant les références attendue/observée. Une rotation normale de token pour le même principal reste possible automatiquement.

**Alternative.** Créer systématiquement une connexion isolée à chaque réauthentification, puis transférer les affectations après vérification d’identité et de portée selon le mandat existant ; aucune validation humaine universelle n’est nécessaire.

**AC.** Compléter T-F013-007/009 : reconnexion de A, substitution par B dans le même tenant puis dans un autre, et remplacement d’un token Notion. Aucun Watch ni contexte ne reprend sous une identité mal attribuée ; les preuves historiques conservent leur compte et les nouveaux accès leur autorité effective.

**Non vérifié.** Mécanisme officiel d’identification disponible pour chaque fournisseur et catégorie de compte ; à qualifier avec F013/Q-08 avant recette, sans présumer un endpoint ou un format de token.
