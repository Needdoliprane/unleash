# ADR-07 — Contrats de sortie et preuves

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : catalogue de validateurs v2, règle de verdict, bibliothèques, limites et parseurs de tests fixés ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

Un modèle peut annoncer un test qu’il n’a pas exécuté ; un JSON valide ne prouve pas sa vérité.

Exigences : REQ-CONTRACT REQ-OBS. Risques/hypothèses/questions : R-06 R-22 ; H-07.

## Options et compromis

Option recommandée : **Prédicats fermés sur observations et schémas, claims séparés**.

Alternatives considérées : Second juge LLM obligatoire ; scripts arbitraires de validation ; aucune validation.

Les prédicats vérifient des faits accessibles ; juge sémantique ajoute coût et autorité contradictoire ; scripts peuvent devenir langage d’exécution non contrôlé.

## Décision

Validation format/type/références, receipts de commandes, snapshot/test, discovery et scopes. Contrôle sémantique appartient au N+1 selon workflow. Nouveau prédicat implémenté Rust versionné.

## Amendement R7

1. Un contrat est un ensemble fermé de règles typées que le moteur Rust évalue sur des schémas, des observations authentifiées, des claims et des reçus. Pas de script libre, pas de langage de politique généraliste, pas de juge LLM obligatoire ; le jugement sémantique appartient au N+1 ou à une évaluation explicitement demandée.
2. Catalogue livré en V1 : `validator-catalog` version 2, soit 13 validateurs (les 11 historiques, `test_execution_matches` et `claim_predicate_matches`). La version 1 reste chargeable pour un profil qui l’épingle. Une règle dont le validateur est absent du catalogue épinglé échoue avec `UNKNOWN_VALIDATOR`.
3. Verdict : `fail` dès qu’une règle obligatoire échoue ; sinon `unknown` si une règle obligatoire est inconnue ; sinon `pass`. Une règle obligatoire `unknown` applique son `on_unknown` publié : `block` met le Run en `blocked` (NeedEvidence), `fail` fait échouer l’étape. Le Studio et le compilateur proposent `block` par défaut.
4. JSON Schema 2020-12 par la crate `jsonschema`, sans résolution réseau ; canonisation RFC 8785 par une implémentation existante qualifiée sur les vecteurs de la RFC.
5. Limites par instance évaluée : 1 MiB, profondeur 64, 128 règles par contrat ; au-delà, `SCHEMA_LIMIT_EXCEEDED`. Un artefact plus grand passe par référence et digest.
6. Parseurs de tests V1 : `parser.junit-xml` v1 (pytest, Maven Surefire, Gradle, jest-junit, Vitest, cargo-nextest) et `parser.go-test-json` v1. La sortie texte de `cargo test` donne un parsing `incomplete`, donc un résultat `unknown` ; les projets Rust utilisent cargo-nextest avec un profil JUnit.
7. Tout nouveau validateur, prédicat de claim ou parseur est du code Rust versionné, livré avec fixtures positives, négatives et inconnues et une nouvelle version de catalogue ; il n’est jamais déduit du Markdown d’un skill.

## Conséquences, complexité et impact V1

Garanties limitées à l’observable ; la qualité des tests ou d’un audit reste jugement. Complexité maîtrisée, aucune obligation d’audit universelle.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Claims fabriqués, exit/artefacts absents, preuve d’ancienne génération et contrat de recherche.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/contract-enforcement.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

