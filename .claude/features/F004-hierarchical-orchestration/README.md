# F004 — Orchestration hiérarchique

## Objectif et valeur

OVPT Unleash doit permettre à un utilisateur de déléguer plusieurs features tout en conservant une organisation inspectable : objectifs, responsables, travaux, preuves et blocages. Cette feature réalise les relations N+2/N+1/N, le cycle de décision borné des leaders et les commandes métier qui relient leur jugement au harness. Elle évite de confondre un agent logique durable avec un processus temporaire ou un appel fournisseur.

N+2 organise, arbitre les missions et porte la responsabilité des skills. N+1 choisit une stratégie de réalisation, compose des rôles, affecte les scopes et juge les résultats. N travaille sous contrat. Un N spécialisé peut être créé directement par N+2 avec mission, owner et périmètre explicites. La même compétence peut être instanciée par plusieurs agents sans mélanger sessions ni autorisations.

## Périmètre V1

La feature couvre arbre organisationnel, création et réaffectation, goals et missions, mailbox des leaders, décisions sur evidence bundle, visibilité atomique des messages humains et coopération avec le scheduler. Les vingt leaders logiques du scénario I restent possibles avec le pool global de cinq unités d'exécution actives par défaut (invocations agentes de tous niveaux, workers de replay et probes fournisseur, [scheduling](../../specs/scheduling.md)). Les invocations de managers comptent dans ce plafond, qui ne peut excéder cinquante. Les processus de tests, terminaux et autres auxiliaires ont des budgets séparés.

L'autonomie de décision ne donne pas au modèle un accès direct aux tables d'état. Il émet des commandes, le harness contrôle et applique. Les prompts n'embarquent pas un deuxième moteur de transition. Les appels de leaders sont courts et bornés ; la session persiste pendant leur sommeil. L'intégration de branches peut être confiée à un N spécialisé créé par N+2 ; l'arbitre métier n'est pas automatiquement le résolveur de code.

## Limites et décisions

La feature ne garantit ni la qualité universelle du plan ni l'absence de conflits sémantiques. Elle fournit les identités et preuves nécessaires pour les traiter. Un scope n'est pas une sandbox OS. Le cycle est le run terminal (Q-03) ; aucune migration de version en place n'existe en V1. Les détails du scheduling et de l'isolation relèvent de leurs services dédiés.

Lire [exigences](requirements.md), [design](technical-design.md), [interfaces](interfaces.md), [dépendances](dependencies.md), [acceptation](acceptance.md) et [tests](tests.md). Les contrats communs sont [agent-hierarchy](../../specs/agent-hierarchy.md) et [goal-management](../../specs/goal-management.md). Le [rapport Expert 4](../../audits/multi-agent-coordination.md) expose risques, objections et alternatives. Tous les résultats d'acceptation de ce dossier sont des preuves attendues, aucune implémentation n'étant fournie.
