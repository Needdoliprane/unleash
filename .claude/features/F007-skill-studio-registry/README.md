# F007 — Skills Studio, Skill Registry et Role Registry

## Objectif

OVPT Unleash donne aux utilisateurs et aux agents un catalogue visible de compétences et de rôles réutilisables. Un skill porte les instructions et le contrat d'une compétence ; un rôle organise objectif, responsabilités, skills, modèle, capacités, contexte et scope. Le Role Registry est une capacité V1 de premier ordre, distincte conceptuellement du Skill Registry. Une implémentation commune de stockage n'efface pas cette distinction.

La feature couvre recherche, découverte de similaires, création NL ou manuelle, composition, édition, comparaison, évaluations, publication, binding, dépréciation et révocation. Elle permet de voir les versions effectivement utilisées et celles seulement candidates. La GUI propose un Studio, et CLI/TUI/agents utilisent les mêmes services. La prose publiée reste distincte des contraintes réellement vérifiables.

## Responsabilités

N+2 porte la responsabilité de maintenance des skills. Il peut déléguer la rédaction technique à un N spécialisé et décider ensuite la publication dans ses grants. N+1 recherche et compose les rôles nécessaires à sa mission ; il peut réutiliser ou créer avec justification. L'utilisateur peut éditer et sélectionner des versions. L'intention « environ 99,9 % entretenus par N+2 » n'est pas convertie en quota de créations ou en interdiction de déléguer.

La recherche (`DiscoverResources`) précède la création ; suivie de `RecordDiscoveryDecision`, elle produit un discovery receipt avec candidats, sources, fraîcheur et limites. `SearchSkills` et `SearchRoles` sont des lectures et n'écrivent aucun reçu. Le choix create reste légitime pour un domaine différent, une licence incompatible ou une intégration inappropriée. La similarité ne prouve pas l'équivalence ; la mutualisation n'est pas forcée.

## Périmètre et limites

Le bundle versionne ensemble prompt, outils déclarés, schémas, contrat, ressources et ACL déclarée. La permission effective reste l'intersection de grants actuels et peut être révoquée. Publier un skill ne donne pas à son auteur le pouvoir d'augmenter ses droits. Les évaluations de publication peuvent être déterministes ; aucun second juge LLM n'est obligatoire.

Les portées repository, projet et global ont une priorité explicite. Les bindings engagés sont immuables. Un slot tardif déclaré peut recevoir une version nouvelle avant première activation, après vérification de compatibilité ; ce mécanisme ne remplace pas une version en vol. Les ressources importées sont des données non fiables. Le code interne Unleash reste Rust, alors que les projets et outils externes peuvent être multilangages.

Spécifications : [skills-and-roles](../../specs/skills-and-roles.md), [role-registry](../../specs/role-registry.md), [modèle](../../architecture/skills-contract-model.md). L'[audit Expert 8](../../audits/skills-and-contracts.md) expose garanties limitées et alternatives. Le présent dossier définit le comportement futur et ne présente aucun registre déjà implémenté.
