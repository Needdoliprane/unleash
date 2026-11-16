# ADR-10 — Autorisation, isolation et AI-Guard

**Statut : accepté le 16 novembre 2026 (R7, arbitrage délégué de Louis).** Amendement R7 : Q-04 et Q-05 tranchées côté Unleash ; seule la preuve externe EXT-GUARD reste ouverte ; voir la section « Amendement R7 ». Proposé le même jour par le porteur du produit et le responsable technique du périmètre. Accepté n’est pas confirmé : cet ADR ne remplace aucun invariant et se révise par un ADR de supersession.

## Contexte

La V1 doit intégrer AI-Guard, mais ses interfaces réelles ne sont pas disponibles. Un shell même UID dépasse un simple scope de fichiers.

Exigences : REQ-SEC. Risques/hypothèses/questions : R-11 R-12 R-13 ; H-11 H-12 ; Q-04 Q-05.

## Options et compromis

Option recommandée : **Action Broker interne + GuardPort versionné, modes explicites**.

Alternatives considérées : Contrôle des seules actions sensibles ; AI-Guard broker obligatoire.

Le port permet tester sans inventer endpoint ; contrôle sensible risque omission ; broker obligatoire apporte dépendance et doit réellement confiner.

## Décision

Sécurité minimale dans Unleash ; mode required fail-closed ; capacités d’isolation vérifiées. Décisions GuardPort pour tous les effets initiés hors `local_read` ; AI-Guard exécuteur seulement après négociation et qualification (amendement R7).

## Amendement R7

1. ActionBroker interne unique et obligatoire dès la V1 ; la sécurité minimale est assurée par Unleash dans tous les modes.
2. Quand AI-Guard est configuré, GuardPort versionné évalue toute action du broker hors classe `local_read` ; la V1 n’offre pas de frontière « actions sensibles seulement ». Sans AI-Guard configuré, le broker Unleash agit seul ; dès qu’un endpoint est configuré, le mode par défaut est `guard_required`, fail-closed. `guard_optional` n’admet qu’une politique de repli préautorisée, versionnée et visible. Valeurs exactes des modes : [AI-Guard](../../specs/ai-guard.md).
3. Les obligations d’un Allow sont limitées au profil `none_accepted` jusqu’à qualification du mapping réel : un Allow assorti d’une contrainte non supportée est refusé, sans dispatch ni résolution de secret.
4. AI-Guard n’est exécuteur (`ExecuteAuthorizedAction`) que si la capacité est négociée et qualifiée ; l’exécuteur par défaut reste Unleash.
5. Isolation (Q-05, [frontières de sécurité](../security-boundaries.md)) : mode `standard` par défaut, qui couvre contenus, modèles, skills importés, autres UID et réseau hostiles ; mode `strict`, qui ajoute le code hostile lancé sous le même UID et refuse l’exécution (`IsolationUnavailable`) si le confinement requis n’est pas prouvé sur l’OS. Compte OS compromis et root restent hors périmètre.
6. Endpoints, payloads, authentification et sémantique des contraintes du vrai AI-Guard ne sont pas décidés faute de pièces : ils relèvent du gate EXT-GUARD et de la recette F017, sans retrait de la V1. Réexaminer si la qualification réfute `none_accepted` ou si le protocole AI-Guard impose une autre frontière.

## Conséquences, complexité et impact V1

Le choix côté Unleash est fermé (Q-04 et Q-05 résolues) ; seule la preuve EXT-GUARD reste ouverte. Mock ne suffit pas à recette. Same-UID non confiné et admin machine hors promesse forte. Complexité nécessaire, portée déclarée.

Toute évolution doit préserver le périmètre fonctionnel V1. Une alternative ne peut être retenue pour supprimer une capacité explicitement demandée. La mise en œuvre est incrémentale ; la recette complète doit produire les preuves avant de revendiquer cette garantie.

## Vérification et critère de réexamen

Canaris, revocation, panne GuardPort, capability mismatch et recette réelle : T-SEC-03, T-SEC-04, T-SEC-10, T-F017-012 et AC-E2E-V.

Réexaminer si la qualification réfute une hypothèse, si un contrat externe manque, ou si une contrainte utilisateur change. Consigner résultat, nouvelle décision et conséquences sur les features ; une mise à jour de bibliothèque n’est pas une autorisation de modifier une exécution engagée.

Détail canonique : [spécification](../../specs/ai-guard.md). Sources techniques et limites : [registre](../../research/sources.md). Les tests indiqués sont attendus sur l’implémentation, non exécutés pendant la rédaction.

