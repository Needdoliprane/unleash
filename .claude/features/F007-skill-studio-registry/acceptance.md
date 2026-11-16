# F007 — Acceptation

| AC | Given / When / Then |
|---|---|
| AC-F007-001 | Étant donné plusieurs similaires, quand une création est choisie, alors reçu, candidats et justification sont conservés ; la présence d'un similaire n'impose pas réutilisation. |
| AC-F007-002 | Étant donné draft NL incomplet, quand il est édité manuellement, alors hypothèses, provenance et historique restent inspectables et la publication attend les champs critiques. |
| AC-F007-003 | Étant donné skill publié, quand seul l'outil ou l'ACL déclarée change, alors une nouvelle version du bundle est requise ; l'ancien digest reste identique. |
| AC-F007-004 | Étant donné délégation N+2 vers N rédacteur, quand le draft est rendu, alors N+2 peut publier dans ses grants ; une capacité supérieure demandée ne s'auto-autorise pas. |
| AC-F007-005 | Étant donné fixtures déterministes suffisantes pour la politique, quand publication est demandée, alors aucun second juge LLM n'est imposé et le rapport affiche couverture et limites. |
| AC-F007-006 | Étant donné v1 utilisée et v2 candidate, quand Studio ouvre le diff, alors prompt/outils/schémas/ACL/contrats sont comparés et versions en usage distinguées. |
| AC-F007-007 | Étant donné un binding engagé v1 et un slot vierge, quand v2 paraît, alors v1 reste inchangée et v2 peut seulement lier le slot vierge compatible par commande explicite. |
| AC-F007-008 | Étant donné import avec script et référence hors bundle, quand il est inspecté, alors rien ne s'exécute ni ne se télécharge automatiquement et provenance/licence sont préservées. |
| AC-F007-009 | Étant donné grant actif puis révoqué, quand le worker épinglé demande l'effet, alors refus immédiat et historique intact sont observables. |
| AC-F007-010 | Étant donné éditions concurrentes GUI et CLI, quand les publications arrivent, alors mêmes règles et erreurs s'appliquent et chaque client reconnecté voit la même version. |
| AC-F007-011 | Étant donné rôle et skill homonymes, quand un rôle est résolu, alors namespace empêche confusion et l'agent instancié garde sa propre identité. |
| AC-F007-012 | Étant donné rôle complet, quand objectif, modèle, contexte ou skill change, alors le diff et la nouvelle version couvrent le champ changé et ses dépendances. |
| AC-F007-013 | Étant donné homonymes global/projet/repository, quand une recherche non qualifiée s'exécute, alors priorité et alternatives sont montrées ; une référence qualifiée reste exacte. |
| AC-F007-014 | Étant donné N+1 autorisé, quand il compose un rôle après discovery, alors publication est possible dans ses grants et la justification est liée à la mission. |
| AC-F007-015 | Étant donné rôle lié et versions d'évaluation épinglées avant contrôle, quand skill v2, nouvel évaluateur ou crash survient, alors reprise et retries utilisent mêmes bindings/compiler/evaluator/catalogue ; une version épinglée indisponible bloque sans latest implicite. |
| AC-F007-016 | Étant donné rôle incompatible par entrée absente/non membre, digest input/output différent, règle obligatoire retirée ou capacité manquante, quand BindRole démarre, alors aucun binding partiel n'est créé et chaque incompatibilité est nommée. |
| AC-F007-017 | Étant donné trois agents même rôle/skill, quand ils exécutent leurs missions, alors scopes, sessions, manifests et preuves sont séparés. |
| AC-F007-018 | Étant donné version historique puis purge autorisée, quand une inspection est demandée, alors tombstone et limite de reconstitution remplacent le contenu supprimé sans fausse preuve. |

Les scénarios C et E du mandat sont couverts avec F004 et F008. Chaque critère se fonde sur observations et reçus, pas sur une description de succès produite par le modèle.
