# Skill Studio et Role Registry

Deux onglets conceptuels : compétences et rôles. La bibliothèque affiche namespace global/projet/repository, version, auteur, état draft/validated/published et statut de registre deprecated/revoked, contrats, permissions requises, usages et évaluations. Le contenu complet reste lisible et exportable ; aucun prompt caché obligatoire.

## Création

Avant création, la recherche retourne candidats et un DiscoveryReceipt. L’utilisateur ou N+2 enregistre une décision reuse ou create avec une stratégie explicite : adapter la même compétence vaut reuse/adapt, dériver une compétence distincte vaut create/adapt, une nouvelle compétence justifiée vaut create/new ([discovery-receipt](../schemas/discovery-receipt.schema.json)). Le N+2 peut déléguer la rédaction technique à un agent spécialisé ; le studio montre demande, rapport et décision de publication.

Le formulaire sépare instructions en prose, entrées/sorties, outils, préconditions, obligations de trace et prédicats vérifiables. Une phrase « tests exhaustifs » ne devient pas un contrôle exécutable. L’éditeur explique ce qui est validable et ce qui restera jugement.

## Rôles

Un N+1 compose un rôle à partir de versions précises de skills, responsabilités, modèle, capabilities, contrats, contraintes de contexte et paramètres de scope. Le rôle est retrouvé dans des missions futures, avec provenance. Une résolution repository > projet > global est visible, et les collisions ne sont pas masquées.

## Publication

Comparer deux versions côte à côte : texte, schémas, outils, permissions et tests d’évaluation. Les contrôles mécaniques bloquent un contrat invalide ou une extension de droits impossible. Une politique d’auto-publication peut autoriser le N+2 dans un mandat ; une revue humaine permanente n’est pas imposée. Les essais sémantiques améliorent la confiance sans prétendre prouver la qualité universelle.

Références : [rôles](../specs/role-registry.md), [contrats](../specs/contract-enforcement.md), F007/F008 et AC-F014-007.

