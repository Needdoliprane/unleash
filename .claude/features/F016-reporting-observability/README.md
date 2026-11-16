# F016 — Rapports, preuves et observabilité d'OVPT Unleash

**Statut : spécification V1, non implémentée.** Famille `REQ-OBS`. Le produit doit permettre à Louis de comprendre l'état réel des missions, la base d'une décision, les effets observés et les incertitudes restantes, même après disparition des workers ou fermeture de la GUI.

## Objectif et valeur

Le reporting relie les résultats annoncés aux opérations effectivement observées. Un agent peut écrire « tests réussis » ; le système doit montrer quelle commande a été exécutée, sur quel snapshot, avec quel résultat et selon quel contrat. Une fusion peut avoir eu lieu alors que le daemon s'est arrêté avant d'en enregistrer l'issue ; le rapport doit exposer cette fenêtre et la réconciliation, sans transformer l'incertitude en récit certain.

La feature couvre journal d'activité, catalogue de preuves, manifestes d'invocation et de contexte, reçus discovery, rapports déterministes, résumés optionnels, métriques, traces, flux d'événements et exports. Les vues GUI/CLI/TUI utilisent les mêmes projections et autorisations. Le daemon poursuit son activité lorsque les clients se déconnectent.

## Principes

Observation, claim et judgment sont séparés. L'observation décrit un fait capturé dans un périmètre donné ; la claim décrit ce qu'un acteur affirme ; le judgment expose la décision sur le contrat effectif. Le reporting conserve leurs désaccords et les liens de preuve. Il n'affirme ni absence de bugs ni garantie exactly-once des effets externes.

Une invocation durable relie agent logique, rôle et version, session, modèle réellement utilisé, workflow/skills effectifs, scope, documents, actions, décisions, livrables et parent. Le manifeste de contexte montre les éléments réellement envoyés au modèle, y compris compactions et recherches, sous réserve des autorisations. Aucun raisonnement privé du modèle n'est demandé ou simulé.

La recherche avant création produit une preuve procédurale transversale. Un rapport permet de voir les candidats trouvés, le choix réutiliser/créer, la justification de domaine, l'antériorité de la recherche et sa fraîcheur. Une trace produite après création reste identifiée comme telle. La qualité d'une recherche n'est pas présentée comme preuve mathématique d'absence de doublon.

## Limites et stockage

Le journal durable est nécessaire à la reprise ; les métriques échantillonnées et traces techniques ne le remplacent pas. Les archives et rapports vivent dans les répertoires runtime configurés ; `.claude/` contient la conception. La rétention distingue preuves requises par contrats actifs, logs de diagnostic, captures dirty et exports. Une suppression autorisée laisse un état explicite d'indisponibilité plutôt qu'un faux lien téléchargeable.

La minimisation et le masquage réduisent le risque de secret dans les logs ; aucun filtre ne garantit de reconnaître toute transformation d'un secret. Les exportations externes sont désactivées par défaut et disposent d'une configuration explicite de périmètre. Les [exigences](requirements.md), [conception](technical-design.md), [interfaces](interfaces.md), [dépendances](dependencies.md), [acceptations](acceptance.md) et [tests](tests.md) précisent les garanties à construire. La référence commune est [traceability](../../specs/traceability.md).
