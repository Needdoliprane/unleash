# Audit 07 — Knowledge et intelligence de code

**Date :** 16 novembre 2026. **Objet :** évaluer les promesses de connaissance partagée avant la spécification. **Limite :** aucune mesure de rappel, de latence ou de justesse n'a été réalisée. [Protocole expert](../agents/knowledge-expert.md).

> **Note R7 (16 novembre 2026) :** addendum R2 manquant. Ce rapport conserve les constats initiaux. Les dépendances natives inventoriées sont autorisées ([politique](../brief/implementation-policy.md), Q-01 résolue) : Tree-sitter n’est plus écarté pour son runtime C11 ; il est retenu en R7 pour le niveau syntaxique (Q-13, [ADR-08](../architecture/adr/ADR-08-knowledge-index.md)), version, licence, grammaires et frontière FFI restant à figer ([SRC-GK-12](../research/git-knowledge-sources.md)). Les niveaux de capture courants sont ceux de [shared-knowledge](../specs/shared-knowledge.md) (`git_tree_coherent`, `atomic_fs_coherent`, `immutable_version_coherent`, `unverified`, `inconsistent`).

## Verdict

Le moteur est utile s'il expose des sources, des limites et des intentions de travail qui aident N+1 à décider. Il devient dangereux s'il transforme une proximité lexicale, un résultat LSP incomplet ou un résumé ancien en vérité globale. La frontière structurante est la séparation entre code versionné, activité observée et connaissance dérivée. Les trois sont reliés ; ils ne sont pas interchangeables.

## Constats et objections prioritaires

| Réf. | Constat ou objection | Conséquence et proposition |
|---|---|---|
| KNOW-R01 | Une branche nommée ne désigne pas un contenu immuable. | Toute réponse cite repo, commit/tree, manifeste dirty, génération et version d'analyseur. Une requête sur « dernière version » retourne aussi la révision du catalogue. |
| KNOW-R02 | Une capture dirty séquentielle peut mélanger des fichiers à des instants différents. | Distinguer octets archivés et cohérence temporelle ; `unverified` par défaut, `inconsistent` si mutation détectée. Pas de promotion automatique en `coherent` après deux scans égaux. |
| KNOW-R03 | Le produit Rust n'implique pas des dépôts cibles uniquement Rust. | Python/TypeScript/Java/Go/Rust et multilangage sont confirmés V1 ; profils LSP externes supervisés, profondeur et dégradation déclarées, aucune équivalence universelle de couverture. |
| KNOW-R04 | Un moteur syntaxique ne résout pas tous les symboles, macros, flags et dépendances. | Qualifier chaque relation : `semantic_resolved`, `syntactic`, `lexical`, `declared`, `unknown`. Les relations absentes ne sont pas des preuves d'absence. |
| KNOW-R05 | Un nom similaire n'implique pas la même politique métier. | `normalizeEmail` et `cleanEmail` sont des candidats à examiner ; Unicode, casse, espaces, erreurs et effets de bord sont comparés avant choix. |
| KNOW-R06 | Le code non mergé peut être privé ou temporairement cassé. | ACL en ingestion et en restitution, badges nonmergé/dirty/ancien, revalidation d'accès avant envoi au modèle. |
| KNOW-R07 | Une recherche globale peut révéler l'existence de symboles interdits par ses compteurs. | Filtrer avant classement, agrégation, extrait et export ; pas de total des résultats exclus. Tester les caches après révocation. |
| KNOW-R08 | L'indexation profonde peut exécuter du code du dépôt. | Désactiver par défaut les actions non autorisées du serveur, utiliser un budget auxiliaire et le broker ; fallback lisible si configuration interdite. |
| KNOW-R09 | « Attendre le helper » peut produire un cycle ou une attente indéfinie. | Graphe d'attente durable, détection de cycles, propriétaire, condition de réveil et expiration métier. |
| KNOW-R10 | Rebuild de l'index ne doit pas transformer une inférence en observation. | Tables et provenance séparées ; seules les projections dérivées sont remplaçables. |
| KNOW-R11 | Une recherche uniquement conseillée disparaît sous pression de livraison. | Opérations discovery du harness et reçu durable avant création identifiable de code, tests, contrats, skills, rôles ou ressources ; justification explicite de création, contrôlable à la clôture. |

## Approche d'analyse contestée

Le recours direct aux crates HIR de rust-analyzer aurait un accès riche au modèle interne mais créerait une forte dépendance de version. Le [livre d'architecture](https://rust-analyzer.github.io/book/contributing/architecture.html), consulté le 16 novembre 2026, explicite que ces crates ne constituent pas une frontière d'API stable. Un adaptateur LSP supervisé est donc proposé, avec versions et capacités négociées. LSP ne devient pas pour autant une API de graphe complet : références et symboles restent conditionnés au projet effectivement chargé et à la synchronisation des documents.

La [configuration rust-analyzer](https://rust-analyzer.github.io/book/configuration.html), consultée le 16 novembre 2026, montre que certaines analyses s'appuient sur build scripts et proc-macros. Un workspace fourni par un tiers ne doit donc pas activer ces possibilités implicitement. Les résultats dégradés restent utiles, mais leur confiance doit être moindre et leur contexte de compilation visible. Les noms des réglages, leurs valeurs effectives et les versions sont conservés, sans reproduire une API présumée identique pour toutes les releases.

Tree-sitter serait un bon candidat syntaxique multilangage, mais son [runtime C11 documenté](https://tree-sitter.github.io/tree-sitter/), consulté le 16 novembre 2026, constitue une dépendance native réelle. Il n'est pas sélectionné par une simple crate wrapper Rust. Un adaptateur de parseur purement Rust doit être évalué langue par langue ; tant qu'il ne l'est pas, le mode annoncé est lexical, pas sémantique.

## Expérience de pensée : helper concurrent

Mission A crée `normalizeEmail` sur une branche non mergée ; mission B cherche `cleanEmail`. Le moteur fournit le candidat A, sa signature, ses appels connus, ses tests observés, son contrat déclaré et son snapshot. N+1 de B peut choisir `reuse` si un artefact compatible est déjà accessible ; `coordinate` si les contrats doivent être alignés ; `wait` si B dépend d'une publication précise ; `copy` si une variation locale est intentionnelle. Le moteur enregistre la décision et ses motifs. Il n'impose ni fusion anticipée ni attente simplement parce que les noms ressemblent.

Une copie porte sa provenance, la différence métier et l'intention de maintenance ; elle ne crée pas nécessairement une dépendance de build. Une attente cite une condition vérifiable — par exemple publication de tel OID dans telle cible — et libère la place d'exécution. Une coordination est un événement entre leaders, avec N+2 visible lorsque les priorités intermissions sont en jeu. Une réutilisation doit vérifier le contenu effectivement consommé, pas seulement le nom du symbole suggéré.

## Alternatives

Un index vectoriel universel améliorerait peut-être certains rapprochements, mais suppose un modèle d'embedding, une politique de transmission et un coût que le brief n'a pas établi. Il reste un adaptateur facultatif de V1, avec clés utilisateur et provenance de version, sans devenir une dépendance pour la recherche exacte. La recherche lexicale plus symboles structurés et intentions actives fournit une base déterministe utile. Un graphe alimenté uniquement par les déclarations d'agents serait moins coûteux mais non fiable ; il est réservé au plan « intentions et claims » et réconcilié avec les observations.

## Critères de sortie et questions ouvertes

Avant qualification V1, démontrer qu'un résultat périmé ne remplace jamais la génération récente, qu'une révocation retire les extraits des nouveaux contextes, qu'un code nonmergé est trouvable quand autorisé, et qu'une corruption d'index n'altère pas le journal. Le jeu de pertinence doit contenir synonymes trompeurs, overloads, macros, fonctions sans appel connu et modules conditionnels. Mesurer précision et rappel sur corpus explicitement défini ; ne publier aucune valeur avant mesure.

À confirmer : profondeur d'analyse par langue confirmée, taille maximale des workspaces, horizon de conservation dirty, recherche par embeddings, et convention de dépendances natives embarquées. La règle de recherche avant création doit couvrir les registries autant que le code et rester vérifiable sans prétendre détecter tout acte de création dans un shell opaque. Aucune de ces inconnues ne justifie de présenter les résultats comme « intelligence complète du dépôt ».
