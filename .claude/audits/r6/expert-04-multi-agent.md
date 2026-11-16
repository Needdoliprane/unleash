# R6 — Expert 04 — Coordination multi-agent

**Revue documentaire indépendante, 16 novembre 2026, avant corrections R6.** Périmètre : protocole multi-agent, hiérarchie, goals, scheduling, sessions, événements, états, Role Registry et F004/F007. Aucun runtime ni test produit exécuté. Deux écarts concrets ; aucun quota de constats recherché.

## Constats confirmés

Les contrats distinguent correctement N+2 organisationnel, N+1 responsable de domaine et N exécutant ; un N spécialisé directement créé par N+2 conserve mission, owner et scope. Les managers proposent, le harness applique. Identités dormantes, plafond partagé 5/50, attente durable parents/enfants, révocation d’epoch, ACL des messages et supervision publiée sont explicités dans [agent-hierarchy](../../specs/agent-hierarchy.md), [scheduling](../../specs/scheduling.md) et [event-model](../../architecture/event-model.md). Les missions généralistes peuvent rester sans agent ni dépôt. Ces invariants ne nécessitent pas de réorganisation.

## R6-E04-01 — Décision de clôture de mission sans contrat exécutable

**Priorité élevée ; raccord documentaire manquant.** [goal-management, « Succession, échec et annulation »](../../specs/goal-management.md) exige une décision humaine ou `owner_agent` après terminalité du run. [F004/interfaces](../../features/F004-hierarchical-orchestration/interfaces.md) interdit au client d’imposer `status=succeeded`. Pourtant le [catalogue client, « Mutations et opérations longues »](../../architecture/client-operation-catalog.md) ne définit que Assign/Revise/Pause/Resume/CancelMission ; `AcceptStep` porte une étape. Aucun contrat ne décrit la soumission et l’application du jugement de clôture de mission, ni son raccord à ces opérations.

**Scénario et risque.** Une procédure sans agent termine sous `forbidden`, avec `completion_policy=human`. L’utilisateur peut examiner les preuves mais la commande canonique permettant son verdict reste indéterminée. Deux équipes peuvent fabriquer une mutation d’état générique, réutiliser indûment AcceptStep ou laisser la mission bloquée. Le problème concerne aussi l’owner agent : son jugement doit produire une commande validable, indépendante de la fin du run.

**Objection examinée.** ReviseMission pourrait porter ce jugement. Son contrat décrit actuellement mandat, mode de clôture et dépendances ; aucun discriminant de décision, précondition ou résultat ne permet de confirmer cette interprétation. Ce n’est donc pas uniquement un nom manquant dans un index.

**Recommandation.** Spécifier une opération unique de décision de mission, avec mission/révision, révision des critères, owner/epoch attendus, verdict typé, evidence bundle et justification. Définir les transitions permises, les vérifications de done et de quiescence, l’autorité humaine/agente, l’événement et le reçu atomiques. La politique mécanique appelle le même domaine sans inventer un appel modèle. **Alternative :** une variante explicitement discriminée de ReviseMission, sans affectation libre du statut.

**Acceptation.** Compléter T-F004-008/011 : clôture humaine d’un replay avec zéro appel modèle ; clôture owner_agent autorisée et bornée ; jugement périmé après réaffectation refusé ; effet inconnu bloquant le done non masqué ; redelivery restituant le même reçu ; GUI/TUI passant par le même contrat. Tester aussi refus d’autorité et décision négative sans succès artificiel.

## R6-E04-02 — Borne sans progression non définie entre réveils

**Priorité moyenne ; précision indispensable à REQ-ORCH-006.** [F004/requirements](../../features/F004-hierarchical-orchestration/requirements.md) exige explicitement une borne configurable avec valeur par défaut documentée. [agent-hierarchy, « Invocations événementielles »](../../specs/agent-hierarchy.md) borne les décisions successives ; [event-model, « Garanties et tests »](../../architecture/event-model.md) borne les invocations « par cause ». Les documents ne fixent ni valeur initiale, ni identité persistante du compteur, ni règles de remise à zéro entre événements et invocations. T-F004-006 annonce NoProgressLimit, sans oracle chiffré ni scénario de reprise du compteur.

**Scénario et risque.** N+1 escalade ; N+2 renvoie une demande reformulée ; chaque réponse crée un nouvel événement et une invocation bornée. Un compteur par invocation ou cause immédiate repart à zéro à chaque tour. Le plafond simultané reste respecté, mais l’arrêt sans progression promis peut ne jamais intervenir avant épuisement du budget global.

**Hypothèse et objection.** Le risque suppose une implémentation possible, non un comportement observé. Les budgets supérieurs rendent la dépense finie ; ils ne remplacent pas le garde-fou distinct demandé pour le ping-pong managérial. L’audit initial définit déjà la progression comme preuve nouvelle ou mutation utile : il faut rendre cette règle opérationnelle et durable.

**Recommandation.** Définir une politique versionnée avec seuil initial explicite, clé de chaîne de décision conservée entre niveaux, compteur transactionnel, progression admissible et conditions de réarmement. Nouveau message, paraphrase, redelivery ou nouveau worker ne réinitialisent pas seuls le compteur. À épuisement : demande visible, suspension des réveils de cette chaîne et escalade bornée autorisée ; aucune dérogation à `forbidden`. **Alternative :** plafond cumulatif par mission, plus conservateur mais susceptible de bloquer des décisions indépendantes.

**Acceptation.** Étendre T-F004-006 : alternance N+1/N+2 avec IDs nouveaux, doublons, crash/redémarrage et escalade finale ; nombre total conforme au seuil, compteur conservé et zéro escalade récursive. Une preuve réellement nouvelle réarme selon la règle publiée, sans effacer l’historique.

## À vérifier après correction

Concordance F004, catalogue, machine de mission, politique de réveil et tests ; couverture des erreurs et des reçus. Les propositions restent à qualifier dans le futur produit. Aucun changement des niveaux, permissions, limites globales, bindings immuables ou règles de replay n’est recommandé.
