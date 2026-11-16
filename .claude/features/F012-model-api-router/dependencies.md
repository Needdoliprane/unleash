# F012 — dépendances et intégration

| Dépendance | Contrat consommé |
| --- | --- |
| F002 | Transactions de réservation, reçus et journal |
| F003 | Slots, bundles, sessions et tentatives |
| F007 | Rôles/skills épinglés et contraintes |
| F008 | Validation de sorties |
| F017 | Secrets, autorisations et destinations |
| F016 | Usage et provenance pour rapports |
| F011 | Registre et contrôle d’un serveur local seulement lorsqu’Unleash en gère explicitement le processus |
| F018 | Réconciliation après coupure, chargement interrompu et issue de génération inconnue |
| F014 / F015 | Paramètres, tests, sélection et inspection identiques depuis GUI/TUI/CLI |
| F019 | Politique modèle du run, interdiction transitive et checkpoints de supervision autorisés |

Une dépendance de contrat peut être développée avec un double, mais le jalon d’intégration exige l’implémentation réelle. Les doubles n’accèdent ni au réseau de production ni à des secrets. Les équipes figent un schéma commun avant de travailler en parallèle ; une modification incompatible impose nouvelle version et test interfeatures.

Les clients consomment cette feature par le backend. Les sorties sont des résultats, événements et références de preuve ; elles ne modifient pas directement les projections d’une autre feature. La transaction de domaine engage les décisions avant dispatch d’I/O.

## Dépendances externes de la voie locale

LM Studio, Ollama et le serveur llama.cpp sont des cibles HTTP distinctes à qualifier. « CPP » est interprété comme llama.cpp dans le périmètre. Les [sources](../../research/local-model-sources.md) décrivent les API observées ; les versions de serveur, modèles, templates, formats et licences réellement utilisés doivent être inventoriés dans les preuves. La documentation ne constitue pas un lockfile ni un test réussi.

Le premier mode réutilise un endpoint préparé par l’utilisateur. Aucun SDK Python/TypeScript, outil d’abonnement CLI, téléchargement de poids ou runtime d’inférence embarqué n’est requis par ce contrat. Une dépendance native ou un serveur externe non Rust reste autorisé par la politique d’implémentation ; le routeur et ses règles métier propres restent Rust.

Les mocks sont suffisants pour développer politiques, budgets et transport, mais les trois backends demandent chacun une qualification réelle avant déclaration de prise en charge. Le [plan local](../../testing/local-model-tests.md) sépare cette preuve des tests de doubles. La concurrence, le placement et les capacités d’outils ne sont pas hérités d’une autre marque ou d’un autre modèle.

