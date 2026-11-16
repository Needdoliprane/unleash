# F005 — Dépendances

F006 fournit le DSL publié, rapport de compilation et digest. F007 fournit versions de rôle et skills ; F008 fournit validation de contrat et evidence bundles. Le noyau de persistance offre transactions état/journal/outbox, révisions et idempotence. Le scheduler est seul propriétaire des slots d'invocation. Les services de session et contexte produisent le manifest transmis au worker. Les adaptateurs Git, processus et documents fournissent les observations sans que le reducer accède directement aux ressources.

La frontière de contrat avec chaque adaptateur doit distinguer intention, début observé, succès, échec et résultat inconnu. Un adaptateur indisponible ne doit pas être remplacé par une affirmation du modèle. La reprise dépend des identités durables d'actions et du fencing ; un PID ne suffit pas. Le service de permissions contrôle les droits au démarrage puis à chaque action et peut révoquer un droit sans modifier les bundles épinglés.

Les équipes peuvent implémenter reducer et compilateur en parallèle sur les schémas et fixtures du dossier. Le moteur doit refuser une version de DSL qu'il ne comprend pas. Les migrations de stockage et de reducer demandent des tests de replay sur historiques de référence. La version workflow ne constitue pas à elle seule un identifiant du code moteur ; les deux sont conservés pour diagnostic.

Le choix de bibliothèques Rust, notamment sérialisation et stockage, est consolidé par l'architecture générale. Les références SCXML, ASL et Temporal sont des précédents conceptuels, pas des dépendances obligatoires ou des capacités que cette feature prétend déjà reproduire.
