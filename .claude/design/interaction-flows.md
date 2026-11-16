# Parcours d’interaction et points de vérité

## Créer et suivre un goal

L’utilisateur choisit projet, objectif, contraintes et workflow ou demande sa création. Le backend enregistre GoalCreated. Selon la source choisie, il prépare template/procédure ou, si une décision agente est nécessaire et autorisée, admet le N+2 selon capacité, qui propose missions et dépendances ; un goal purement mécanique n’invoque aucun agent ; la GUI montre le plan engagé puis les équipes. Aucun agent dormant n’occupe un slot visuellement. Les résultats conduisent aux rapports, puis à la condition de fin déclarée.

## Réutiliser un helper non intégré

Un N lance discovery ; le résultat indique repo, branche, snapshot et état « non disponible ici ». Le N choisit coordination/attente/transfert contrôlé ou nouvelle implémentation justifiée, enregistrés comme décision reuse ou create avec stratégie explicite. La décision lie le DiscoveryReceipt et les candidats. L’utilisateur peut ouvrir la provenance et la dépendance sans croire le helper déjà importé.

## Modifier un workflow

Créer draft v2 depuis v1, éditer, valider, simuler puis publier. Le run v1 continue avec sa définition jusqu’à son état terminal ; l’UI indique sa version. À la fin terminale du run (Q-03, [ADR-04](../architecture/adr/ADR-04-workflow-semantics.md)), seul un nouveau run créé avec la référence exacte de v2 et `predecessor_run_id` adopte v2 ; aucune migration en place n’existe en V1. Les effets incertains du prédécesseur gardent leurs barrières sur les actions concernées. Modifier un binding déjà engagé suit la même prudence ; un slot tardif vierge peut être lié explicitement.

## Réagir à un crash

La fenêtre voit reconciling, puis la mission retrouve sa session et ses preuves. Un outil sûr peut reprendre ; un effet inconnu reste blocage avec les preuves recherchées. L’utilisateur peut inspecter avant d’autoriser une décision de répétition ou d’abandon. La barre d’état n’avance pas pour masquer l’incertitude.

## Publier une compétence

Le besoin déclenche recherche, décision de création, délégation technique éventuelle, draft, tests/évaluations et publication par autorité. Le rôle composé référence une version exacte. Une publication ne modifie pas les invocations déjà actives.

## Équivalence clients

Un goal créé CLI apparaît GUI ; un message GUI apparaît TUI ; la fermeture d’un client ne modifie pas mission. Les reçus et événements sont identiques. Scénarios obligatoires A–O dans [acceptance-scenarios](../testing/acceptance-scenarios.md).

## Exécuter un parcours complet uniquement depuis le terminal

Créer un projet et son goal ; composer un rôle depuis des skills existants ; éditer, valider et publier un workflow ; configurer fournisseur et connecteur ; sélectionner des documents ; lancer une mission ; intervenir auprès du N+1 ; examiner contexte, preuves, diff et intégration ; demander un rapport. Le parcours utilise les mêmes commandes backend que la GUI et ne requiert pas de l’ouvrir. Le navigateur de consentement fournisseur est autorisé depuis les deux clients. Reprendre après perte du terminal doit retrouver reçus et état engagé, sans inventer l’issue d’une commande.

Ce parcours complète les tests unitaires de widgets : il est exécuté sur backend réel avec comparaison d’état normalisé, permissions et causalité. Une seule opération manquante dans la TUI empêche de déclarer la parité complète.
