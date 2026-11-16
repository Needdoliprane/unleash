# F017 — Sécurité locale et intégration AI-Guard

Statut : conception proposée pour la V1 complète ; aucune implémentation du produit n’est livrée dans ce dossier.

## Objectif

Faire appliquer une autorité déterministe sur les actions, protéger les secrets et intégrer AI-Guard sans inventer son API ni coupler le moteur à son implémentation.

## Portée

La V1 couvre le broker local obligatoire, les secrets, les scopes, l’identité des agents, les politiques, le mode d’isolation déclaré et l’adaptateur AI-Guard. Connexion, santé, état de politiques, audit, incidents et révocation sont administrables selon capacités négociées. Le contrat réel reste un gate externe.

Le code et les tests internes seront Rust. Les décisions externes non vérifiées sont nommées comme telles. Une capacité exigée reste dans le périmètre même si son gate de qualification n’est pas encore satisfait.

## Lecture et réalisation

Commencer par la [spécification](../../specs/ai-guard.md), puis [requirements](requirements.md), [technical-design](technical-design.md) et [interfaces](interfaces.md). Les [dépendances](dependencies.md) séparent contrats simulables et intégration réelle. Les [critères](acceptance.md) et [tests](tests.md) forment le contrat de sortie du chantier.

Les équipes peuvent développer en parallèle sur des doubles de contrat versionnés. Elles doivent rejoindre les services communs avant la recette. Aucun succès sur maquette ou mock ne doit être converti en preuve de comportement système réel. Les mises à jour de conception sont tracées dans ADR et matrices, avec distinction entre invariant utilisateur et proposition technique.

