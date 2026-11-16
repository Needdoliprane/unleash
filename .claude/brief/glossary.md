# Glossaire normatif

| Terme | Sens dans Unleash | Ne signifie pas |
| --- | --- | --- |
| Goal | Objectif utilisateur versionné avec politique de complétion | Liste de chats |
| Mission | Mandat borné sous un goal avec owner et dépendances | Un processus |
| N+2 | Manager logique organisationnel | LLM permanent ou développeur de tous conflits |
| N+1 | Leader logique de réalisation technique ou de domaine | Second moteur d’état dans un prompt |
| N | Agent réalisant une tâche spécialisée | Un niveau de délégation supplémentaire libre |
| Agent logique | Identité, rôle, mandat et session persistants | PID |
| Session | Historique durable, références et mémoire reconstruisible | Conversation serveur fournisseur garantie |
| Appel API | Requête modèle locale ou distante individuelle, avec éventuel streaming | Invocation complète ni workflow |
| Invocation | Tranche bornée d’activité d’un agent, avec contexte épinglé et slot | Toute la durée de vie de l’agent |
| Tentative | Exécution identifiée d’une invocation ou action avec reprise contrôlée | Nouveau travail indépendant |
| Worker/fork | Incarnation locale éphémère | Source de vérité durable |
| Tâche | Unité de travail sous contrat, pouvant nécessiter plusieurs invocations | Étape entière nécessairement |
| Étape | Nœud d’un workflow, avec état et conditions explicites | Synonyme d’appel LLM |
| Run | Exécution d’une définition de workflow immuable | Version publiée du workflow |
| Cycle | Décision Q-03 : run terminé avec effets enfants réconciliés ou sous barrière | Chaque tour de chat |
| Invocation éligible | Invocation habilitée à démarrer un nouveau run, après l’état terminal du run précédent (Q-03) | N’importe quel réveil d’un N+1 |
| Skill | Compétence réutilisable avec instructions et contrat | Permission auto-accordée |
| Rôle | Composition versionnée de responsabilités, skills, modèle et contraintes | Étiquette fugace du prompt |
| Scope | Mandat de ressources et responsabilités, avec règles applicables | Sandbox OS implicite |
| Discovery receipt | Preuve persistante d’une recherche, couverture et choix | Preuve d’absence de doublons |
| Observation | Fait capté par outil ou adaptateur identifié | Vérité complète de tout ce qui s’est passé sur l’OS |
| Claim | Déclaration d’un modèle ou acteur | Preuve d’exécution |
| Judgment | Appréciation du N+1 ou autre autorité | Invariant mécanique |
| Evidence | Artefact ou résultat référencé et borné dans son contexte | Simple phrase « tests OK » |
| Snapshot | Octets et provenance identifiés, avec niveau de cohérence | Lecture atomique d’un worktree vivant par défaut |
| Disponible | Présent et utilisable dans l’environnement cible identifié | Seulement découvert ailleurs |
| Lease | Droit temporaire d’utiliser une ressource gérée | Verrou sur tous les Git externes |
| Fencing token | Époque monotone empêchant un ancien propriétaire d’engager une action | Garantie d’annulation d’un effet distant |
| Outbox | Intentions de dispatch engagées avec l’état | Exactly-once universel |
| Outcome unknown | Effet possible dont le résultat ne peut être établi | Échec sûr à rejouer |
| Pause | Empêcher nouvelles actions et atteindre un checkpoint sûr | Annulation instantanée de tout réseau distant |
| Interruption | Demande de terminaison d’exécution avec réconciliation | Effacement du travail |
| Reconnexion | Rattrapage des événements puis synchronisation de snapshot | Rejouer les commandes envoyées sans déduplication |
| AI-Guard | Produit de sécurité distinct ; frontière côté Unleash acceptée (Q-04, ADR-10), contrat réel à qualifier (EXT-GUARD) | API supposée existante |
| .claude/ | Dossier de conception livré, référence documentaire de la réalisation ; héberge aussi, depuis R7, l’outillage optionnel des agents de développement | Répertoire runtime d’Unleash ou dépendance Claude Code (`state/`, `worktrees/` et `settings.local.json` sont exclus du manifest) |
| Déterministe | Transition reproductible à entrées enregistrées identiques | Sortie textuelle LLM reproductible bit à bit |

## Termes ajoutés R3–R5

| Terme | Sens dans Unleash | Ne signifie pas |
| --- | --- | --- |
| AutomationArtifact | Version immuable d’une procédure, de ses contrats, paramètres et références de moteur | Suite de clics non qualifiée ou nouveau moteur de workflows |
| AutomationDataRevision | Snapshot typé d’un jeu d’entrées et de son parsing | Fichier mutable relu différemment au cours du Run |
| AutomationRun | Projection d’un Run dont la source est une version d’automatisation | Deuxième machine à états |
| ReplayWorker | Exécuteur mécanique admis dans le plafond global | Agent LLM ni worker gratuit hors contrôle de ressources |
| Replay | Exécution avec model_policy forbidden | Monde externe figé ni résultat métier identique malgré nouvelles données |
| Supervised | Exécution avec tâches de décision publiées et appels modèles bornés | Un modèle décide chaque clic ou chaque ligne de log |
| Learn / Repair | Travail de conception ou d’adaptation produisant un candidat versionné | Hotpatch d’une procédure en cours |
| ObservationReport | Résultats typés, références de preuves, état des effets et limites | Log libre suffisant pour établir la vérité métier |
| Local-only | Politique sur le placement effectif d’inférence et les destinations | Simple hostname localhost |
| GuidedSetup | Brouillon de configuration guidée, validé puis appliqué idempotemment | Nouvelle logique métier propre à la GUI |
| Procédure recommandée | Template inspectable qui propose étapes et contrôles adaptés | Audit ou juge LLM universel ajouté aux workflows de l’utilisateur |
| OwnerRef | Référence au responsable humain ou agent d’une mission | Nouveau niveau d’agent dans la hiérarchie |
