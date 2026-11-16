# Timeline d’activité

La timeline affiche des événements durables, regroupables par projet, mission, étape, agent, outil ou incident. Les segments de logs restent accessibles dans un panneau distinct ; ils ne gonflent pas artificiellement l’histoire managériale.

## Causalité

Chaque ligne peut ouvrir sa cause, son résultat, l’action concernée, le contexte et la preuve. Les trois catégories observable, déclaré et jugé ont des libellés stables. Un événement tardif ou doublé est conservé selon la politique de journal, mais ne crée pas une fausse deuxième exécution.

Afficher sequence et temps de collecte dans le détail technique ; l’heure murale d’un worker ne réordonne pas le journal. Après restauration, un changement de stream force une resynchronisation visible.

## Flux volumineux

Pagination/virtualisation, suivi activable, regroupement des tokens et commandes répétées. Si le client prend du retard, montrer « rattrapage » et watermark. Le backend conserve l’autorité et le client peut reprendre à un cursor. Aucun buffer illimité dans la GUI.

Les logs externes peuvent contenir séquences de terminal ou secrets. Le rendu texte neutralise contrôles dangereux et applique les politiques ; l’accès au brut est séparé et explicite si conservé.

## Vérification

Scénarios redelivery, déconnexion après mutation, explosion de logs, preuve manquante et incident d’effet incertain. La timeline ne doit ni masquer le blocage ni prétendre que toute ligne de commande prouve une modification du code. Voir F016 et AC-F014-008/011.

