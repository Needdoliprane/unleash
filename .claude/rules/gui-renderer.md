---
paths:
  - "apps/gui-renderer/**"
---

# Règles du renderer GUI (React + TypeScript)

Décision : [ADR-03](../architecture/adr/ADR-03-rust-gui.md). Le renderer est une couche de présentation : il n'est jamais l'autorité métier.

Appliquer le [design system OVPT d’Unleash](../design/visual-design-system.md) à tout changement d’interface. Inventorier les composants et tokens concernés, conserver routes et thèmes et valider le renderer à 320, 390, 768 et 1440 px. Consigner les résultats de clavier, focus, contrastes et débordements sur les écrans modifiés.

- **Aucune logique métier.** Pas de transition d'état, d'autorisation, de budget ni de validation métier dans le TypeScript ; une prévalidation de forme n'est qu'une aide. Toute action passe par une opération du catalogue (`.claude/catalog/operations.json`) via l'hôte Rust `unleash-gui` et `unleash-client`.
- **Types générés.** Les types du protocole sont générés depuis les DTO Rust (ts-rs) dans `src/generated/` et ne sont jamais édités à la main ; la CI échoue si la régénération produit un diff.
- **TypeScript strict** : `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes` ; pas de `any` implicite ; ESLint et Prettier sans avertissement.
- **Dépendances verrouillées.** Node.js LTS et pnpm servent seulement au build ; versions figées par `pnpm-lock.yaml` ; aucun chargement distant, CDN ni code exécuté hors bundle ; CSP stricte.
- **Contenus non fiables inertes.** HTML, Markdown, SVG, URL et logs venant des agents, documents ou sites sont rendus sans exécution (échappement, assainissement ; `dangerouslySetInnerHTML`, `innerHTML` et `eval` interdits par un lint bloquant ; le Markdown est rendu depuis un AST borné produit côté Rust, [ADR-03](../architecture/adr/ADR-03-rust-gui.md)). Aucun secret dans le DOM, le stockage local ou les diagnostics.
- **Accessibilité et qualité** : WCAG 2.2 AA, navigation clavier complète, focus visible, IME, lecteurs d'écran ; listes longues virtualisées ; budgets de performance d'AC-F014.
- **Parité.** Chaque fonction a son équivalent TUI dans le même groupe `PAR-…` ([parité](../design/gui-tui-parity.md)).
- **Tests** : logique de présentation testée sans rendu, composants testés avec Testing Library, parcours testés contre un serveur de fixtures du protocole.
