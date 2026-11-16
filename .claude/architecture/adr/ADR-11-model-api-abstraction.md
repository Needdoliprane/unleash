# ADR-11 — Abstraction des modèles API

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : clients écrits par Unleash, clés BYO et profils recommandés (Q-07), catalogue de capacités et de prix (Q-14) ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Les fournisseurs diffèrent en outils, schemas, effort, contexte, rétention et usage. Les aliases du brief ne sont pas structurels.

Exigences : REQ-MODEL. Risques/hypothèses/questions : R-04 R-20 ; H-14 ; Q-07 Q-14.

## Options et compromis

Option recommandée : **Ports petits natifs OpenAI/Anthropic + profils compatibles qualifiés**.

Alternatives considérées : SDK orchestration fournisseur ; dialecte unique universel.

Ports natifs préservent capacités ; SDK peut aider sous adaptateur ; universel gomme les restrictions et invente équivalence.

## Décision

Router résout priorités puis contraintes, réserve budget, assemble contexte et traduit protocole. Instances et profils versionnés, aucun abonnement CLI. Fallback soumis autorisation de données.

## Amendement R7

1. La décision est adoptée : ports natifs OpenAI Responses et Anthropic Messages, profils compatibles déclarés et qualifiés (Chat Completions), instances et profils versionnés, aucun abonnement CLI, fallback seulement par liste autorisée.
2. Les clients sont écrits par Unleash : `reqwest` avec `rustls` et vérification des certificats par le magasin de la plateforme, parseur SSE interne borné, DTO `serde` stricts. Aucun SDK fournisseur ni passerelle tierce dans le chemin critique.
3. Comptes (Q-07) : clés API fournies par l’utilisateur (BYO), stockées par référence dans le coffre OS ; aucun compte, proxy ni clé OVPT. Profils recommandés `gpt-6.1-sol` (développement) et `claude-fable-5-2` (audit), actifs seulement après `TestProviderProfile` réussi ; budgets de départ prudents et visibles ; fallback vide par défaut ([model-routing](../../specs/model-routing.md)).
4. `context_mode=explicit_unleash_history` est le seul mode, avec `store=false` sur OpenAI Responses ; les outils hébergés sont désactivés par défaut.
5. Catalogue (Q-14) : chaque release livre un `ModelCatalogSnapshot` versionné et signé avec elle ; import manuel ou saisie de tarif par `SetProviderProfile` ; aucune récupération réseau automatique ; validité et état périmé visibles ([F012](../../features/F012-model-api-router/technical-design.md)). Une capacité déclarée par le catalogue n’admet jamais seule une capacité requise.

## Conséquences, complexité et impact V1

Maintenir fixtures et catalogue capacités/prix versionné par release (Q-14 : import explicite, validité 90 jours par défaut, tarif périmé bloquant par défaut) ; pas de garantie d’accès réel avant probe. Les sorties refusées/tronquées restent des états distincts.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

F012 protocole/streaming/budgets, modèle incompatible avant réseau.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/model-routing.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

