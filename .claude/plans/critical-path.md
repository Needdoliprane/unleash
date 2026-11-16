# Chemin critique et expériences de décision

Le chemin critique est logique ; un calendrier serait trompeur sans équipe et sans mesures M0. Les travaux graphiques, recherche documentaire et revue de contrat peuvent commencer tôt, mais leur intégration dépend du noyau commun. La V1 comprend les 19 features, sans réduction de F001–F018 ; F019 et les compléments locaux/guidés entrent dans les jalons communs, pas dans une annexe après livraison.

## Chaîne technique dominante

Identités/autorité → transaction état+journal → admission mixte et journal d'actions → workflows/contrats/artefacts → adaptateurs métier et sources → recette de panne et de reprise à travers les interfaces. La collaboration Git est une branche essentielle du développement ; les procédures CSV/navigateur empruntent le même socle sans dépôt ou agent obligatoire.

Le point le plus coûteux à corriger tard est l’identité d’une **action** distincte de l’invocation ou du worker mécanique et de leur tentative. Sans elle, idempotence, facturation inconnue, preuve de test, effet navigateur et reprise Git deviennent des cas particuliers incompatibles. La seconde décision critique est la sémantique de fin du run et de ses versions/bindings, fixée par Q-03 : le Run est la frontière de migration ([ADR-04](../architecture/adr/ADR-04-workflow-semantics.md)) ; elle détermine migrations, contexte et reprise. La troisième est l'origine d'exécution : forbidden doit rester transitive et local_only survivre aux fallbacks, sans contournement par un outil ou un job de conception non mandaté.

## Chemin F019 et compléments R3–R5

| Jalon | Dépendance à lever | Preuve décisive à produire |
|---|---|---|
| M0 | BrowserPort, pilote de référence WebDriver (`fantoccini`, Chrome for Testing, Q-18), profil médié/script et isolation | Qualification sur fixture : cible ambiguë, CSS valide/résultat faux, contrôle d'effet même domaine, canal privé et reprise. Aucun moteur déclaré qualifié par sa seule documentation. |
| M2 | Admission agent_invocation/replay_worker/provider_probe et fournisseurs locaux | Même plafond 5/50, probes fournisseur comprises, ressources navigateur/locales distinctes ; trois tuples LM Studio/Ollama/llama.cpp de la matrice Q-19 qualifiés, OOM/local_only et annulation incertaine. |
| M3 | Profils v1.1/v1.2 (for_each, after_batch), artefact/données immuables et contrats | Replay sans agent/modèle/Git ; supervision publiée par task/choice aux checkpoints, aucune task cachée sous forbidden, version réparée sans hot swap. |
| M5 | Parcours guidé et opérations GUI/TUI complètes | Départ sans agents configurés, template et rôles préparés selon permissions, aperçu clair, modes/coûts visibles, paramètres et erreurs modifiables dans le terminal. |
| M7 | Qualification transversale | Scénarios W/X/Y/Z de la recette commune, preuves F019 et locales, effets inconnus non répétés, restauration et parité ; limites non masquées par taux moyen. |

Ce chemin concrétise H-17 à H-21 et INV-058 à INV-065. Les hypothèses de qualification ne sont pas des résultats exécutés ; voir [plan V1](v1-implementation-plan.md), [tests F019](../testing/automation-replay-tests.md) et [tests locaux](../testing/local-model-tests.md).

## Chemins parallèles qui peuvent devenir critiques

- **GUI et TUI** : la qualification de la pile Tauri 2 + React/TypeScript acceptée ([ADR-03](../architecture/adr/ADR-03-rust-gui.md)) et celle de Ratatui/Crossterm ([ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md)) doivent inclure saisie IME, lecteurs d’écran, arbre/table volumineux, sélection de logs, thèmes et terminal. Découvrir trop tard qu’un framework exige une réécriture d’un contrôle essentiel retarde toute la supervision ; un tel défaut éliminatoire déclenche le réexamen prévu par ADR-03.
- **AI-Guard** : fournir le contrat réel tôt. Le côté Unleash est fixé par [ADR-10](../architecture/adr/ADR-10-ai-guard-boundary.md) (Q-04) ; tant que le contrat manque, la V1 progresse, mais sa recette d’intégration EXT-GUARD reste à produire.
- **Connecteurs** : les vérifications Google/Microsoft du client officiel `ovpt_official` peuvent durer plus longtemps que l’adaptateur Rust ; la voie `bring_your_own` permet la recette fonctionnelle en attendant (Q-08). Le parcours Notion V1 est le token d’intégration local (Q-09) ; aucune distribution OAuth publique Notion n’est promise.
- **Isolation macOS** : un outil shell arbitraire sous le même utilisateur ne devient pas confiné par une ACL applicative. Le mode `standard`, défaut de Q-05, affiche cette limite ; le mode `strict` doit démontrer la frontière sur l’OS ou refuser ([frontières de sécurité](../architecture/security-boundaries.md)).
- **Portabilité analyse de code** : une recherche lexicale ne peut pas être vendue comme résolution de références équivalente dans tous les langages. Matrice de capacités et UX de confiance sont nécessaires.
- **Navigateur et scripts** : l'absence de contrôle d'action dans un script libre ne se répare pas par une allowlist de domaine. Qualification des effets, credentials, descendants et protocole de reprise précède toute promesse de contrôle autonome.
- **Localité d'inférence et mémoire** : le même alias/endpoint peut cacher une configuration différente. Versionner la qualification et bloquer local_only si le placement est inconnu ; aucune redirection cloud pour résoudre une saturation.
- **Parcours guidé** : un template inadéquat ou une création répétée de rôles peut produire effets/coûts inattendus. Le backend garde idempotence, aperçu et autorité ; simplicité de présentation ne vaut ni suppression de fonction TUI ni assouplissement de mandat.

## Critères qui départagent les principales alternatives

Depuis R7, l’option citée en premier dans chaque ligne est acceptée, avec l’ADR ou la question qui la fixe. Les expériences la qualifient et en étalonnent les valeurs par défaut ; une alternative n’est rouverte que sur défaut mesuré, par un ADR de supersession.

| Choix | Expérience | Critère de décision |
| --- | --- | --- |
| SQLite à writer unique (accepté, ADR-01) vs infrastructure plus lourde | Charge mixte 5/50, écritures groupées, gros logs hors DB, lectures GUI/TUI longues | Respect latences et reprise avec writer local ; distribué seulement si besoin mesuré non résolu |
| Réserve souple (défaut accepté, Q-15) vs réserve dure (option) | 5 N puis 3 N + 2 workers mécaniques, urgence N+1 autorisée, deux projets, quotas saturés | Temps d’attente borné sous hypothèses déclarées ; débit comparé ; aucun dépassement ou réveil modèle interdit ; la mesure étalonne les valeurs par défaut sans changer le mécanisme |
| Métadonnées centrales et index spécialisés dérivés (acceptés, ADR-08) vs index central unique | Dirty snapshots, rebase, 10 worktrees, recherche helper | Provenance correcte, invalidation et coût de reconstruction ; pas uniquement pertinence moyenne |
| Pile GUI acceptée (ADR-03) vs déclenchement du réexamen, avec TUI équivalente | Même écran réel, parcours guidé et données | Défaut éliminatoire IME, lecteur d’écran, charge ou fermeture des commandes non corrigeable par remplacement de composant ; revue visuelle, clavier/a11y, latence, licences et parité des fonctions |
| Contrat fermé (accepté, ADR-06/07) vs skill libre | Skill plausible mais déclarations fausses et permissions excessives | Rejets mécaniques explicables ; impossibilité de publication auto avec élévation implicite |
| Process-host séparé (accepté, ADR-05) pour PTY et services ; enfant direct du daemon pour les workers d’invocation | Tuer daemon pendant PTY/service | Logs/PTY encore récupérables quand promis, ressources maîtrisées |
| Qualification du pilote WebDriver de référence (Q-18) | Même procédure et mêmes contre-exemples sémantiques | Capacité, médiation, versions, annulation et reprise réelles ; aucune équivalence de locators supposée |
| Replay et supervision par blocs (modes V1 acceptés, ADR-16) : fréquence de checkpoints | Mêmes données, défauts et oracles, fréquence de checkpoints variable | Erreurs détectées, coûts mesurés et appels bornés ; logs corroborés, aucune obligation d'un juge universel |
| Profil local admissible (accepté, ADR-17) vs autre cible explicitement autorisée | Tuple réel, cold/warm, OOM, alias modifié et proxy loopback | Capacités et placement établis ; local_only jamais relâché implicitement, ressources incertaines non libérées sans preuve |

Les valeurs mesurées, le matériel, les versions et le corpus accompagnent la qualification de l’ADR accepté ; une mesure qui le réfute appelle un ADR de supersession. Une préférence de framework ou une promesse de fournisseur n’est pas une preuve de passage.

## Ce qui ne doit pas être sur-conçu

Pas de bus distribué pour relier les modules d’un daemon. Pas de microservice d'automatisation, de deuxième DB ou de scheduler navigateur. Pas de moteur de plugins arbitraires avant besoin d’extension qualifié. Pas de moteur d'inférence réécrit en Rust. Pas de graph database obligatoire pour la connaissance. Pas de sauvegarde de RAM des forks ou navigateurs. Pas de DSL général Turing-complet pour reproduire un langage existant. Pas de locks de fichiers globaux pour masquer des périmètres de travail mal attribués. Les alternatives restent documentées si une mesure ultérieure les rend nécessaires.

