# R6 — Expert 8 : skills, rôles et contrats

**Revue documentaire indépendante, 16 novembre 2026 ; constats avant correction.** Périmètre : protocole expert, modèle skills/contrats, registries, schémas, F007/F008, discovery F009 et tests associés. Aucun produit exécuté. Les constats portent sur la représentation des contrôles promis, pas sur l’absence assumée de garantie sémantique universelle.

Le bundle immuable, l’entrée unique du rôle, la compatibilité tardive par digests exacts, les versions d’évaluation préépinglées, la révocation et la publication sans juge obligatoire sont correctement séparés. La préférence modèle est traitée par **R6-E05-03**. Le reçu discovery exigeant une invocation même pour une création manuelle est transmis à **E07**, sans doublon ici. Aucune modification des niveaux N/N+1/N+2 ou des permissions n’est proposée.

## R6-E08-01 — Le rapprochement claim/preuve n’a pas de prédicat défini

**Priorité élevée ; manque contractuel confirmé.** Le [catalogue](../../specs/contract-enforcement.md) donne à `claim_evidence_links` un « prédicat de correspondance » et la détection de contradictions. [RecordClaim](../../features/F008-contract-engine/interfaces.md) reçoit une « proposition structurée », sans définir son vocabulaire. Dans [skill.schema](../../schemas/skill.schema.json), les paramètres fermés de ce validateur sont seulement `claim_pointer` et `require_snapshot_match`. Ni une forme normative de proposition, ni sa correspondance avec les observations ne sont fournis dans ce périmètre.

**Scénario et risque.** Un claim affirme des tests réussis ; une observation authentique du même sujet porte un échec. [CT-01](../../testing/contract-tests.md) exige une contradiction. Les identifiants et le snapshot permettent de relier les objets, mais ne spécifient pas le prédicat qui rend leurs valeurs contradictoires. Deux implémentations doivent inventer une convention supplémentaire. Elles peuvent bloquer systématiquement, comparer seulement les liens, ou introduire une interprétation par modèle malgré l’absence de juge obligatoire. Ce sont des risques de conception, pas des comportements observés.

**Correction.** Définir un petit vocabulaire fermé de claims vérifiables, avec sujet exact, prédicat versionné, valeur attendue et types d’observation admissibles. Épingler sa version avec le catalogue. La prose explicative reste distincte ; un claim hors vocabulaire reste jugement ou `unknown`, sans pass de vérité implicite. **Alternative :** limiter explicitement `claim_evidence_links` à l’intégrité des liens ; les validateurs spécialisés établissent les contradictions typées, sans prétendre comparer toute proposition libre.

**Acceptation.** Étendre T-F008-004/007/009 : même claim typé avec observation positive, négative, absente et mauvais sujet ; changer seulement sa formulation ne change pas le verdict. Type inconnu ou prédicat non autorisé refusé/inconnu ; zéro appel juge. Les deux sources restent inspectables. **À confirmer :** vocabulaire V1 et autorité qui publie ses versions.

## R6-E08-02 — La preuve de test ne peut désigner la commande exigée

**Priorité élevée ; décalage confirmé.** [F008 technique](../../features/F008-contract-engine/technical-design.md) permet d’exiger action, digest d’argv, cwd et environnement ; [T-F008-005](../../features/F008-contract-engine/tests.md) doit rejeter leurs différences. Pourtant `test_observation_matches` ne comporte que observation, snapshot attendu, parseur, minimum de cas et exigence de succès. `observed_process_exit` ajoute seulement le code attendu. Aucun paramètre ne désigne la commande attendue ni une spécification d’exécution autoritaire.

**Scénario et risque.** Lorsque le contrat demande une suite précise, deux commandes du même worker utilisent le même snapshot et le même parseur : la suite requise échoue, un filtre bénin réussit avec assez de cas. La sortie référence la seconde observation. Authenticité, fraîcheur, snapshot et nombre de cas peuvent tous correspondre ; ces propriétés ne prouvent pas l’exécution de la commande demandée. Le risque concerne cette exigence explicite, sans imposer des tests à tous les skills.

**Correction.** Ajouter une référence d’exécution attendue résolue depuis le contexte autoritaire de tâche ou un plan versionné : outil/version, argv digest, identité de cwd, environnement pertinent et sélection de suite. Comparer l’observation à cette référence ; une valeur fournie uniquement par la même sortie ne constitue pas l’attendu. **Alternative :** un validateur séparé `observed_command_matches`, fermé, versionné et combiné obligatoirement lorsque le contrat exige une commande particulière.

**Acceptation.** Compléter T-F008-005 avec deux observations authentiques simultanément disponibles ; permuter leur ordre et substituer séparément argv, cwd, environnement et suite. La preuve de la mauvaise commande ne satisfait jamais l’exigence ; attendu absent produit `unknown`, sans juge. **À confirmer :** représentation du plan d’exécution et périmètre d’environnement comparable sans exposer de secrets.

## Vérification effectuée

Validation initiale JSON Schema 2020-12 en mémoire : fixture skill et règles minimales conformes ; ajout de `predicate_ref`, `expected_argv_digest` ou `expected_cwd_pointer` rejeté par `additionalProperties`. Ces noms illustrent les données manquantes, sans imposer leur API. La revue initiale modifiait seulement ce rapport ; les tests produit restent à réaliser.

## Addendum — corrections documentaires intégrées

**E08-01/02 corrigés dans les contrats, qualification produit `not_run`.** [Contract-enforcement](../../specs/contract-enforcement.md) définit `claim_predicate_matches` et `test_execution_matches` au catalogue v2 proposé. Quatre formats V1 nouveaux rendent explicites claim, attendu autoritaire, binding avant effet et projection observée. Les prédicats sont fermés ; qualitatif/nonvérifiable reste unknown. L'attendu vient du plan/contrat/mandat accepté, jamais de la sortie ni d'une sélection favorable après résultat. Les bindings historiques et fixtures v1 sont conservés.

F008 précise critères, refus, reprise et tests ; discovery suit origin/source_refs/DiscoveryUseObservation d'E07. Contrôle documentaire observé : sept schémas valides, quinze fixtures valides, trois états positifs supplémentaires, quinze mutations rejetées, digests/références cohérents et 42 liens internes présents. Aucun moteur Rust, ordre d'effet ou droit réel n'est prouvé.

Raccords remis au coordinateur : index des nouveaux schémas/fixtures, catalogue documentaire, F006/F007 et tests transversaux. Aucun changement des préférences modèle, grants ou niveaux organisationnels.
