# Audit indépendant — Expert 10, sécurité et AI-Guard

Statut : architecture de sécurité proposée, pas audit du code AI-Guard. Sources primaires recensées dans [security-sources](../research/security-sources.md).

## Actifs et adversaires

Les actifs sont les secrets API/OAuth, le code privé, les documents, l’autorité d’exécution, la session, les preuves et le budget. Une menace réaliste vient d’un document ou dépôt contenant des instructions malveillantes, d’un outil MCP compromis, d’une dépendance exécutée pendant build, d’un worker erroné, d’un autre processus local et de la compromission d’un fournisseur. L’administrateur de la machine et le noyau ne sont pas considérés maîtrisables par une application utilisateur.

## Constats et objections

SEC-R1 — Un worktree et un scope de fichiers ne confinent pas un shell. Un processus lancé avec les droits de l’utilisateur peut sortir du répertoire, ouvrir le réseau ou modifier des données accessibles. Un broker qui autorise « lancer shell » n’observe pas chaque syscall ensuite.

SEC-R2 — Un socket 0600 isole des autres utilisateurs, pas automatiquement d’un programme hostile lancé sous le même UID. Un jeton présent dans un fichier lisible par ce même UID ne crée pas de frontière forte supplémentaire.

SEC-R3 — Une consigne « ne révèle pas les secrets » n’est pas une protection. Les clés restent hors contexte et hors environnement des projets, dans le coffre OS ; seul le composant qui effectue l’appel les résout. Le besoin éventuel d’un secret outil reçoit un mandat propre et un mécanisme d’injection borné.

SEC-R4 — Un index central peut devenir un canal de fuite entre projets si la recherche filtre seulement après ranking ou si le cache ne porte pas le scope. Les filtres d’autorisation doivent intervenir avant exposition, y compris les snippets, embeddings et résultats négatifs.

SEC-R5 — Une signature ou hash local ne rend pas un journal inviolable face au propriétaire du compte. L’intégrité opérationnelle et la preuve opposable externe sont distinctes.

SEC-R6 — Si AI-Guard est « requis » et indisponible, laisser continuer les actions transforme la dépendance de sécurité en promesse vide. Si AI-Guard est optionnel, le produit doit afficher précisément le mode local.

## Comparaison des frontières

| Option | Avantage | Limite / risque | Recommandation |
| --- | --- | --- | --- |
| AI-Guard sur actions sensibles classées | Faible latence, déploiement simple | Classification incomplète, effets composites shell | Mode explicitement choisi, couverture visible |
| AI-Guard sur chaque action à effet de bord initiée | Cohérence des décisions et audit | Granularité de commande, pas confinement descendants | Candidat d’intégration principal |
| AI-Guard broker d’exécution obligatoire | Autorité centralisée, possibilité isolation | Dépendance forte, disponibilité et IPC, contrat inconnu | Profil strict après qualification du produit réel |

Ces trois options restent ouvertes. Le port interne d’autorisation permet de les tester sans les prétendre déjà implémentées par AI-Guard.

## Recommandations mécanisme/frontière/autorité

L’autorité utilisateur accorde des capabilities limitées ; rôle et skill ne peuvent que les restreindre. Toute action fixe subject, resource, verb, payload_digest, scope_revision, policy_revision, deadline, idempotency_key et requirements d’isolation. L’autorisation est revalidée juste avant effet ; une révocation interdit les actions suivantes et ouvre une réconciliation des actions déjà en vol.

Le mode standard contrôle les outils typés, journalise les commandes autorisées et explicite la portée des shells. Le mode strict refuse une commande qui réclame confinement si le backend d’isolation ne prouve pas les restrictions nécessaires. Ni une regex shell ni un parser de commande ne suffisent à cette preuve.

Documents, README de dépôts, messages d’outils et skills importés restent des données de confiance distincte. Une instruction dans un document ne peut promouvoir une permission, publier un skill, changer le budget ni modifier les destinataires réseau.

## Points ouverts et critères

Qualifier accès Keychain d’un daemon launchd, verrouillage du coffre, code signing, permissions des fichiers et Linux sans coffre accessible. Tester un faux socket, un retour OAuth avec state différent, une symlink hors scope, une injection par document, une révocation concurrente, un outil MCP mensonger et un broker AI-Guard indisponible.

Acceptation : aucun secret-canari n’entre dans le contexte modèle, les logs normaux ou les rapports ; toute opération refusée laisse une trace sans données sensibles ; une politique required ne passe jamais en local implicitement ; suppression et rétention sont observables avec limites de backups. Voir [F017](../features/F017-ai-guard-integration/README.md) et [tests sécurité](../testing/security-tests.md).

