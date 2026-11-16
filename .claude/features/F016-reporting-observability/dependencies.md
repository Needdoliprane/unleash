# F016 — Dépendances et responsabilités

| Producteur ou consommateur | Contrat échangé | Limite essentielle |
|---|---|---|
| Noyau/persistance | État + événements + outbox, séquence monotone | Le reporting ne crée pas une seconde autorité de mission |
| Sessions et invocations | Identités, parents, versions et manifestes | Une session durable ne dépend pas de la RAM du worker |
| Model/API Router | Configuration effective, demande envoyée, usage retourné et erreurs | Pas d'identifiant de modèle inventé ni de coût facturé déduit sans source |
| Terminal et broker | Intention, autorisation, processus, sorties et issue | PID seul ou stdout « success » ne suffit pas |
| [F010 Git](../F010-git-coordination/README.md) | OID, candidats, effets observés et reprise | Ref actuelle présente ne prouve pas toujours l'auteur de l'effet |
| [F009 knowledge](../F009-knowledge-engine/README.md) | Provenance, confiance et reçus discovery | Similarité et fraîcheur n'impliquent pas équivalence métier |
| Workflows/contrats/registries | Versions effectives, règles de clôture, roles/skills | Aucun test ou audit universel ajouté par la projection |
| Sécurité/AI-Guard frontière | ACL, secrets, rétention, autorisations | Le contrat externe AI-Guard reste à confirmer |
| GUI/CLI/TUI | Projections, événements, curseurs et exports | Même backend, mêmes statuts, reconnexion explicite |

OpenTelemetry est une option d'export technique documentée dans [la recherche](../../research/git-knowledge-sources.md), pas un stockage métier obligatoire. Le daemon et la reprise fonctionnent sans collecteur externe. Les versions du SDK Rust retenu doivent être qualifiées par capacité ; une version de spécification ne démontre pas automatiquement la maturité de toutes les implémentations.

Les rapports structurés JSON et les manifestes sont déterministes. Un rendu Markdown, une vue GUI ou un résumé modèle peuvent s'y ajouter sans devenir une source de vérité indépendante. Les exports conservent la politique d'accès, les redactions et les limites des preuves. Une preuve absente doit rester absente dans tous les rendus.

La rétention relève d'un contrat partagé avec persistance et sécurité. F016 expose les conséquences des suppressions ; il ne choisit pas seul une durée universelle ni ne promet une suppression chez des fournisseurs externes. Les sauvegardes et copies exportées sont inventoriées par leur périmètre, avec responsabilités explicites.
