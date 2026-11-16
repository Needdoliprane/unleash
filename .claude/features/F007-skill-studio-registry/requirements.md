# F007 — Exigences atomiques des deux registres

Les familles REQ-SKILL et REQ-ROLE sont distinctes. Les preuves EV sont attendues de l'implémentation. Le choix d'un stockage commun n'autorise pas à retirer les opérations, états ou écrans propres aux rôles.

| Exigence | Règle testable | AC | Test | EV |
|---|---|---|---|---|
| REQ-SKILL-001 | Rechercher avant création et conserver reçu, candidats, choix reuse/create avec stratégie explicite et justification. | AC-F007-001 | T-F007-001 | EV-F007-001 |
| REQ-SKILL-002 | Créer/éditer un draft NL ou manuel révisionné avec hypothèses et provenance. | AC-F007-002 | T-F007-002 | EV-F007-002 |
| REQ-SKILL-003 | Publier un bundle immuable associant instructions, outils, schémas, contrat, ressources et ACL déclarée. | AC-F007-003 | T-F007-003 | EV-F007-003 |
| REQ-SKILL-004 | Permettre N+2 de déléguer rédaction à N puis publier sous ses grants, sans auto-escalade. | AC-F007-004 | T-F007-004 | EV-F007-004 |
| REQ-SKILL-005 | Valider format/références/permissions et évaluations applicables sans second juge LLM universel. | AC-F007-005 | T-F007-005 | EV-F007-005 |
| REQ-SKILL-006 | Afficher versions, usages actifs, diff et vérifications exécutables séparées des jugements. | AC-F007-006 | T-F007-006 | EV-F007-006 |
| REQ-SKILL-007 | Maintenir le bundle de tout binding engagé ; autoriser version nouvelle seulement sur binding vierge éligible ou run suivant. | AC-F007-007 | T-F007-007 | EV-F007-007 |
| REQ-SKILL-008 | Traiter imports comme données, préserver licence/provenance, bloquer références hors bundle non autorisées. | AC-F007-008 | T-F007-008 | EV-F007-008 |
| REQ-SKILL-009 | Révoquer les nouveaux usages et permissions applicables sans effacer l'historique ni réécrire un bundle. | AC-F007-009 | T-F007-009 | EV-F007-009 |
| REQ-SKILL-010 | Exposer mêmes opérations et événements en GUI, CLI, TUI et agents avec résolution de conflits. | AC-F007-010 | T-F007-010 | EV-F007-010 |
| REQ-ROLE-001 | Représenter un registre de rôles avec identité/version distinctes du skill et de l'agent. | AC-F007-011 | T-F007-011 | EV-F007-011 |
| REQ-ROLE-002 | Versionner objectif, responsabilités, skills exacts et entry_skill membre, modèle/politique, capacités, permissions, contrats, contexte et scope. | AC-F007-012 | T-F007-012 | EV-F007-012 |
| REQ-ROLE-003 | Résoudre repository > projet > global avec priorité visible, référence qualifiée et ambiguïté explicite. | AC-F007-013 | T-F007-013 | EV-F007-013 |
| REQ-ROLE-004 | Permettre N+1 de rechercher, composer et publier des rôles dans ses grants avec discovery receipt. | AC-F007-014 | T-F007-014 | EV-F007-014 |
| REQ-ROLE-005 | Binder une version exacte et ses skills atomiquement ; figer le binding engagé, retries compris. | AC-F007-015 | T-F007-015 | EV-F007-015 |
| REQ-ROLE-006 | Refuser rôle incompatible avec niveau, capacité fournisseur, permission ou profil de slot ; input/output exigent digests exacts, règles obligatoires conservées, extensions seulement restrictives. | AC-F007-016 | T-F007-016 | EV-F007-016 |
| REQ-ROLE-007 | Instancier plusieurs agents sur un même rôle sans partager implicitement scope, session ou identité. | AC-F007-017 | T-F007-017 | EV-F007-017 |
| REQ-ROLE-008 | Conserver versions et usages nécessaires à inspection/reprise, avec tombstones et limites de reconstitution lors de purge. | AC-F007-018 | T-F007-018 | EV-F007-018 |

Le terme « modèle » dans un rôle peut être une référence exacte à une politique versionnée ; l'invocation conserve le modèle réellement résolu. Un fallback non autorisé est une erreur. La disponibilité d'un modèle externe n'est pas supposée à partir d'un nom de préférence utilisateur. La résolution de scope ne constitue pas une permission de lire un candidat masqué par ACL.
