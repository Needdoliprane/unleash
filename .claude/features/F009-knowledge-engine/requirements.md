# F009 — Exigences

Toutes les exigences appartiennent à V1. « Doit » exprime un contrat à implémenter et qualifier, pas un état vérifié. Le niveau de confiance est catégoriel et lié à la méthode, jamais un score numérique inventé.

| ID | Exigence testable | Acceptation | Test | Preuve attendue |
|---|---|---|---|---|
| REQ-KNOW-001 | Toute réponse doit citer sa TypedSourceRef, version/digest/génération, ACL observée et états datés pour le consommateur ; une source Git ajoute repo/worktree, commit/tree ou manifeste dirty, une source fichiers/documents/registre reste représentable sans Git. | AC-F009-001 | T-F009-001 | EV-F009-001 |
| REQ-KNOW-002 | Une capture non atomique doit rester `unverified`, ou `inconsistent` si une mutation est détectée ; seuls les mécanismes qualifiés peuvent annoncer cohérence. | AC-F009-002 | T-F009-002 | EV-F009-002 |
| REQ-KNOW-003 | Les cinq langues confirmées et leur combinaison doivent disposer de profils V1 qualifiables, avec capacités, versions et mode dégradé déclarés. | AC-F009-003 | T-F009-003 | EV-F009-003 |
| REQ-KNOW-004 | Les autorisations doivent filtrer ingestion, résultats, compteurs, extraits et contexte, avec invalidation des caches lors d'une révocation. | AC-F009-004 | T-F009-004 | EV-F009-004 |
| REQ-KNOW-005 | Les changements pertinents et reprises doivent invalider l'éligibilité courante affectée sans réécrire reçus/usages passés ; avant effet puis clôture, versions/scope/ACL/bornes sont contrôlés et tout changement extérieur pertinent impose une nouvelle discovery. | AC-F009-005 | T-F009-005 | EV-F009-005 |
| REQ-KNOW-006 | Symboles, signatures, modules, références, liens API/tests et relations doivent porter méthode, profil et confiance ; l'incomplétude doit être visible. | AC-F009-006 | T-F009-006 | EV-F009-006 |
| REQ-KNOW-007 | La découverte doit inclure les travaux non mergés autorisés et leurs propriétaires, en distinguant découverte de disponibilité dans le worktree demandeur. | AC-F009-007 | T-F009-007 | EV-F009-007 |
| REQ-KNOW-008 | Chaque création identifiable doit être reliée à un reçu préalable et une autorisation/observation de son usage exact ; origine invocation/replay/control_plane dérivée du mandat, cause et principal vérifiables, aucune invocation fictive pour parcours humain/mécanique. | AC-F009-008 | T-F009-008 | EV-F009-008 |
| REQ-KNOW-009 | Le choix réutiliser/créer et sa stratégie de coordination doivent être enregistrés avec justification et provenance, sans imposer une équivalence ou mutualisation. | AC-F009-009 | T-F009-009 | EV-F009-009 |
| REQ-KNOW-010 | L'ajout d'une dépendance d'attente doit détecter les cycles concurremment et conserver condition de réveil, propriétaire et délai. | AC-F009-010 | T-F009-010 | EV-F009-010 |
| REQ-KNOW-011 | L'index et l'éligibilité courante doivent pouvoir être reconstruits sans modifier les sources de requête, observations, décisions, reçus ou usages durables ; une lacune d'historique reste unknown jusqu'à réconciliation suffisante. | AC-F009-011 | T-F009-011 | EV-F009-011 |
| REQ-KNOW-012 | Indexation et recherche doivent respecter des budgets bornés, publier leurs limites et distinguer absence de résultat, erreur, accès refusé et recherche partielle. | AC-F009-012 | T-F009-012 | EV-F009-012 |

## Interprétations contraignantes

REQ-KNOW-008 ne suppose pas qu'un shell permet d'identifier toute création sémantique. Les opérations structurées de création sont contrôlées avant effet ; les créations détectées après un patch générique sont marquées `observed_after_creation` et réconciliées selon le contrat de clôture, sans reçu antidaté. Les trous de couverture restent visibles.

REQ-KNOW-003 exige la prise en charge V1 des projets des cinq langues, avec qualification des profils externes retenus ; un fallback lexical n'est pas un prétexte pour supprimer les adaptateurs d'une langue du périmètre. La profondeur interlangage reste conditionnée aux contrats et outils disponibles. REQ-KNOW-002 sépare la conservation d'octets d'une garantie sur l'instant de capture.

REQ-KNOW-005 applique l'[algorithme temporel commun](../../specs/shared-knowledge.md). L'effet exact autorisé et sa publication identique prouvée ne rendent pas non conforme la recherche préalable ; une autre modification, même du même agent, n'est pas exemptée par son auteur. La revalidation à la clôture reste postérieure et ne répare pas l'absence de preuve antérieure. Une borne expirée déclenche un contrôle mécanique documenté ; elle n'efface pas l'histoire.

REQ-KNOW-008 couvre également l'automatisation et la création depuis les trois clients. Le schéma peut rejeter une forme mélangée ; il ne prouve pas l'existence ni l'autorité des références. Les FKs, scopes, lignée du mandat, égalité de principal et ordre des événements sont des contrôles obligatoires du domaine.
