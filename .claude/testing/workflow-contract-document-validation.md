# Contrôle documentaire — workflows, agents, skills et contrats

**Contrôle historique réalisé le 16 novembre 2026 sur la révision antérieure à R3 ; les comptes et contrôles courants font foi dans [document-validation](document-validation.md).** Ce contrôle vérifie les artefacts de conception du périmètre F004–F008. Il ne vérifie aucune implémentation Rust, daemon, adaptateur, autorisation réelle ou réponse de modèle. Les tests T-F004 à T-F008 restent planifiés.

## Résultats observés

- Quatre schémas valides selon le métaschématique JSON Schema Draft 2020-12 : workflow, skill, rôle, profil contractuel.
- Neuf fixtures JSON conformes à leur schéma, dont deux workflows, trois skills, trois rôles et un profil.
- Références des rôles vers leurs skills/contrats résolues dans les fixtures ; entry_skill exacte et membre de skills.
- Inputs de toutes les tâches conformes au schéma de l’entrée du rôle choisi. Le slot tardif intégrateur est vérifié contre la fixture de rôle compatible.
- Digests input/output et règles obligatoires du profil comparés aux vrais objets de la fixture intégrateur ; aucune chaîne de digest simplement crue.
- Branches et identifiants de nœuds uniques ; bornes calculées de 11 activations pour feature-workflow et 5 pour merge-only, sous leurs budgets de 100.
- 58 exigences uniques, chacune reliée à un AC, un T et un EV présents dans les fichiers correspondants.
- Cinq features avec sept fichiers chacune et plus de 1 500 mots utiles chacune.
- 107 liens relatifs contrôlés dans le périmètre, aucun lien manquant au moment de ce contrôle.

## Contre-exemples documentaires exercés

- champ eval inconnu : rejet attendu observé.
- guard hors projection : rejet attendu observé.
- boucle sans borne : rejet attendu observé.
- quorum sans seuil : rejet attendu observé.
- rôle sans entrée : rejet attendu observé.
- validateur inconnu : rejet attendu observé.
- input incompatible : rejet attendu observé.
- output incompatible : rejet attendu observé.
- règle obligatoire retirée : rejet attendu observé.
- règle obligatoire affaiblie : rejet attendu observé.
- entrée non membre : rejet attendu observé.

Les refus de schéma sont distincts des contrôles sémantiques limités de ce contrôle documentaire. Le compilateur Rust futur doit encore implémenter toute l’analyse de domination, de portée, de permission, de coût, de concurrence et de reprise. La compatibilité documentaire vérifie des digests et règles ; elle ne constitue pas un moteur de contrats de production.

## Cohérence après seconde revue

Les enums Agent/Invocation suivent session-model ; la reprise conserve invocation_id et change attempt_id. Discovery utilise decision reuse/create et strategy séparée. Le workflow reste immuable au run ; un slot tardif peut seulement recevoir un binding encore vierge, puis reste épinglé. L’entrée du rôle est unique et la compatibilité tardive V1 compare les schémas par digest exact avec conservation des règles obligatoires. Compiler/evaluator/catalogue sont épinglés avant évaluation. Le changement d’évaluateur après crash est un scénario d’acceptation produit encore à exécuter, pas une preuve réalisée ici.

## Limites et risques ouverts

La frontière de cycle/migration, ouverte lors de ce contrôle, est fixée depuis R7 par Q-03 : le Run est la frontière et la migration en place est écartée de la V1. Le protocole externe AI-Guard n’est pas fourni. Le choix et la conformité d’une crate Rust JSON Schema/JCS restent à vérifier. Les identifiants de modèle et outils des fixtures sont internes/fictifs, sans affirmation de disponibilité fournisseur. Aucun effet réel Git, processus ou réseau n’a été exécuté par les fixtures.
