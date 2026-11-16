# F016 — Critères d'acceptation

Tous les scénarios restent **à exécuter**. Les résultats doivent être examinables dans les trois clients et dans l'export structuré lorsqu'ils relèvent d'une même autorisation.

| ID | Given / contexte | When / action | Then / résultat observable |
|---|---|---|---|
| AC-F016-001 | Une invocation avec rôle, modèle, skills, workflow et documents précis se termine | Supprimer le worker et redémarrer le daemon | Identités, versions, parents, actions et livrables restent inspectables |
| AC-F016-002 | Un agent affirme des tests réussis alors qu'une observation constate l'échec | Construire le rapport et rendre un jugement | Claim originale, observation et décision restent distinctes ; contradiction visible |
| AC-F016-003 | Un test passe sur un ancien snapshot, avec sortie tronquée | Demander validation d'un candidat nouveau | La preuve garde ancienne portée et troncature ; aucune promotion implicite au nouveau candidat |
| AC-F016-004 | Créations avec reçu préalable, reçu périmé et recherche a posteriori | Évaluer la clôture contractuelle | Trois situations distinguées ; justification reuse/create et sources accessibles ; absence jamais masquée |
| AC-F016-005 | Une transition produit un événement diffusé deux fois | Réappliquer après crash | Une transition et un événement durable ; consommateurs sans double effet |
| AC-F016-006 | Le contexte demandé subit compaction, filtre ACL et troncature | Inspecter l'invocation réellement envoyée | Ordre, contenu autorisé, sources et différences visibles ; aucune prétention au raisonnement privé |
| AC-F016-007 | Cinq invocations actives, vingt leaders dormants, auxiliaires et retry API | Lire métriques et rapport de coûts | Compteurs distinguent catégories ; unknown reste unknown ; estimation séparée de facturation |
| AC-F016-008 | Des secrets synthétiques apparaissent dans champs structurés et flux libres | Afficher, exporter et envoyer un nouveau contexte après révocation | Champs connus masqués, ACL respectée, limites des flux libres explicites et aucun export brut par défaut |
| AC-F016-009 | Une mission done=merge n'a demandé aucun test | Générer deux rapports sur la même séquence et politique | Résultat déterministe, cible/livrable exacts, tests not_requested et risques non vérifiés visibles |
| AC-F016-010 | GUI déconnectée, CLI active et curseur ancien | Reconnecter GUI puis TUI après purge de fenêtre | Même état métier au watermark ; resync explicite si nécessaire ; daemon continu |
| AC-F016-011 | Le journal devient indisponible juste avant ou après un effet | Demander une nouvelle action puis redémarrer | Nouvelle action non journalisable bloquée ; effet déjà parti réconcilié ou inconnu, sans résultat inventé |
| AC-F016-012 | Un rapport inclut une preuve bientôt supprimée et un contenu révoqué | Appliquer rétention puis exporter | Octets visés retirés du périmètre contrôlé, tombstones et masquage présents, aucun lien prétendu valide vers preuve disparue |

## Critère de confiance utilisateur

Le scénario F est accepté quand Louis peut partir d'une claim de test, ouvrir l'observation correspondante, vérifier commande et snapshot, puis comprendre le jugement de N+1. Le scénario O est accepté quand GUI, CLI et TUI convergent par le même service après déconnexion. Le reporting doit rester utile même si le provider est indisponible et ne peut rédiger aucun résumé.
