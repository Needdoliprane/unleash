# F006 — Dépendances

Le routeur de modèles réalise la génération NL avec clés utilisateur et capacités négociées. Il ne remplace pas la validation déterministe. F007 fournit catalogue, rôles, skills et profils contractuels exacts. F008 fournit schémas locaux et validateur de compatibilité. Le service de permissions expose les plafonds applicables. Le stockage transactionnel conserve drafts, rapports et publications.

En usage opérationnel, F005 consomme uniquement des définitions publiées compatibles avec son profil. L’exception explicite est un snapshot immuable qualification_only sous purpose=evaluation, créé par EvaluateAutomationDraft avec digest, cible et mandat exacts selon F019 ; le même snapshot est refusé par CreateRun/StartRun opérationnels. F005 peut partager le noyau de simulation. Ce partage doit porter sur la sémantique, pas sur une interface qui autoriserait le simulateur à émettre des effets réels. Les clients GUI/CLI/TUI utilisent les mêmes diagnostics et JSON Pointers ; aucune normalisation particulière à un écran ne doit changer le digest.

Les bibliothèques JSON Schema et JCS restent à sélectionner côté Rust. Le dossier cite des spécifications primaires, pas une compatibilité déjà vérifiée d'une crate. La passe sémantique reste nécessaire même si la validation de schéma réussit. Un validateur externe ne peut pas déterminer à lui seul qu'un scope ou une version de rôle est autorisé.

Pour paralléliser, livrer d'abord le schéma et corpus de diagnostics, puis parser/validation de structure, analyse de contrôle, publication et UI. Toutes ces étapes font partie de V1. Les changements de limites de profil doivent être traités comme évolution versionnée, avec mise à jour des exemples, pas comme ajustement silencieux d'implémentation.
