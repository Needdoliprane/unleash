# Red team architecturale — scénarios qui peuvent casser Unleash

Cette première analyse attaque les propositions avant leur gel. Elle est rédigée par l’agent principal à partir des audits séparés. Une seconde revue avec un sous-agent distinct, après rédaction, est consignée dans [final-independent-review](final-independent-review.md). Ne pas confondre ces deux degrés d’indépendance.

> **Note R7 (16 novembre 2026) :** addendum R2 manquant. Ce rapport conserve les constats de la revue initiale. Ses mentions d’une interdiction absolue de présentation hors Rust, de Q-01/Q-17 ouvertes ou d’egui comme candidat principal (notamment la réponse 18) décrivent cet état historique et sont supersédées par la [clarification utilisateur](../brief/clarifications.md) et la [politique actuelle](../brief/implementation-policy.md). La GUI est Tauri 2 avec renderer React/TypeScript strict, acceptée dans [ADR-03](../architecture/adr/ADR-03-rust-gui.md) (Q-02 résolue R7) ; voir aussi [ADR-15](../architecture/adr/ADR-15-interface-parity-and-pragmatic-rust.md) et la [revue ciblée R2](frontend-terminal-revision-review.md).

## Attaques concrètes prioritaires

**ADV-01 — Double effet après crash, critique.** Le broker envoie un merge/publish, l’effet réussit, le daemon tombe avant receipt. Un retry naïf crée une deuxième publication ou réapplique une opération. Solution : intent durable, OIDs/clé d’opération, reconnaissance de résultat, OutcomeUnknown si preuve absente. Alternative : intervention humaine sur chaque effet ambigu. La seconde est moins autonome mais honnête ; aucune transaction SQLite ne couvre un service extérieur.

**ADV-02 — Deux versions du même helper, élevé.** A écrit un helper dirty, l’index le voit ; B le réutilise logiquement mais il n’est pas dans sa branche. A rebase et renomme. Solution : provenance par snapshot, disponibilité distincte et dépendance explicite, reçu de recherche revalidé. Alternative : ne proposer que commits intégrables avec dirty visible à titre indicatif. Test : rebase entre discovery et clôture, aucun statut « disponible ici » sans présence.

**ADV-03 — Managers bloqués par leurs enfants, critique.** Cinq N attendent une revue N+1 en gardant leurs slots. Solution : waiting_durable libère après checkpoint ; admission prioritaire et fairness. Alternative : réserve dure d’un slot contrôle. Test : vingt leaders, cinq workers initiaux, événements managers et fournisseur lent ; aucune allocation supérieure à cinq.

**ADV-04 — Skill auto-publié qui s’accorde des droits, critique.** Un rédacteur met « shell global autorisé » et le N+2 publie. Solution : prompt/contrat ne peuvent augmenter l’autorité ; publication vérifie capacités. Alternative : revue humaine pour extensions de permissions uniquement. Test : import et création automatique ne permettent pas l’action hors mandat.

**ADV-05 — Le code est observé sur un état qui n’a jamais existé, élevé.** L’index parcourt deux fichiers pendant des écritures. Un double scan peut manquer un changement ABA. Solution : objets archivés immuables et niveau de cohérence de capture séparé. Alternative : analyses fiables seulement sur tree Git ou workspace quiescent prouvé. La connaissance dirty reste offerte, avec limite visible.

**ADV-06 — Stale worker et PID réutilisé, critique.** Après expiration lease, un autre processus prend le même PID et l’ancien worker envoie encore une action. Solution : identité génération/boot/démarrage, handshake, fence contrôlé au broker et arrêt/reconciliation. Alternative : fail-stop de la mission si identité non vérifiable. Aucun kill sur PID seul.

**ADV-07 — Fausse isolation, critique.** Un build de projet écrit dans le répertoire de données Unleash ou vole un secret accessible sous le même UID. Solution : secrets hors environnement, séparation des canaux et mode strict avec backend d’isolation qualifié. Alternative : mode standard explicitement limité et refus de promesse hostile. Un broker d’argv sans confinement ne garantit pas toutes les actions descendantes.

**ADV-08 — Workflow modifié à la prochaine requête API, critique.** v2 change les gardes pendant v1 et un résultat antérieur est relu avec autre contrat. Solution : AST run immuable, adoption au nouveau run après frontière explicite ; bindings engagés fixes. Alternative : migration contrôlée à checkpoint avec mapping, plus complexe et non choisie par défaut.

**ADV-09 — GUI soignée mais succès mensonger, élevé.** La barre passe à 100 % parce que N dit « terminé », alors qu’une branche de join travaille encore. Solution : afficher fin mécanique, revue et done utilisateur séparément, plus état quiescent. Alternative : limiter l’indicateur à étapes vérifiées, sans pourcentage global inventé.

**ADV-10 — Bibliothèque de compétences proliférante, élevé.** Chaque mission crée son rôle et son skill à nom voisin, tous publiés. Solution : recherche enregistrée, namespaces, provenance, consolidation par N+2 et dépréciation ; création justifiée permise. Alternative : curateur humain périodique. Aucune distance d’embedding ne prouve une équivalence.

## Réponses aux vingt-cinq questions imposées

| Q | Analyse / solution recommandée | Alternative crédible et test discriminant |
| --- | --- | --- |
| 1. N+2 sans microgestion | Rapports contractuels, déclencheurs ciblés, pas logs par défaut ; budget de décisions organisationnelles | Règles d’escalade plus strictes ; mesurer invocations manager/mission et arbitrages utiles |
| 2. Autonomie N+1 sans moteur caché | Commandes typées sur revision, AST propriétaire du harness, choix de tâches dans slots autorisés | Catalogue de plans plus fermé ; rejouer décisions doublées et vérifier une transition |
| 3. Compiler expressif et borné | AST fermé sequence/parallel/choice/repeat, gardes pures typées, bornes et budgets | SCXML restreint ; comparer workflows réels et erreurs statiques, aucun eval libre |
| 4. Done libre et cohérent | Prédicat utilisateur sur faits valides, séparé de sécurité et clôture des effets | Module done nommé versionné ; Dev→Merge passe sans audit ajouté |
| 5. Contrats vérifiables | Schéma, fichiers/diffs, commande et exit, artefacts/hash, discovery, autorisations | Contrôleur Rust spécifique ; tests mutés et claims mensongers doivent échouer |
| 6. Helpers similaires | Recherche lexicale/syntaxique/sémantique sous scope, candidats avec confiance et provenance | Recherche exact-symbol seulement ; corpus doublons et faux positifs métier |
| 7. Base de connaissance | Métadonnées centrales SQLite, objets et index spécialisés reconstruisibles | Index unique ; comparer rebuild, stale et filtrage ACL |
| 8. SQLite suffit-il | Writer unique, transactions courtes, WAL local et backpressure ; la borne 50 n’est pas 50 writers | DB serveur si mesures réfutent capacité ; charge mixte avec critères proposés |
| 9. Robustesse PID+session | Ajouter identité de lancement, fencing, journal d’actions, process-host et reconciliation | Worker isolé sans reprise PTY ; kill/reuse PID doit être sans dommage |
| 10. Reprise non sûre | Actions non idempotentes sans preuve externe, commandes composites et requêtes ambiguës restent unknown | Réapprobation humaine ; injecter coupure après effet avant receipt |
| 11. Doubles merges | Target oldOID/newOID, CAS, intent durable et réconciliation avant retry | Forge avec checks requis si choisie ; merge effectif une fois sous redelivery |
| 12. Conflit non textuel | Signaux API/tests/contrats et revue selon workflow ; agent intégration | Sérialisation des intégrations ; exemples compilation réussie mais comportement cassé |
| 13. Intentions sans blocage | WorkIntent indicatif, périmètre/version/échéance, conflits signalés | Lease ciblé seulement ressource exclusive ; mesurer attente inutile |
| 14. N+2 → N direct | Autorisé pour mandat spécialisé avec owner, scope et rapport ; même moteur | Passer systématiquement par N+1 ; comparer coût et clarté sur audit isolé |
| 15. Coûts dynamiques | Admission + réservations atomiques, budgets hérités, plafonds d’essais et wakeup coalescé | Quotas par mission fixes ; charge cinquante demandes sur budget commun |
| 16. Publication autonome skills | Contrat mécanique et tests publication, droits bornés, évals de confiance, N+2 décide | Revue humaine sur classes à risque ; publication malveillante doit rester sans élévation |
| 17. Sessions/contextes | Agent logique durable, bundles par invocation et branches de mandat | Session par mission plus stricte ; tester contamination croisée et compaction |
| 18. GUI Rust riche et belle | Choix sur spike représentatif egui/Iced/GPUI/Slint selon contraintes | Rust-WASM/Tauri après arbitrage ; mesurer rendu, a11y, terminal et charge |
| 19. Visibilité sans CoT privé | Manifests accessible/sélectionné/envoyé, sources, exclusions, blocs opaques identifiés | Conserver seulement hashes sous politique stricte ; comparer payload mock et affichage |
| 20. AI-Guard découplé | Port versionné et capability negotiation ; mapping après lecture contrat réel | Broker obligatoire externe ; tests panne/révocation et absence de contournement |
| 21. Réveils utiles | Event classes, mailbox coalescée, triggers de décision et seuils | Périodicité de consolidation ; mesurer wakeups perdus/dupliqués sans scruter en boucle |
| 22. Tests charge/panne | Horloge/graine contrôlées, fault points, invariants et historiques comparables | VM de chaos pour OS réel ; compléter doubles par crash processus/machine |
| 23. Sur-ingénierie | Écarter Redis, microservices, langage de scripts de guard, event sourcing intégral | Introduire seulement sur métrique/contrat qui l’exige ; coût d’exploitation mesuré |
| 24. Évidences mal spécifiées | Versions, freshness, pause, fork, acteur, snapshot, coûts inconnus et disponibilité helpers explicités | Réduire les mots ambigus ; test d’une équipe indépendante sur les contrats |
| 25. Risques long terme | DSL devenu langage général, couplage modèle/GUI, promises isolation/recovery et skills sans provenance | Ports petits et versions, ADR revus ; suivre migration et compatibilité sur deux versions |

## Désaccords à préserver

Le pinning complet de tous rôles dès le début du run paraissait plus simple, mais limitait l’adaptation technique N+1. L’arbitrage proposé autorise uniquement de nouveaux bindings dans des slots tardifs prévus et encore vierges ; les bindings déjà engagés restent figés.

La réserve dure d’un slot manager donne une disponibilité plus simple au prix de débit worker. La réserve souple utilise mieux les cinq slots mais sa latence dépend de checkpoints/deadlines. Les deux modes doivent être testés ; ne pas promettre une préemption physique impossible.

La publication CAS sur une branche cible non checkout protège le dépôt mais ajoute une friction utilisateur. L’adoption du repository doit traiter ce point en amont et proposer une stratégie explicite, jamais déplacer silencieusement la branche sur laquelle travaille l’humain.

