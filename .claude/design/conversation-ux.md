# Conversations et interventions humaines

Une conversation appartient à un agent logique et peut être filtrée par mission. Chaque message conserve auteur, destinataire, mission, date, références et événements causaux. Les messages du modèle et les résultats d’outils se distinguent visuellement.

## Envoi à un N+1

Le compositeur montre le destinataire exact et la mission. Après commit, le message affiche « enregistré » et « visible dans l’activité N+2 ». Une marque distincte indique une invocation du manager qui l’a effectivement consommé. Aucun accusé « lu » n’est inventé depuis la seule livraison d’un événement.

L’envoi lors d’une déconnexion devient brouillon local. Si la connexion se coupe après acceptation, le client retrouve le reçu par command_id. Une nouvelle tentative d’UI ne produit pas un second message.

## Effets et révisions

Une demande humaine peut devenir proposition de révision de mission. La conversation montre le diff, l’autorité et l’effet attendu sur les étapes non engagées. Le message originel reste immuable. Une correction est un nouveau message, pas une réécriture de l’historique.

L’utilisateur peut citer un snapshot, un rapport ou un document. Le lien conserve sa provenance ; le texte du chat ne suffit pas à rendre une preuve ancienne applicable au nouveau code.

## Confort de lecture

Flux regroupés par décision, réponses longues repliables mais intégralement consultables, navigation vers pièces et contexte de l’invocation. Le texte n’est pas animé à chaque token de tous les agents. Les avis N+1 et N+2 sont présentés comme jugements attribués, distincts de résultats mécaniques.

Test : scénario J et AC-F014-004 ; le N+2 peut rester dormant malgré un message informatif visible.

