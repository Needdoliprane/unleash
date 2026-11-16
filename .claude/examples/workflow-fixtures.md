# Fixtures de workflows, skills et rôles

Ces JSON sont des définitions de conception, pas des runs exécutés. Les identifiants `fixture-*`, `repo-ovpt-fixture` et `model-policy.default-fixture` désignent des données de test. La politique modèle ne prétend pas nommer un modèle externe disponible ; le routeur futur devra résoudre une configuration autorisée et compatible. Les noms `tool.*` sont un catalogue interne proposé, pas des API existantes de fournisseurs.

| Fichier | Schéma | Usage |
|---|---|---|
| [skill.json](skill.json) | [skill.schema.json](../schemas/skill.schema.json) | Compétence multilangage de modification, avec preuve d'artefact et découverte |
| [integration-skill.json](integration-skill.json) | Même schéma skill | Compétence spécialisée d'intégration, preuve Git requise |
| [leader-skill.json](leader-skill.json) | Même schéma skill | Décision N+1 sur evidence bundle ; enveloppe de décision validée par F004 |
| [role.json](role.json) | [role.schema.json](../schemas/role.schema.json) | Rôle N réalisant une modification |
| [integration-role.json](integration-role.json) | Même schéma rôle | Rôle N intégrateur, créable directement par N+2 avec mission |
| [leader-role.json](leader-role.json) | Même schéma rôle | Rôle N+1 pour jugement attribué |
| [feature-workflow.json](feature-workflow.json) | [workflow.schema.json](../schemas/workflow.schema.json) | Branches parallèles, join all, jugement puis merge, intégrateur en slot tardif |
| [merge-only-workflow.json](merge-only-workflow.json) | Même schéma workflow | Développement puis merge, sans test/audit métier ajouté |
| [contract-profile.json](contract-profile.json) | [contract-profile.schema.json](../schemas/contract-profile.schema.json) | Digests exacts input/output et règles de l'intégrateur, versions d'évaluation épinglées |

## Prérequis de simulation ou d'intégration future

Charger les trois skills puis les trois rôles dans un registre de test. Les contrats cités par les rôles sont les contrats inline de leurs skills. Fournir une politique de modèle fictive compatible, un catalogue d'outils fictifs et grants limités au repository de test. Définir les scopes de mission `api`, `ui`, `mission`, `implementation`, `integration` avec identités de ressources distinctes. Ces scopes sont des clés d'affectation `scope_key`, résolues contre le mandat selon le [DSL](../architecture/workflow-dsl.md#nœuds-et-résultats) : les chaînes dans la définition ne créent pas de permissions.

Charger le profil contractuel exact de l'intégrateur avant BindRole. Ses digests sont calculés sur le sous-ensemble des fixtures compatible JCS (clés ASCII, entiers sûrs, aucun flottant). Chaque rôle désigne son `entry_skill` membre ; les contrats des tâches sont ceux de cette entrée. Le test documentaire compare les vrais objets aux digests du profil et ne se contente pas de croire les chaînes déclarées.

Le workflow de feature utilise un seul rôle développeur pour deux agents séparés. Le rôle intégrateur doit être lié explicitement au slot tardif avant son activation, puis rester immuable. Le contexte d'intégration (racine `/context` de la projection d'évaluation, résolue par le serveur depuis le manifest épinglé, jamais fournie par le modèle ni par le client ; voir [contract-enforcement](../specs/contract-enforcement.md)) fournit `source_commits` résolus depuis les missions sources ; les chaînes de mission du fichier ne sont pas des commits. La valeur `review_target: "build"` de la tâche leader est une donnée littérale : le harness construit l'evidence bundle du bloc `build`. Les instructions du leader exigent un evidence bundle construit par le harness ; l'output seul ne crée pas une preuve.

## Sorties des blocs

Une tâche expose l'output conforme du skill. Une séquence exporte les outputs de son dernier enfant ; un choice ceux de sa branche choisie. Les fixtures utilisent donc `/results/delivery/outputs/integrated` comme done. L'observation Git requise par le contrat de l'intégrateur empêche qu'un simple booléen déclaré soit une preuve suffisante. Les missions d'entrée, snapshots et artefacts fictifs ne doivent jamais être affichés comme observations réelles.

## Contrôle documentaire effectué

Les quatre schémas ont été vérifiés comme JSON Schema Draft 2020-12 et les neuf fichiers ont été validés contre leur schéma. Une vérification de références entre fixtures, de types de tâches, de digests de profil et de bornes complète le contrôle documentaire. Le compilateur Rust, le runtime, les permissions et les effets réels restent à construire et à tester ; cette validation ne constitue pas leur acceptation.

## Fixtures R3–R5 : actions et supervision

[automation-reconciliation-workflow.json](automation-reconciliation-workflow.json) utilise le schéma v1.1 avec bindings vides, quatre budgets modèle nuls et une action de rapprochement. La politique forbidden interdit récursivement les tâches agentes ; elle ne prouve pas à elle seule que le driver implémenté ne contacte pas un modèle. Le catalogue et la frontière d’exécution doivent être qualifiés.

[automation-supervised-workflow.json](automation-supervised-workflow.json) observe un bloc, appelle une seule tâche de supervision, puis rend le rapport uniquement si décision et observations concordent. Le [rôle](automation-supervisor-role.json) et le [skill](automation-supervisor-skill.json) associés utilisent une portée projet sans dépôt obligatoire. Leurs outils et model-policy sont des dépendances de fixture proposées, pas des services existants. Le catalogue test doit les fournir avant toute validation sémantique exécutable. Les autres décisions terminent cette fixture par un état demandant une suite ; un autre workflow peut publier les branches reconcile/correct/escalate autorisées. Aucun on_error implicite n’est ajouté.

[automation-observation.json](automation-observation.json) illustre le contrat typé du bloc. effect_state none signifie absence de mutation externe dans le bloc ; known renvoie aux issues enregistrées de toutes les mutations concernées ; unknown conserve au moins une issue non déterminée. known ne signifie pas succès métier. La capture peut être terminée tandis que le métier reste en échec. Le harness vérifie la provenance des références, pas seulement la forme JSON.

Les noms demo.* sont des opérations documentaires non implémentées, aux contrats de test à enregistrer ; aucun sélecteur marchand, identifiant de compte ou fichier utilisateur n’est présent. Les fixtures v1 initiales restent inchangées. Les contraintes de références résolues, producteur dominant, droits, effets et quiescence nécessitent un compilateur sémantique futur.

## Fixture R6 : itération v1.2

[automation-rows-workflow.json](automation-rows-workflow.json) utilise le schéma [v1.2](../schemas/workflow-v1.2.schema.json) : `for_each` sur `/input/rows` (max_items 10 000, batch_size 1 000), body `fixture.observe-row` et after_batch `fixture.record-batch` machine-only sous `forbidden`, bindings vides et budgets modèle nuls. Borne statique 1 + 10 000×1 + 10×1 = 10 011 ≤ max_step_activations 20 000. Les opérations `fixture.*` sont des contrats de test non implémentés ; voir [workflow-data-iteration](../architecture/workflow-data-iteration.md).
