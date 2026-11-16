# F004 — Critères d'acceptation

Chaque critère exige un résultat observable par les services et par une vue cliente reconnectée. Les preuves attendues ne sont pas encore produites sur un produit.

| AC | Given / When / Then |
|---|---|
| AC-F004-001 | Étant donné un arbre valide, quand une commande crée un cycle, un quatrième niveau ou deux owners effectifs, alors elle est rejetée sans modifier arbre, mission ni révision ; les trois erreurs sont distinguables. |
| AC-F004-002 | Étant donné N+2 autorisé, quand il crée un N d'intégration avec mission, owner et scope, alors agent et session sont durables et dormants ; omettre l'un des champs obligatoires empêche la création entière. |
| AC-F004-003 | Étant donné deux décisions sur la même révision et des bundles précis, quand elles arrivent concurremment, alors une seule mutation incompatible est appliquée ; l'autre retourne conflit et conserve sa proposition inspectable. |
| AC-F004-004 | Étant donné vingt leaders logiques et cinq N actifs, quand une urgence manager survient, alors aucun sixième slot n'apparaît ; la politique de priorité et l'attente sont visibles ; configurer 51 est refusé. |
| AC-F004-005 | Étant donné un message humain N+1, quand le daemon s'arrête aux points de commit, alors message et visibilité N+2 existent ensemble ou pas du tout après reprise ; aucune pièce hors ACL n'est révélée. |
| AC-F004-006 | Étant donné logs ordinaires et événement urgent, quand la politique est évaluée, alors les logs peuvent être coalescés sans appel ; l'urgence est planifiée et chaque invocation s'arrête à son budget ou après lot borné. |
| AC-F004-007 | Étant donné un worker actif, quand la mission change d'owner, alors epoch avance ; une action nouvelle sous l'ancien epoch est refusée et les effets déjà engagés sont listés pour réconciliation. |
| AC-F004-008 | Étant donné un goal avec deux missions, quand une mission et ses steps réussissent mais l'autre échoue, alors le goal ne devient pas automatiquement achieved ; son jugement et ses preuves restent distincts. |
| AC-F004-009 | Étant donné un besoin de compétence, quand N+1 recherche puis réutilise ou crée un rôle, alors reçu, candidats, choix et justification sont conservés ; un nouveau rôle compatible peut lier un slot vierge sans changer un slot engagé. |
| AC-F004-010 | Étant donné un conflit textuel ou sémantique signalé, quand N+2 arbitre la stratégie, alors une mission N spécialisée réalise l'intégration et rapporte les preuves ; l'arbitrage et le travail de résolution ont des auteurs distincts. |

Les scénarios A, C, G, I et J du mandat sont couverts par cette feature en coopération avec stockage, Git et scheduler. La présence d'un résultat LLM plausible ne suffit à aucun de ces critères ; les identités, révisions, commandes et observations attendues sont inspectables.

## AC-F004-011 — Mission sans dépôt ni agent

Exigence : REQ-ORCH-011.

- Étant donné : Une procédure publiée et des CSV sont sélectionnés par un utilisateur sans agents configurés.
- Lorsque : Créer la mission puis exécuter et clôturer selon critères observables.
- Alors : La responsabilité est unique, aucun Git artificiel ni invocation n’est créé ; sous forbidden le compteur modèle reste zéro, rapports compris.
- Preuve attendue : EV-F004-011, environnement, versions, observations et limites. Statut : non exécuté.

## AC-F004-012 — Démarrage guidé et contrôle par blocs

Exigence : REQ-ORCH-012.

- Étant donné : Un template versionné définit un supervisor et trois blocs, avec deux points de revue permis.
- Lorsque : Appliquer deux fois le setup avec la même clé puis exécuter en mode supervisé.
- Alors : Une seule organisation est créée ; les invocations n’excèdent pas la politique, les blocages inconnus restent réconciliés, le manager ne clique pas étape par étape.
- Preuve attendue : EV-F004-012, environnement, versions, observations et limites. Statut : non exécuté.

### Oracles complémentaires R6

AC-F004-006 : huit tours sans progression au défaut proposé, y compris alternance N+1/N+2 avec événements neufs, puis aucune neuvième admission. Crash et redelivery n’effacent ni ne doublent la réserve ; une preuve admissible réarme le compteur consécutif, jamais les coûts cumulés. AC-F004-008/011 : un replay terminé sous forbidden et clôture human peut réussir via RecordMissionDecision sans aucun appel modèle ; critères faux/inconnus, ancien owner/epoch et bundle périmé refusent la transition. RecordGoalDecision ne déduit pas achieved du seul nombre de missions closes.
