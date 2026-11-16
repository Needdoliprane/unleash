# F003 — Runtime des agents et admission globale

## Objectif

Exécuter des tours d'agents N/N+1/N+2 à partir de sessions durables, sous contrats, versions et budgets explicites. Un agent logique persiste pendant qu'il dort ; un worker disparaît après sa tentative sans emporter l'identité, les décisions ni les preuves. La limite par défaut de cinq porte sur toutes les unités d'exécution actives du daemon utilisateur (invocations agentes de tous niveaux, replay_worker et provider_probe), avec plafond dur cinquante.

F003 rend possible vingt leaders logiques, des travailleurs parallèles et un manager événementiel sans multiplier des processus actifs pour chaque conversation. Elle prend en charge attente durable, équité interprojets, préemption coopérative, reprise et contexte réellement envoyé. Aucune nouvelle intelligence n'est cachée dans le scheduler ou le process-host.

## Distinctions essentielles

Rôle, skill, agent, session, invocation, tentative, appel API, étape et run/cycle sont des objets différents. Une invocation active peut effectuer plusieurs actions API ; chacune reste traçable et budgétée. Une attente réseau active conserve le slot ; une attente de jugement ou d'enfant le rend après checkpoint sûr. Un fork décrit une branche temporaire d'exécution/contexte, pas une promesse de conserver la RAM ni un appel Unix `fork()` imposé.

## Périmètre et dépendances

Le runtime utilise F001 pour commandes/événements, F002 pour réservations et sessions, le router modèle pour capacités/prix et le broker pour effets. F011 conserve les processus auxiliaires utiles après suspension ; F018 réconcilie les pannes. Les [sessions](../../architecture/session-model.md) et [le scheduling](../../specs/scheduling.md) sont les spécifications canoniques.

Les preuves H/I et une partie de C/J/K sont centrales. Ce dossier est une conception autorisée ; il ne rapporte aucune invocation de produit réellement exécutée ni garantie de compatibilité fournisseur déjà validée.
