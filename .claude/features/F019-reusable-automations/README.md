# F019 — Automatisations réutilisables apprises puis rejouées

## Objectif

Permettre à un utilisateur de faire apprendre une procédure, de publier une automatisation paramétrée, puis de la réexécuter avec de nouvelles données. Il choisit un replay sans LLM ou une supervision où un LLM examine les logs/résultats aux points utiles pendant que le script réalise les gestes. Une modification du site, du contrat ou de la procédure peut conduire à une réparation explicitement autorisée et à une nouvelle version. Le harness sert ainsi des activités de comptabilité, de gestion personnelle et de développement, au-delà de la seule production de code.

**Confirmé par l'intention R3/R5 :** apprentissage éventuellement assisté par LLM, réutilisation de scripts/procédures et données CSV, possibilité d'un replay sans LLM, orchestration optionnelle par LLM depuis les logs/résultats, réparation explicite, cœur Rust et réemploi de moteurs existants. **Proposé dans cette feature :** modèle `AutomationArtifact`, modes publiés `replay/supervised` et jobs de conception `learn/repair`, profil d'actions, publication, limites, commandes et preuves. **Décidé en R7 :** pilote navigateur de référence WebDriver piloté en Rust (`fantoccini`, chromedriver et Chrome for Testing épinglés, [Q-18](../../brief/open-questions.md)). **À qualifier :** versions figées des moteurs, isolation sur les OS ciblés ([Q-05](../../architecture/security-boundaries.md), Q-11), performances et compatibilité des sites. Aucun code produit ni résultat de test n'est livré par ce dossier.

## Parcours

L'utilisateur recherche une automatisation existante puis réutilise ou crée un draft. `LearnAutomation` peut observer une tâche autorisée avec un LLM et proposer des paramètres, locators et contrôles. Le candidat est validé et évalué contre des résultats attendus observables, puis publié en version immuable. `CreateRun` sélectionne version, mode et `AutomationDataRevision`. En `replay`, le moteur impose `model_policy=forbidden`. En `supervised`, des tâches `leader_decision` bornées interviennent entre des blocs mécaniques, avec décisions typées `continue/reconcile/correct/escalate`. Les clics restent exécutés par le driver/script.

En cas de dérive, le run expose le blocage et ses preuves. `RepairAutomation` ouvre un travail de conception distinct, à la demande ou selon une politique préautorisée bornée ; sa publication n'altère aucun run engagé. Un parcours débutant peut proposer une supervision aux checkpoints utiles, avec fréquence et coûts annoncés, sans exiger que l'utilisateur construise ses agents ou le graphe à la main. Cette proposition de défaut ne supprime ni le mode zéro modèle ni les workflows sans juge LLM.

Une trace réussie et un script généré ne suffisent pas à certifier une automatisation. Le contrat vérifie les données, l'autorité, l'identité de la cible et le résultat métier. Le même bouton CSS peut rester cliquable après un changement fonctionnel : le run doit détecter la divergence pertinente plutôt que déclarer réussite sur la seule absence d'erreur du navigateur.

## Exemples de qualification proposés

| Public | Apprentissage ou préparation | Replay et critère observable |
|---|---|---|
| Comptable | Définir un import CSV de factures et un rapprochement de montants/identifiants sur un système de test. | Même procédure sur une nouvelle révision ; identifiants, devise, totaux et nombre de pièces concordent avec la fixture indépendante. Une ambiguïté de compte bloque. |
| Particulier | Relever une navigation de catalogue et préparer une liste de courses ou un panier simulé depuis un CSV. | Vérifier article, unité, quantité et plafond de prix ; état de panier de fixture conforme. Aucun achat ou paiement réel n'est requis pour la qualification. |
| Développeur | Préparer un scénario E2E et ses paramètres sur une application de test. | Exécuter une suite déterministe via un moteur réutilisé, comparer l'état attendu et conserver les preuves, sans réanalyser le scénario avec un LLM à chaque passage. |

Ces exemples ne créent pas trois orchestrateurs spécialisés ni trois obligations de connecter des services réels. Les profils de rôle et l'expérience grand public réutilisent F004/F007/F014/F015 ; F019 apporte l'artefact et son exécution commune. Les scripts de projet peuvent être dans le langage de leur écosystème, par exemple JavaScript avec Playwright, sous un contrat externe explicite ; cela ne déplace pas les permissions, l'orchestration ou la persistance hors du cœur Rust.

## Garanties et périmètre

Le mode `replay` consomme zéro appel modèle, y compris dans ses descendants, managers, rapports, embeddings ou outils. Le mode `supervised` montre les appels et décisions autorisés aux points déclarés ; il ne revendique pas zéro LLM. Un log « succès » ne remplace jamais les assertions et snapshots du résultat métier. Les deux modes utilisent du temps machine et des permis d'exécution globaux. La décision Q-15 ([ADR-13](../../architecture/adr/ADR-13-scheduler-budget.md)) conserve 5 unités actives par défaut et 50 au maximum, partagées entre invocations agentes, workers de replay et probes fournisseurs ; les budgets navigateur/processus/mémoire restent distincts et ne multiplient pas ce plafond.

Le contrôle déterministe n'empêche pas le système distant de changer. L'automatisation ne promet ni sélecteurs éternels, ni comportement métier universel, ni exactly-once externe, ni restauration de la RAM d'un navigateur. Un effet inconnu reste à réconcilier avant répétition. L'absence de LLM est garantie sur les composants et descendants contrôlés par Unleash, pas sur l'implémentation interne opaque du site tiers.

## Références

- [Exigences et traçabilité locale](requirements.md) : 18 exigences REQ-AUTO-001 à REQ-AUTO-018.
- [Modèle d'artefact](../../architecture/automation-artifact-model.md) et [profil d'exécution](../../architecture/automation-execution-profile.md).
- [Design technique](technical-design.md), [interfaces](interfaces.md) et [dépendances](dependencies.md).
- [Acceptation](acceptance.md), [tests](tests.md) et [campagne de replay](../../testing/automation-replay-tests.md).
- [Architecture navigateur](../../architecture/browser-execution.md), [spécification navigateur](../../specs/browser-automation.md) et [sources primaires](../../research/browser-automation-sources.md).
