# Contribuer à OVPT Unleash

Merci de votre intérêt. Le dépôt contient le dossier de conception de référence (`.claude/`) et le code Rust du produit.

## Avant de coder

1. Lire [`AGENTS.md`](AGENTS.md) : ordre de lecture, autorités documentaires, invariants et règles.
2. Lire la feature concernée dans `.claude/features/` (exigences, conception, interfaces, acceptation, tests).
3. Appliquer [les règles Rust](.claude/rules/rust.md), [les règles de test](.claude/rules/testing.md) et, si un contrat change, [les règles du dossier](.claude/rules/design-dossier.md).

## Proposer un changement

- Une branche par changement, commits au format [Conventional Commits](https://www.conventionalcommits.org/fr/) citant les exigences (`REQ-…`).
- Les gates de la [définition de terminé](.claude/dev/definition-of-done.md) doivent passer : fmt, clippy `-D warnings`, tests, doc, cargo-deny, validation du dossier.
- Un changement de contrat met à jour le dossier `.claude/` dans le même changement et passe `python3 tools/design-package/validate.py .`.
- Les contributions sont acceptées sous la licence Apache-2.0 du projet (section 5 de la licence). Signez vos commits avec `Signed-off-by` (Developer Certificate of Origin).

## Marques

Le code est sous Apache-2.0 ; les noms « OVPT » et « Unleash » et leurs logos ne sont pas concédés par cette licence. Un fork redistribué doit porter un autre nom et ne pas laisser croire à une distribution officielle. Une vérification d'antériorité du nom est prévue avant la première publication publique.
