# Politique de sécurité

## Versions prises en charge

Avant la première version publique, seule la branche `main` reçoit des correctifs. Après publication, la dernière version mineure publiée et `main` sont prises en charge.

## Signaler une vulnérabilité

Ne publiez pas de ticket public. Utilisez le signalement privé de vulnérabilité de la forge qui héberge le dépôt (GitHub Security Advisories), ou l'adresse de sécurité publiée par OVPT avec la première version. Indiquez la version ou le commit, l'environnement, les étapes de reproduction et l'impact observé.

Engagements : accusé de réception sous 5 jours ouvrés, évaluation initiale sous 10 jours ouvrés, divulgation coordonnée sous 90 jours au plus, crédit au signalant s'il le souhaite.

## Périmètre

Le modèle de menace de référence est dans [`.claude/audits/security-threat-model.md`](.claude/audits/security-threat-model.md) et les frontières dans [`.claude/architecture/security-boundaries.md`](.claude/architecture/security-boundaries.md). Sont notamment dans le périmètre : élévation d'autorité entre agents, contournement de l'action broker ou d'AI-Guard, fuite de secrets, exécution non autorisée, confusion d'identité de processus et altération du journal. Le compte utilisateur OS compromis et les privilèges root sont hors périmètre du mode standard.
