# F013 — Connecteurs documentaires

Statut : spécification proposée pour la V1 complète d’OVPT Unleash ; aucun composant applicatif n’est implémenté par ce dossier.

## Objectif et valeur

Permettre de choisir précisément les ressources Google Drive, OneDrive et Notion utiles à une mission, puis prouver quelle révision a été utilisée.

## Périmètre et frontières

Explore, Pin, Scope et Watch sont fournis pour les trois connecteurs. Le fonctionnement local ne suppose ni crawl complet ni endpoint webhook public. Les contenus et métadonnées sont filtrés selon compte, projet et affectations, avec les limites de chaque fournisseur visibles.

Le code de cette feature et ses tests développés pour Unleash seront Rust. Les services externes et les outils des projets supervisés gardent leurs langages. Cette séparation respecte l’annexe A et ne dispense pas d’auditer les dépendances natives.

## Parcours de réalisation

Lire la [spécification canonique](../../specs/connectors.md), puis les [exigences](requirements.md), le [design technique](technical-design.md) et les [interfaces](interfaces.md). Le chantier démarre avec les dépendances et doubles décrits dans [dependencies](dependencies.md). Chaque comportement possède un [critère](acceptance.md) et un [test prévu](tests.md).

La feature est terminée seulement lorsque les contrats sont implémentés, les cas de panne passent et les preuves attendues existent. Un mock démontre un contrat local, pas l’accès réel à un fournisseur. Les points ouverts qui touchent un compte, une licence ou AI-Guard restent des gates explicitement visibles, sans report artificiel en V2.

