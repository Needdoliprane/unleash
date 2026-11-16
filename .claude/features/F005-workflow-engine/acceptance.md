# F005 — Acceptation

| AC | Given / When / Then |
|---|---|
| AC-F005-001 | Étant donné un run v1 actif, quand v2 est publiée puis plusieurs appels et une reprise surviennent, alors chaque transition du run conserve le digest v1 et aucun alias n'est relu pour remplacer sa définition. |
| AC-F005-002 | Étant donné état initial et journal identiques, quand le reducer est rejoué sans adaptateurs, alors les mêmes états, compteurs et intentions canoniques résultent, sans requête réseau ni action Git. |
| AC-F005-003 | Étant donné guards vrais, faux, absents et mal typés, quand choix et boucle s'évaluent, alors seule la branche définie est prise ; inconnu bloque et erreur de type n'est pas convertie. |
| AC-F005-004 | Étant donné bornes connues, quand retries, boucles et nouveaux workers les consomment, alors aucun compteur n'est remis à zéro et aucune action supplémentaire n'est admise après épuisement. |
| AC-F005-005 | Étant donné trois branches, quand all, any_success et quorum sont exercés avec résultats tardifs, alors seuil, annulation et clôture sont distincts ; aucun résultat tardif ne ressuscite une branche. |
| AC-F005-006 | Étant donné une action dont l'effet précède un crash de journalisation, quand le run reprend, alors l'action est outcome_unknown et ne se répète pas avant observation ou arbitrage autorisé. |
| AC-F005-007 | Étant donné workflow Dev → Merge et droits suffisants, quand les sorties attendues sont observées, alors le run réussit sans test/audit ajouté ; un merge interdit reste bloqué. |
| AC-F005-008 | Étant donné v2 publiée pendant v1 et une branche v1 encore active, quand une invocation se termine, alors v2 n'est pas adoptée ; un nouveau run éligible après clôture peut sélectionner v2. |
| AC-F005-009 | Étant donné effets déjà observés et outbox non notifiée, quand le daemon redémarre, alors les intentions manquantes sont retrouvées et les effets observés ne sont pas réexécutés aveuglément. |
| AC-F005-010 | Étant donné un slot vierge et un slot engagé, quand un rôle compatible nouveau est proposé, alors seul le slot vierge accepte la liaison ; manifest et refus sont inspectables. |

L'acceptation distingue terminalité de résultat métier, de processus et d'effet externe. Les rapports doivent inclure le snapshot de configuration, la version de reducer et les raisons de blocage. Le scénario K du mandat est vérifié avec appels, étapes, branches et crash ; le scénario L vérifie le done configurable sans affaiblir les autorisations.
