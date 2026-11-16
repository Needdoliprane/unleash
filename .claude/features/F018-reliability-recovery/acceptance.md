# Acceptation F018

| ID | Given / When / Then |
|---|---|
| AC-F018-001 | Étant donné une session avec décisions et action en vol, quand le worker meurt, alors le contexte durable reste inspectable et une nouvelle tentative n'est créée qu'après classification de l'action. |
| AC-F018-002 | Étant donné C0–C12 et variantes objet/backup C10a–d/C11a–d, quand le daemon redémarre, alors la lignée est conservée, receipts/tombstones et états correspondent aux commits, pins suspects restent protecteurs, deleting interdit le rattachement et aucune action ambiguë n'est redéclenchée automatiquement. |
| AC-F018-003 | Étant donné un reboot avec anciens PID enregistrés, quand recovery démarre, alors le boot différent invalide ces identités et les requêtes distantes restent à réconcilier. |
| AC-F018-004 | Étant donné un effet externe réussi dont le receipt a été perdu, quand une reprise est demandée, alors outcome_unknown bloque le retry ; une preuve ou décision autorisée explicite permet la suite et reste auditée. |
| AC-F018-005 | Étant donné un host/worker ancien et une génération réaffectée, quand ancien contrôle/résultat arrive, alors StaleFence ou IdentityUnverified protège la nouvelle génération. |
| AC-F018-006 | Étant donné slots suspects, processus vivants et coût inconnu, quand le ledger est rapproché, alors chaque réservation garde la disposition appropriée et une divergence bloque le pool plutôt qu'être effacée. |
| AC-F018-007 | Étant donné backup40, host100, ancien token, commande avec réponse perdue et effets/coûts postérieurs, quand restore puis reconnect/resync surviennent, alors neutralisation et capsule durable précèdent activation ; ancien host/token/commande refusés, même avec révision restaurée égale ; effet non prouvé reste unknown, coût non effacé, couverture perdue gèle les scopes ou globalement si étendue inconnue. |
| AC-F018-008 | Étant donné recul d'heure et veille, quand une deadline murale semble passée, alors aucune ressource suspecte n'est réattribuée sans revalidation de propriétaire/fence. |
| AC-F018-009 | Étant donné un rapport avec inconnues et preuve manquante, quand GUI/CLI/TUI s'y reconnectent, alors les mêmes items/états/actions permises apparaissent au même watermark. |
| AC-F018-010 | Étant donné FULL, IOERR ou corruption, quand une commande et une reprise sont tentées, alors aucun succès non prouvé n'est accusé et les fichiers de journal sont conservés pour diagnostic/restauration. |

N est couvert par 002/004/005/006/007, H par 001 et O par 009. Les garanties d'absence de retry automatique sont vérifiées par un observateur externe de fixture, pas par la seule absence d'événement de répétition dans le journal Unleash.

Le cas 007 inclut une action déjà terminée après le snapshot et une action encore prepared dans celui-ci mais exécutée depuis. La capsule est vérifiée après un second crash de recovery ; aucun nouveau dispatch ne peut être justifié par le seul retour à un état ancien. Une file client qui remplace automatiquement authority_generation ou command_id échoue ce critère.
