# Explorateur du contexte

L’écran distingue trois ensembles : ce que l’agent était autorisé à consulter ; ce que le context builder a sélectionné ; ce que le payload a réellement transmis. Confondre ces ensembles fausserait la traçabilité.

## Sections

Identité logique, session/invocation/tentative ; déclencheur et parent ; modèle et configuration effective ; workflow/run/bindings ; rôle/skills exacts ; mission et scope ; messages retenus ; documents et code par snapshots ; résultats d’outils ; compactions ; exclusions et coûts estimés. Chaque section possède provenance, version/hash, taille et statut de disponibilité.

Une compaction ouvre son résumé, les sources couvertes, les sources omises et la version du processus de synthèse. L’utilisateur peut comparer avant/après si les contenus sont encore conservés. Le résumé n’est pas présenté comme fidèle par construction.

## Confidentialité

Les secrets résolus pour HTTP n’apparaissent pas comme contexte. Les blocs opaques fournisseurs affichent seulement type, fournisseur et nécessité protocolaire. Le produit ne prétend pas révéler le raisonnement interne privé. Une portion supprimée par rétention est signalée, avec manifest intact lorsque permis.

L’accès au contexte d’un projet vérifie ses permissions, y compris fichiers et extraits documentaires. Un lien de preuve n’autorise pas une lecture croisée d’un autre projet.

## Test

Comparer hash et segments du payload capturé par mock avec l’inspecteur ; interrompre le worker puis relire ; révoquer un document puis tenter un nouveau contexte ; supprimer un artefact autorisé et vérifier son marqueur manquant. Critère AC-F014-005 et REQ-MODEL-009.


## Précision de transmission

Le contexte de départ d'une invocation et les contextes de ses appels successifs sont distincts. Chaque appel possède un manifeste immuable et un modèle effectif. Le placement included signifie présent dans le payload préparé ; dispatch_attempted indique remise au transport, et une observation fournisseur peut confirmer la réception. Après rupture réseau, la réception peut rester inconnue. L'inspecteur n'affiche pas 'reçu par le modèle' à partir d'une simple intention d'envoi. Un fallback ajoute un appel visible sans remplacer le manifeste précédent.

Ces ensembles suivent le [manifeste de contexte](../schemas/context-manifest.schema.json) : `selection` (accessible/included/excluded) porte autorisé et sélectionné ; `availability` (retained/redacted/purged/opaque/missing) indique séparément ce qui reste consultable ; la transmission vient du ModelCallRecord (prepared/dispatch_attempted/completed/unknown/not_sent) et n’est jamais déduite d’un segment.
