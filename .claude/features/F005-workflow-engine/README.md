# F005 — Moteur de workflow

## Objectif

Cette feature transforme une définition publiée en progression durable, inspectable et bornée. Le moteur détermine quelles étapes peuvent s'activer, quelles preuves permettent de continuer et quand le run peut se fermer. Il laisse les choix techniques aux agents dans les points de décision prévus et applique seul les transitions, scopes, contrats et budgets.

La valeur utilisateur est double : rendre le travail autonome sans boucle manager permanente, et expliquer précisément pourquoi une mission avance, attend, échoue ou termine. Les états sont communs à GUI, CLI et TUI. Une reprise après crash réutilise la même identité de run et ses événements ; elle n'interprète pas de nouveau un texte de plan pour reconstruire ce qui aurait dû se produire.

## Périmètre V1

Le moteur exécute tâches, séquences, choix, branches parallèles, jointures, boucles bornées et échecs explicites, actions mécaniques (v1.1) et itérations for_each (v1.2). Il prend en charge deadlines, budgets, pauses, annulations, corrections et réconciliation des résultats inconnus. Tests et audits sont des étapes configurables. La définition de done utilisateur peut être Dev → Merge ; aucune étape métier supplémentaire n'est imposée par le moteur.

La définition de workflow est immuable pour un run. Les bindings fixes sont résolus au démarrage ; les slots tardifs déclarés peuvent être liés explicitement avant leur première activation, dans les capacités prévues. Chaque binding engagé devient immuable. Une nouvelle version publiée ne remplace jamais la version d'une étape déjà engagée ou de ses retries.

## Limites et arbitrages

Le déterminisme porte sur la réduction d'état à événements identiques ; les réponses LLM et systèmes distants restent externes. Un budget borne les efforts, pas la difficulté intrinsèque du problème. Le moteur ne revendique aucune répétition exactly-once d'un effet distant. Un résultat inconnu peut imposer un blocage et une réconciliation.

La fin de cycle est la terminalité du run (Q-03), avec branches clôturées et barrières de succession explicites. La migration à checkpoint est écartée de la V1 ; aucun remplacement à chaud. Les détails sont dans [workflow-engine](../../specs/workflow-engine.md), [DSL](../../architecture/workflow-dsl.md) et [états](../../architecture/state-machines.md).

Le [rapport Expert 3](../../audits/workflow-determinism.md) précède ces propositions. Les sept fichiers de ce dossier définissent exigences, interfaces, dépendances, scénarios et preuves futures. Aucun moteur produit n'a été implémenté dans ce travail de conception.
