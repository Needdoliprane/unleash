# F008 — Acceptation

| AC | Given / When / Then |
|---|---|
| AC-F008-001 | Étant donné output valide de forme mais contenant un claim faux, quand la validation s'exécute, alors seul le contrôle structurel passe ; les vérifications de preuve gardent leur résultat distinct. |
| AC-F008-002 | Étant donné validateur inconnu ou paramètres scriptés, quand le contrat est publié, alors il est refusé sans code libre ; un contrat valide utilise compiler/evaluator/catalogue épinglés avant évaluation et inchangés après reprise. |
| AC-F008-003 | Étant donné permission initiale révoquée après binding, quand une action est demandée, alors elle est refusée malgré bundle épinglé ; un auteur N+2 ne peut s'auto-accorder le grant. |
| AC-F008-004 | Étant donné claim typé ou qualitatif, observation et jugement sur un même sujet, quand ils sont consultés, alors auteur/producteur authentifiés, révision, snapshot et liens restent distincts ; une forme JSON ou un auteur déclaré ne crée aucune observation. |
| AC-F008-005 | Étant donné une attente scellée avant effet et deux commandes du même worker/snapshot, quand la suite requise échoue et un filtre bénin réussit, alors la seconde observation ne satisfait pas la commande exigée ; attendu absent, tardif ou fourni par la sortie refuse l'admission ou la preuve, sans pass. |
| AC-F008-006 | Étant donné preuve obligatoire absente ou purgée, quand la clôture est demandée, alors le verdict unknown bloque/échoue conformément au contrat et n'est pas converti en réussite. |
| AC-F008-007 | Étant donné un claim de prédicat V1 et la projection complète du binding attendu, quand les valeurs diffèrent, alors ClaimContradicted conserve les deux sources ; texte modifié seul ne change rien, qualitatif/projection manquante reste unknown, et un pass de liens ne satisfait pas la règle de vérité obligatoire. |
| AC-F008-008 | Étant donné création manuelle/agente/replay sous discovery, quand l'effet puis la clôture sont contrôlés, alors origine, sources, bornes et usage historique sont vérifiés ; cible modifiée exige revalidation finale distincte, et aucun reçu postérieur ne répare le préalable absent. |
| AC-F008-009 | Étant donné contrat simple couvert par fixtures, quand publication et évaluation sont exécutées sans second modèle, alors elles fonctionnent et le rapport distingue conformité et qualité non démontrée. |
| AC-F008-010 | Étant donné preuve sensible puis purge autorisée, quand un autre scope ou une ancienne session l'inspecte, alors ACL et tombstone sont respectés sans fuite ni fausse validité. |

Le scénario F exige que les claims de tests soient reliés à des observations réelles, pas seulement à un fichier nommé test.log. Le scénario E exige publication versionnée sous autorité. Le scénario D utilise discovery et statut nonmergé en coopération avec knowledge. Les preuves doivent porter environnement et snapshot testés, ainsi que les versions du contrat et du validateur.

R6 précise l'oracle : correspondance exacte des SubjectRef, expectation_ref, binding/action courants, tool/argv/cwd/environnement/suite/parseur. Les variantes d'ancien snapshot, scope révisé, mauvaises commande ou tentative échouent la correspondance ; donnée indisponible donne unknown. Les tests de résultat exigent code attendu, parsing complet et compteurs cohérents ; si tous les cas doivent passer, aucun cas ignoré n'est assimilé à un succès. Un claim vrai d'échec ne satisfait pas la règle séparée de réussite. Toutes les acceptations produit ci-dessus restent `not_run`.
