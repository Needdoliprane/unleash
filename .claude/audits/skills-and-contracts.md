# Audit Expert 8 — Skills, contrats et vérification

> **Addendum R2 — 16 novembre 2026.** Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). Bibliothèques existantes et HTML/CSS sont autorisés ; Kivy et Tauri sont identifiés ; la GUI et la TUI doivent avoir une parité fonctionnelle complète. Les choix actuels sont dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md), [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md). Les autres observations techniques restent à lire avec leurs limites initiales.

**Émis avant les spécifications, 16 novembre 2026.** Regard spécialisé du sous-agent `workflow_agents_contracts`, distinct des regards workflow et coordination par son mandat. Sources : [registre](../research/workflow-skills-sources.md), SRC-CT-01/02/03 et SRC-SK-01. Aucun contrat externe AI-Guard ni code existant n'a été fourni.

## Constats

Le brief demande des skills visibles, éditables, versionnés, créés en langage naturel et généralement entretenus par N+2. Les skills doivent décrire entrées, sorties, outils et traces attendues. Un contrat exploitable doit contenir des contraintes effectivement évaluables. Une instruction comme « produire du code excellent » n'est pas un prédicat exécutable, même si elle figure dans un schéma JSON valide.

Les conventions Agent Skills décrivent un format documentaire. Les descriptions et annotations d'outils MCP ne suffisent pas non plus à autoriser leurs effets. La proposition Unleash doit donc distinguer import de prose, définition de capacités demandées, permission effective et vérification du résultat. Un modèle peut émettre un objet conforme qui contient une affirmation fausse ; une observation peut être authentique mais trop ancienne ou relative au mauvais snapshot.

## Risques

| Risque | Gravité | Conséquence et réponse |
|---|---|---|
| R-CT-01 : prose « compilée » en garantie | Critique | Fausse assurance ; classer les assertions par vérifiabilité |
| R-CT-02 : auteur N+2 augmente sa propre ACL | Critique | Auto-escalade ; intersection des grants, révocation immédiate |
| R-CT-03 : version du prompt change sans contrat | Élevée | Run impossible à reconstituer ; bundle indivisible et digest |
| R-CT-04 : claim de test accepté sur parole | Élevée | Faux succès ; relier action, sortie, snapshot et commande observés |
| R-CT-05 : preuve valide réutilisée sur mauvais commit | Critique | Validation d'un autre code ; lier identités de contenu et contexte |
| R-CT-06 : script importé exécuté par convention | Critique | Code arbitraire hors broker ; imports documentaires inertes |
| R-CT-07 : évaluateur LLM obligatoire pour publier | Moyenne | Coût, faux consensus, blocage ; tests ciblés et jugement optionnel |
| R-CT-08 : révocation absente des runs épinglés | Critique | Persistance d'une permission retirée ; grant effectif relu à l'action |

## Hypothèses proposées

**H-CT-01.** Le skill est publié comme bundle immuable : instructions, contrat, schémas locaux, références d'outils versionnées, capacités demandées, ACL déclarée, provenance et rapport d'évaluation. Chaque binding engagé du run conserve le digest de ce bundle. Les grants effectifs, qui peuvent se restreindre ou expirer, restent des décisions d'autorisation séparées. Cette séparation permet le pinning sans neutraliser une révocation. Après objection de consolidation, un slot tardif encore vierge peut recevoir un nouveau binding explicite et compatible ; ce n'est pas une substitution de version engagée.

**H-CT-02.** Le contrat utilise une liste fermée de validateurs : forme JSON, présence et intégrité d'artefact, portée de chemin, identité de snapshot Git, commande observée et code de retour, relation claim/preuve, résultat de test parsé par adaptateur déclaré, diff de portée, observations d'intégration. Les demandes hors catalogue restent des assertions de jugement ou exigent le développement d'un validateur Rust revu ; elles ne deviennent pas du code évalué depuis le skill.

**H-CT-03.** Un skill peut être publié sans second juge LLM. Il doit passer les contrôles de format, de références, de compatibilité et de permissions, plus les évaluations exigées par sa politique de publication. La qualité sémantique peut être évaluée humainement ou par modèle lorsque le contexte le justifie. Cette appréciation est conservée avec sa méthode et ses limites.

## Objections et alternatives

**Objection : trop de contrats ralentiront la création NL.** Un draft peut rester incomplet et afficher les vérifications manquantes. Des profils de contrat réutilisables permettent de lier rapidement « patch Rust » à scope, snapshot et traces observables. Le compilateur expose les hypothèses ; il ne prétend pas déduire seul l'intention juridique ou métier d'une phrase.

**Objection : sans tests métier universels, la publication serait dangereuse.** Un skill de synthèse, un skill d'intégration et un skill de génération ne nécessitent pas les mêmes évaluations. Les contrôles structurels et d'autorité sont universels ; les tests métier relèvent du contrat et de la politique déclarée. Imposer systématiquement une étape d'audit de code violerait le done configurable du brief.

**Alternative : tout vérifier par un autre LLM.** Ce choix peut enrichir un jugement, mais ne remplace pas un code de retour observé ni une vérification de permission. Deux modèles peuvent reproduire la même erreur ou interpréter le même artefact trompeur. Il ajoute latence et coût. La conception garde cette option explicite sans la rendre obligatoire.

**Alternative : signer les bundles pour garantir leur sûreté.** Une signature peut identifier une provenance suivant son infrastructure de confiance ; elle ne démontre ni innocuité ni correction. L'intégrité par digest est utile dans tous les cas. La signature et la chaîne de confiance restent des mécanismes supplémentaires à spécifier, pas une preuve de qualité.

## Recommandations

Séparer `Observation`, `Claim` et `Judgment` avec identifiants et provenance. Une observation vient d'un adaptateur autorisé ; un claim appartient à son auteur ; un jugement N+1 combine les deux et peut conclure `accept`, `correct` ou `escalate`. Le harness vérifie la conformité et les permissions, puis applique la décision si sa révision et ses preuves sont toujours valides. Un résultat contradictoire est un objet de correction, pas un texte à ignorer.

Effectuer la recherche de skills avant création, sans transformer la similarité en preuve d'équivalence. Conserver licences, versions et raisons d'un fork. À l'import, présenter la prose comme données non fiables, préserver son origine et empêcher les références qui sortent du bundle ou tirent des schémas à distance. La publication est une opération de registre versionnée et idempotente ; elle ne modifie aucun run courant.

Afficher dans le Studio les vérifications exécutables, celles dépendant d'un adaptateur et celles laissées au jugement. Les échecs doivent porter un code, la règle concernée, le sujet et les observations utilisées. L'utilisateur doit pouvoir inspecter pourquoi un claim « tests passés » est accepté, rejeté ou inconnu.

## Vérifications et critères de sélection

### Complément après annexe utilisateur

Le Role Registry est distinct conceptuellement du Skill Registry : objectif, responsabilités, skills exacts, modèle, capacités, permissions, contrats et contexte sont versionnés ensemble. La résolution affiche la priorité repository > projet > global. N+1 recherche puis compose les rôles ; N+2 peut déléguer la rédaction technique d'un skill à N et garder la décision de publication. Les contrats incluent les discovery receipts transversaux, avec choix reuse/create avec stratégie explicite justifié. Les projets traités et leurs outils peuvent être multilangages ; Rust-only ne concerne que le code interne Unleash.

Après seconde revue, la décision de discovery décrit l'identité (`reuse` ou `create`) et sa stratégie est séparée. `adapt` ne constitue donc plus une troisième décision ; les couples et références obligatoires suivent [skills-contract-model](../architecture/skills-contract-model.md). Ce complément remplace la taxonomie initiale et conserve la raison de l'arbitrage : permettre coordination, attente, copie ou adaptation sans confondre identité réutilisée et identité nouvelle.

Soumettre un output parfaitement conforme mais faux, un log hors snapshot, une permission ajoutée par l'auteur, un outil absent, un schéma récursif trop coûteux, une référence réseau et une publication concurrente. Dans chaque cas, contrôler la décision du moteur, la raison et l'absence d'effet non autorisé. Révoquer ensuite un grant sur un run épinglé : le prochain effet est refusé sans réécrire le bundle.

Pour un skill simple, produire les fixtures contractuelles et publier sans second appel de juge LLM ; vérifier que le rapport mentionne précisément les garanties limitées. Les critères sont la lisibilité de la frontière de confiance et la capacité d'équipes indépendantes à produire le même verdict sur les mêmes observations. La sélection des bibliothèques, adaptateurs et politiques reste une étape d'implémentation. Les preuves futures correspondent à REQ-SKILL et REQ-CONTRACT.
