---
name: retex
description: Appliquer un retour d’expérience quand Louis corrige le travail, qu’une preuve utilisée est invalidée ou qu’un blocage revient. Corriger la cause, vérifier le premier effet et conserver la leçon dans le dépôt.
---

# Retex

1. **Constater.** Retrouver la fiche et le journal du goal. Identifier le résultat attendu, le dernier effet observé et la preuve qui manque. Ouvrir la [base de retours d’expérience](../../../.claude/dev/retours-experience.md) et chercher un cas déjà traité.
2. **Expliquer.** Suivre la chaîne de livraison jusqu’au premier raccord manquant. Identifier quelle décision, quel contrat ou quelle dépendance explique l’écart. Rattacher chaque cause retenue à une preuve. Pour un contrôle externe, vérifier le dernier résultat ou l’état de l’exécution existante, puis nommer la condition de reprise et son responsable. Poursuivre les travaux indépendants.
3. **Corriger.** Appliquer la plus petite correction qui agit sur la cause dans la règle, l’outil ou le parcours existant. Reprendre la tentative concernée lorsque son entrée a changé ou qu’une preuve nouvelle justifie la reprise. Vérifier le premier effet sur un cas réel. Pour un défaut de code, reproduire le défaut et vérifier la correction par le point d’entrée concerné. Si la vérification reste impossible, consigner le contrôle restant et sa condition de reprise.
4. **Capitaliser.** Quand la correction est réutilisable, compléter l’entrée existante ou ajouter une entrée courte dans la base. Indiquer le déclencheur, la cause, le changement appliqué et le contrôle de son effet. Vérifier dans le candidat que le lien ouvre le résumé des preuves dans le journal. Mettre à jour la fiche si le prochain résultat ou son responsable a changé. À [G7](../../dev/goal-protocol.md#7-clôture-et-rapport), remettre les mêmes entrées à `retro`, avec leur effet vérifié ou la preuve encore attendue.
