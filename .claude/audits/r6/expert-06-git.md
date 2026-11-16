# R6 — Expert 06 — Git et intégration

**Date :** 16 novembre 2026. **Périmètre :** conception proposée, sans implémentation produit. Lecture du mandat Git, de `specs/git-integration.md`, `architecture/git-coordination.md`, des sept documents F010, des contrats/preuves, de F009 et des scénarios Git transversaux. Les deux constats ci-dessous demandent une précision normative avant implémentation ; aucun exploit produit ni test Rust exécuté n'est revendiqué.

## R6-E06-01 — P1 — L'identité de cible omet les références symboliques

**Constat confirmé.** [F010/interfaces](../../features/F010-git-coordination/interfaces.md), « Allocations et types », définit `TargetRef` comme nom complet normalisé. [La spécification](../../specs/git-integration.md), « Préparation et publication », impose CAS et cible non checkout, mais ni politique de références symboliques ni publication sans déréférencement. La normalisation des symlinks de chemins, dans l'architecture, ne définit pas celle des symrefs Git. Le manuel [git-update-ref](https://git-scm.com/docs/git-update-ref), « DESCRIPTION », confirme que la forme à trois arguments suit les symrefs ; `--no-deref` change ce comportement. Source consultée le 16 novembre 2026.

**Scénario et risque.** Le mandat autorise `refs/heads/integration`, symref vers `refs/heads/main`. Les deux noms résolvent oldOID ; seul `main` est checkout et hors mandat. Une implémentation contrôlant les checkouts/ACL par le nom demandé puis exécutant le CAS proposé peut déplacer `main`, malgré un oldOID exact. Le scénario existe dès l'adoption, sans programme hostile concurrent. Une substitution après contrôle constitue une variante.

**Correction proposée.** Exiger une cible directe et un namespace autorisé ; conserver type et identité de ref dans l'intention. Qualifier la publication, la création et la suppression des refs gérées sans déréférencement, y compris les refs de conservation. Une symref observée bloque l'admission ; une substitution extérieure détectée entraîne réconciliation/quarantaine. Ne pas promettre une transaction avec les checkouts externes.

**Alternative et objection.** Supporter les alias exige chaîne de résolution, autorisation des référents et contrôle des checkouts réels, avec préconditions qualifiées. Le périmètre coopératif déjà déclaré ne résout pas une symref préexistante ; refuser cet état suffit en V1.

**Tests attendus.** Compléter T-F010-001/006/011 : symref préexistante vers cible checkout/hors scope, alias pendant la fenêtre avant CAS, symref de conservation avant nettoyage. Oracle : aucun référent non autorisé déplacé ou supprimé, refus/incertitude explicites. Ces tests restent à réaliser.

## R6-E06-02 — P1 — Un OID épinglé ne neutralise pas `refs/replace`

**Constat confirmé.** [F010/conception](../../features/F010-git-coordination/technical-design.md), « Candidat et publication », fonde candidat et preuves sur les OID ; [F009](../../specs/shared-knowledge.md), « Identité de source », exige lecture sans transformation implicite. Aucun profil Git commun ne traite les replacement refs. Le manuel [git-replace](https://git-scm.com/docs/git-replace), « DESCRIPTION », confirme leur utilisation par défaut pour la plupart des commandes, y compris la lecture d'un commit par OID, et documente `--no-replace-objects`/`GIT_NO_REPLACE_OBJECTS`. Source consultée le 16 novembre 2026.

**Scénario et risque.** Un dépôt adopté contient une substitution C→D. Préparation, capture ou oracle d'ancestry interroge C et reçoit contenu/parents de D. La substitution disparaît ensuite ; C et la ref cible n'ont pas changé, mais les observations ne portent plus le même graphe ou contenu. Une preuve étiquetée seulement C peut être réutilisée à tort ; une réconciliation peut attribuer une inclusion au graphe substitué. Ce risque ne suppose ni collision de hash ni changement de branche.

**Correction proposée.** Définir un profil de lecture brute cohérent pour inspection, préparation, captures, validations et réconciliation : replacements désactivés par option/environnement contrôlé, configuration effective enregistrée, absence de repli implicite. Signaler les substitutions à l'adoption ; les observations antérieures dont le profil est inconnu deviennent à revalider, avec invalidation des index/reçus concernés.

**Alternative et objection.** Accepter les substitutions nécessite d'épingler leur mapping et leur politique dans l'identité de snapshot et toutes les preuves ; c'est plus complexe. Les objets physiques restent immuables, mais cela ne garantit pas que les commandes ordinaires retournent l'objet demandé sans remplacement.

**Tests attendus.** Étendre T-F010-004/008/009 et CT-03 : remplacement de commit et blob, ajout/suppression entre capture et publication, oracle d'ancestry avec remplacement. Les octets, parents et preuves doivent rester liés au profil brut, ou l'opération bloquer explicitement.

## Conclusions bornées

Les missions sans dépôt R3 n'affaiblissent pas les obligations Git des missions modifiant un repository. Écrivain unique/worktrees enfants, CAS exact, lease en réconciliation, validation sur candidat, cible checkout, conservation des candidats et fraîcheur des index sont déjà couverts. Les limites same-UID, ABA et conflits sémantiques/non textuels sont déclarées ; elles ne sont pas recomptées comme findings. Aucun troisième écart suffisamment établi.

**État :** deux corrections documentaires proposées, en attente de relecture ; qualification produit future.
