# F010 — Plan de tests Git

**État : non exécuté.** Tous les programmes de test Unleash seront Rust ; ils pilotent de vrais dépôts temporaires et le Git qualifié. Les tests utilisent des frontières de panne déterministes pour distinguer un problème de synchronisation d'une simple temporisation aléatoire.

| Test | Exigence / acceptation | Méthode | Preuve attendue |
|---|---|---|---|
| T-F010-001 | REQ-GIT-001 / AC-F010-001 | Inventaire d'un dépôt attaché avec allocations externes, dirty et merge en cours | EV-F010-001 : état avant/après et readiness |
| T-F010-002 | REQ-GIT-002 / AC-F010-002 | Barrière de démarrage de deux writers ; puis mêmes opérations dans worktrees distincts | EV-F010-002 : registre d'ownership et contenus finaux |
| T-F010-003 | REQ-GIT-003 / AC-F010-003 | Modifier fichier entre lecture et patch, symlink hors scope et epoch périmé | EV-F010-003 : rejets typés et absence de modification interdite |
| T-F010-004 | REQ-GIT-004 / AC-F010-004 | Préparer à OID fixes puis déplacer source/ref avant publication | EV-F010-004 : candidat, parents, stratégie et nouvelles préconditions |
| T-F010-005 | REQ-GIT-005 / AC-F010-005 | Bloquer le sous-processus, expirer lease, relancer daemon et demander nouvelle lease | EV-F010-005 : identité de processus, fencing et état reconciling |
| T-F010-006 | REQ-GIT-006 / AC-F010-006 | Deux CAS concurrents, mouvement externe et cible checkout | EV-F010-006 : old/new OID, gagnant unique et fichiers humains intacts |
| T-F010-007 | REQ-GIT-007 / AC-F010-007 | Conflit textuel avec fixtures des deux branches et mandat spécialisé | EV-F010-007 : événements conflit/assignation/résolution |
| T-F010-008 | REQ-GIT-008 / AC-F010-008 | Deux workflows identiques sauf étape de validation ; conflit sémantique sans conflit textuel | EV-F010-008 : étapes réellement exécutées et limites du rapport |
| T-F010-009 | REQ-GIT-009 / AC-F010-009 | Arrêt à chaque frontière intention/dispatch/CAS/observation/journal ; retour ABA extérieur | EV-F010-009 : matrice d'états récupérés et décisions de reprise |
| T-F010-010 | REQ-GIT-010 / AC-F010-010 | Rebase d'une source référencée par knowledge, test et receipt | EV-F010-010 : anciennes/nouvelles provenances et invalidations |
| T-F010-011 | REQ-GIT-011 / AC-F010-011 | Cleanup sur dirty, processus actif, preuve retenue puis cas réellement retirable | EV-F010-011 : refus, conservation et suppression finale autorisée |
| T-F010-012 | REQ-GIT-012 / AC-F010-012 | Matrice chemins/objet/sous-module/unborn, événement redélivré et worktree déplacé | EV-F010-012 : manifeste d'inspection et unicité des transitions |

## Tests de faute complémentaires dans les mêmes scénarios

Injecter disque plein à l'écriture d'intention et de résultat, erreur de permission, objet manquant, hook non autorisé et annulation de processus. L'intention non durable interdit de démarrer un nouvel effet. Un résultat non durable après effet produit une reprise explicite ; le journal ne doit pas inventer un échec certain. Le test doit aussi constater qu'un nouveau daemon ne confond pas un PID réutilisé avec l'ancien processus.

Une simulation de crash processus ne qualifie pas la panne de courant. Les essais de durabilité matérielle devront être réalisés séparément sur configuration déclarée ; le rapport indique ce qui a été injecté. Les systèmes de fichiers réseau ne sont pas supposés offrir les mêmes garanties que le stockage local choisi.

## Oracles indépendants

L'oracle de publication lit la ref réelle et l'ancestry avec Git, compare les OID de l'intention et observe le worktree cible. L'oracle métier vérifie workflow/étapes/contrat, sans déduire l'état de la dernière ligne stdout. Le test d'absence de phase forcée échoue si un audit LLM ou une suite de tests apparaît dans B alors qu'il n'est pas demandé.

Le corpus de conflit sémantique inclut un appel dont le contrat est changé dans une branche et un consommateur nouveau dans l'autre. Si des tests sont prescrits, ils doivent révéler le comportement attendu ; sinon le rapport conserve le risque non validé et n'affirme pas la sûreté sémantique. Cette distinction est le résultat recherché, pas une imperfection à masquer.

Chaque EV conserve versions, configuration, manifestes de dépôt, journal, commandes expurgées, OID et états récupérés. Les scénarios sont intégrés aux parcours [A, D, G, L et N](../../testing/integration-tests.md), avec les preuves du [reporting](../F016-reporting-observability/README.md).

### Scénarios R6 pour T-F010-001/004/006/008/009/011

Fixtures Git isolées : symref préexistante et injectée avant CAS vers une branche témoin ; nettoyage d’une ref de conservation substituée ; replacement de commit et blob ajouté/retiré entre capture et validation ; ancestry de réconciliation. Les EV relèvent valeurs des refs témoins, octets/parents, profil exact, refus/quarantaine et absence de preuve réutilisée sous profil inconnu. Une qualification OS/Git réelle est requise ; ces tests restent not_run.

### Scénarios R7 pour T-F010-001/006/009/011/012

T-F010-001 et T-F010-006 ajoutent : cible checkout dans le worktree humain dirty, avec index et fichiers comparés octet pour octet avant et après ; cible en rebase, en bisect, en merge et en cherry-pick ; proposition de la branche gérée non checkout par défaut ; libération du checkout par l'utilisateur selon les instructions affichées, puis réobservation ; checkout humain réalisé pendant la fenêtre CAS, détecté par l'observation postérieure et classé incident ; Git 2.38 refusé `GIT_VERSION_UNSUPPORTED` et Git 2.39 accepté, sur macOS et sur Linux. T-F010-009 vérifie chaque valeur de `reconciliation_status`, le dépôt bare sans reflog et l'absence de publication automatique hors `effect_observed`. T-F010-011 ajoute la rétention par défaut de 7 jours d'un worktree propre et publié par le balayage automatique, son retrait explicite par `RetireWorktree` avant cette échéance et le refus `dirty_worktree` d'un commit non publié. T-F010-012 ajoute un volume démonté puis un `git worktree prune` extérieur sur un worktree verrouillé. Les preuves attendues restent celles du tableau ; ces tests restent not_run.
