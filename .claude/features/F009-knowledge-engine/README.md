# F009 — Knowledge Engine et découverte d'OVPT Unleash

**Statut : spécification V1, non implémentée.** Famille : `REQ-KNOW`. Responsable de conception : regard 07 Knowledge, avec revue architecte et Git. Les états, tests et artefacts cités sont attendus ; aucun résultat de performance ou de justesse n'est revendiqué.

## Objectif et valeur

Donner aux agents et à Louis une vue vérifiable du code, du travail concurrent et des preuves disponibles. Le moteur doit répondre à « existe-t-il déjà une ressource adaptée ? », « qui la prépare ? », « quel contenu ai-je réellement consulté ? » et « puis-je l'utiliser dans ce worktree ? ». Il évite que des missions recréent un helper ou un contrat faute d'avoir découvert un travail non mergé, tout en laissant une nouvelle implémentation légitime lorsque le domaine diffère.

La recherche avant création concerne fonctions, helpers, classes, composants, API, contrats, tests, skills, rôles et ressources techniques. Le harness enregistre un reçu de recherche et une décision motivée. Il ne promet pas de prouver l'absence de doublons ; il permet de vérifier la démarche et les sources effectivement considérées.

## Périmètre V1

Le moteur couvre les dépôts Python, TypeScript, Java, Go, Rust et multilangages. Des profils LSP externes supervisés apportent les symboles et relations disponibles ; la recherche lexicale sert de voie de base et de mode dégradé déclaré. Le cœur Unleash est Rust ; les bibliothèques natives sont autorisées par la politique R2 et les projets analysés ainsi que leurs outils conservent leurs langages. Les versions et capacités des profils sont qualifiées, avec leurs besoins de runtime et de construction.

Le code intégré et nonmergé, les commits de feature, les modifications dirty, les intentions en développement, les suppressions et les attentes d'intégration sont visibles selon ACL. Les fichiers/documents sélectionnés et versions de registre utilisent les mêmes services avec une provenance typée sans Git ; aucune invocation agente n'est requise pour une recherche ou décision humaine/mécanique. La fraîcheur et l'accessibilité locale sont séparées : trouvé ne signifie ni intégré ni présent dans le worktree ou les sources du demandeur. Le moteur propose des candidats de réutilisation, conserve les décisions et alimente les contextes réellement envoyés aux modèles.

Le reçu V1 est un contrat cible non publié. Son origine invocation/replay/control_plane est dérivée du mandat authentifié, ses sources sont des `TypedSourceRef`. L'utilisation préalable du reçu est observée durablement pour une opération exacte. Sa preuve historique demeure distincte de l'éligibilité courante et d'une éventuelle nouvelle découverte à la clôture après changement extérieur pertinent ; la création autorisée ne périme pas mécaniquement sa propre preuve.

## Limites et décisions

Git reste vérité de ses objets ; fichiers/documents et registres gardent leurs références propres ; l'activité enregistrée reste vérité des observations du harness ; les index sont dérivés. Une capture dirty, documentaire ou de fichiers séquentielle n'est pas présentée comme un instantané temporel cohérent. Une relation lexicale n'est pas promue en résolution sémantique. `normalizeEmail` et `cleanEmail` peuvent remplir des contrats incompatibles : N+1 choisit réutilisation, coordination, attente, copie ou création avec justification.

Les analyses susceptibles d'exécuter du code de build passent par le broker et sa politique. Les profils indisponibles n'empêchent pas de lire les fichiers autorisés, mais la profondeur dégradée est indiquée. Les performances cibles seront fixées sur corpus et matériel déclarés ; cette feature ne fabrique ni mesure ni garantie d'intelligence exhaustive.

## Lecture du dossier

Les [exigences](requirements.md) définissent douze obligations traçables. La [conception technique](technical-design.md) précise capture, indexation et reprise ; les [interfaces](interfaces.md) décrivent les payloads métier. Les [dépendances](dependencies.md) explicitent les contrats partagés. Les [acceptations](acceptance.md) et [tests](tests.md) fixent les preuves attendues. Les décisions canoniques restent dans [shared-knowledge](../../specs/shared-knowledge.md) et [knowledge-indexing](../../architecture/knowledge-indexing.md), après [audit](../../audits/knowledge-audit.md).
