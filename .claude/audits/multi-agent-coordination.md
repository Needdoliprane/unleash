# Audit Expert 4 — Systèmes multi-agents et hiérarchie

**Émis avant les spécifications, 16 novembre 2026.** Regard spécialisé du sous-agent `workflow_agents_contracts`. Ce regard analyse organisation, autorité et coordination ; il ne prétend pas fournir un benchmark. Sources documentaires : [registre](../research/workflow-skills-sources.md), notamment SRC-AG-01 ; les recommandations qui suivent sont des propositions Unleash.

## Constats confirmés

Le brief impose N+2 organisationnel, N+1 chef de mission autonome et N réalisateur. N+2 peut créer directement un N spécialisé lorsque cela est utile. Il entretient l'essentiel des skills, avec une intention exprimée par « environ 99,9 % » qui ne doit pas devenir un quota artificiel. La limite d'exécution vise les invocations actives, cinq par défaut et cinquante au maximum ; elle ne limite pas à cinq le nombre d'identités logiques. Un leader dormant conserve mission, session et boîte d'événements.

Les termes rôle, skill, agent, session et worker portent des responsabilités différentes. Sans séparation, trois exécutants utilisant le même skill risquent de partager à tort une identité ou un worktree. L'arbre organisationnel ne doit pas être confondu avec le graphe de dépendances entre missions. Un propriétaire de mission reste unique, même si plusieurs missions concourent à un goal commun.

## Risques

| Risque | Gravité | Mécanisme de prévention proposé |
|---|---|---|
| R-ORCH-01 : deux orchestrateurs prennent les mêmes décisions | Critique | Leaders proposent ; reducer du harness valide et applique |
| R-ORCH-02 : résultat sur une révision obsolète | Élevée | `expected_revision`, hash d'evidence bundle, rejet explicite |
| R-ORCH-03 : agent N créé directement sans propriétaire | Élevée | Mission, scope, owner et objectif exigés atomiquement |
| R-ORCH-04 : diffusion humaine manquée par N+2 | Élevée | Message, événement et entrée mailbox dans la même transaction |
| R-ORCH-05 : réveil de tous les leaders à chaque log | Élevée | Politique d'événements intéressants, coalescence bornée, quotas |
| R-ORCH-06 : handoff qui laisse un ancien worker autorisé | Critique | Changement d'owner avec nouvel epoch, fencing des anciens effets |
| R-ORCH-07 : priorité permanente N+2 qui affame N | Élevée | Files équitables, vieillissement, réservation sans préemption destructive |
| R-ORCH-08 : conversation privée utilisée comme autorité | Élevée | Toute décision appliquée possède commande, preuves et auteur durables |

## Hypothèses

**H-ORCH-01 proposée.** Une mission possède un owner unique N+1, ou N+2 pour une mission spécialisée directement pilotée. Un N créé par N+2 n'a pas le droit de créer sa propre sous-hiérarchie. L'origine organisationnelle et l'owner de mission sont des relations distinctes, exposées à l'utilisateur. Une réaffectation change les références autorisées ; elle ne falsifie pas l'historique de création.

**H-ORCH-02 proposée.** N+2 et N+1 reçoivent des invocations courtes déclenchées par des événements regroupés. Une invocation produit au plus un lot borné de commandes, pas une boucle autonome qui garde un slot jusqu'à la fin de la mission. Un délai de silence ne prouve pas une panne ; le scheduler considère l'état de l'invocation et du lease.

**H-ORCH-03 proposée.** Le nombre de décisions consécutives sans progression observable a un plafond distinct. Une progression signifie nouvelle preuve ou mutation de mission acceptée, pas un changement de formulation. Ce garde-fou bloque un ping-pong N+1/N+2 qui consommerait la totalité du budget sans avancer.

## Objections et alternatives

**Objection : l'autonomie N+1 devient cosmétique.** Le N+1 garde le choix technique, les décompositions, les skills, les scopes et les verdicts. Le harness ne remplace pas ce jugement ; il empêche une décision d'agir hors du contrat, sur un état périmé ou sur des faits inventés. Un résultat techniquement mauvais reste possible même lorsque les règles formelles sont respectées. L'interface distingue donc conformité et appréciation.

**Objection : toute communication humaine doit réveiller N+2 immédiatement.** Le brief exige visibilité, pas nécessairement coût d'un appel. L'écriture atomique de la mailbox rend le message inspectable instantanément. Une règle peut réveiller N+2 pour conflit, priorité, blocage ou demande explicite, et seulement signaler les autres messages. Cette distinction préserve réactivité et budget.

**Alternative : un N+1 pour toute spécialité directe.** Elle simplifie l'uniformité de l'arbre mais introduit un manager supplémentaire pour l'intégration ou une inspection courte. Elle contredit l'intention d'autoriser N+2 à créer N directement. La proposition conserve trois niveaux conceptuels et rend la mission spécialisée explicite.

**Alternative : conversation partagée pour tous.** Elle offre un contexte commun facile à démarrer mais expose des données hors scope et rend les décisions difficiles à attribuer. Le dossier préfère des manifests de contexte à la demande, un journal commun filtré par ACL et des preuves par snapshot. Aucun raisonnement privé du modèle n'est demandé.

## Recommandations

Instancier agents logiques et sessions indépendamment des processus. Utiliser une enveloppe de décision avec révision, evidence bundle, acteur, mission, autorisations et clé d'idempotence. Vérifier toutes ces données avant toute transition. Le lot est atomique ou rejeté ; aucune moitié de réaffectation ne doit être appliquée. Les propositions d'élargissement du scope sont routées à l'autorité compétente, elles ne constituent pas une extension implicite.

Faire des événements une entrée de planification, jamais une permission. L'événement « workflow v2 publié » informe mais ne remplace pas le workflow du run courant. L'événement « humain a demandé X » conserve l'identité, l'audience et le contenu autorisé ; il ne transforme pas une chaîne de texte en outil exécutable. Un message peut nécessiter une décision N+1, une notification N+2, les deux, ou aucune invocation immédiate.

Le N+2 peut arbitrer un conflit métier puis créer une mission d'intégration pour un N spécialisé. Le spécialiste résout le conflit de code sous scope et contrat ; le N+2 ne devient pas automatiquement réalisateur. Les conflits sémantiques entre branches doivent remonter comme risques même en l'absence de conflit textuel.

## Vérifications et critères

### Addendum de consolidation

L'annexe précise Role Registry première classe et découverte transversale. Le vocabulaire courant de discovery utilise décision reuse/create et stratégie séparée, conformément au [modèle canonique](../architecture/skills-contract-model.md). L'objection au pinning de toutes les compétences au départ est retenue : un slot tardif contraint peut être lié une seule fois avant activation, sans substitution d'un binding engagé. Les états Agent/Invocation et la reprise même invocation_id/nouvel attempt_id suivent [session-model](../architecture/session-model.md), qui fait autorité.

Créer vingt leaders logiques, trois exécutants portant le même skill mais des scopes différents, puis déclencher un mélange d'événements N/N+1/N+2. Mesurer le nombre d'invocations actives par type : il ne dépasse jamais cinq dans la configuration initiale. La configuration refuse cinquante et un. Simuler un résultat sur révision ancienne et une réponse du worker révoqué : aucun effet nouveau n'est autorisé.

Écrire un message humain N+1 et tuer le daemon aux frontières de transaction. Après reprise, le message et la visibilité N+2 apparaissent tous les deux ou aucun des deux ; aucune exposition supplémentaire de contenu hors ACL n'est créée. Vérifier séparément qu'un événement visible ne provoque pas systématiquement un appel LLM.

Le critère d'acceptation est un arbre compréhensible avec propriétaires, scopes et preuves observables ; ni le nombre d'agents ni leur prose ne constitue une mesure de réussite. Les validations futures sont REQ-ORCH. Aucune de ces validations n'est présentée comme exécutée sur Unleash.
