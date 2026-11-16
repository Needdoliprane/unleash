# F016 — Conception technique

## Architecture proposée

`ActivityJournal` reçoit les événements métier du writer ; `EvidenceCatalog` référence les artefacts ; `InvocationManifestBuilder` conserve les entrées effectives ; `ClaimReconciler` rapproche affirmations et observations sans effacer leurs différences ; `ReportProjector` construit des vues déterministes ; `EventSubscriptionService` sert les curseurs ; `TelemetryAdapter` exporte éventuellement les signaux techniques. Tous ces composants sont Rust et parlent les mêmes contrats de domaine.

Le journal possède une séquence locale monotone distincte de l'heure murale. Une correction d'horloge ne réordonne pas des événements engagés. Les liens causaux `causation` {kind, id} (receipt_id pour kind=command) et `correlation_id` du [schéma d'événement](../../schemas/event.schema.json) relient commandes, invocations et actions ; ils ne supposent pas que deux événements proches dans le temps sont causalement liés. Les projections courantes sont reconstruisibles depuis l'état transactionnel autoritaire et les événements encore retenus nécessaires à leur type ; leur watermark porte stream_id et sequence. Cela ne permet pas de reconstruire un état historique arbitraire sans snapshots et entrées suffisants.

## Écriture et artefacts

État métier, événement et outbox sont engagés ensemble pour une transition. Le contenu volumineux est écrit hors transaction dans une zone temporaire contrôlée, scellé par digest, puis référencé après validation. Un artefact incomplet reste `pending` ou `unavailable`; aucun enregistrement ne prétend que ses octets existent avant constat. Le stockage doit définir sa stratégie de flush et de récupération, sans confondre atomicité de métadonnées et durabilité de fichier.

Les digests servent à détecter des changements de contenu et à associer les preuves à leurs sources. Ils ne constituent pas une signature contre un propriétaire local capable de modifier base et fichiers. Une authentification cryptographique forte demanderait un modèle de clés distinct ; elle n'est pas implicitement promise par « audit trail ».

`TestObservation` conserve code de sortie ou signal, identité durable de processus, commande structurée, cwd, toolchain, snapshot, paramètres autorisés, sorties et indicateurs de troncature. Une sortie tronquée peut prouver un code reçu mais pas l'intégralité des assertions textuelles absentes. Une interruption de capture laisse `output_incomplete`; une fin non reçue reste `termination_unknown`.

## Claims, décisions et discovery

Une claim suit [contract-claim.v1](../../schemas/contract-claim.schema.json) : auteur résolu par le canal, sujet exact, statement, proposition observable fermée ou qualitative et observation_ids invoquées. Le réconciliateur applique des règles déterministes pour les correspondances simples : test invoqué existant, snapshot égal, commande terminée, reçu valide. Les points sémantiques restent des judgments de leader attribués. Le système n'insère pas un second juge LLM pour chaque résultat. Les contradictions sont des objets résolus par une nouvelle décision, pas supprimés par mise à jour de la phrase initiale.

Les reçus discovery sont conservés par F009 et indexés dans les rapports F016. Le rapprochement avec les créations vérifie catégorie, scope, génération et antériorité. Le contrat peut exiger cette preuve à la clôture ; les créations non identifiables par l'outil sont un trou de couverture affiché. Une revalidation après rebase ou crash devient une nouvelle preuve liée, sans antidater la recherche initiale.

## Contextes et rapports

Le ContextManifest de chaque appel est scellé au dernier point où le contenu préparé est connu ; sa transmission et sa réception sont observées séparément par le ModelCallRecord. Il référence les messages, règles, versions de skills et rôles, documents pin, résultats de recherche, compactions et configuration réelle. Les écarts entre contexte demandé et contexte préparé sont visibles, notamment troncature ou suppression par ACL. Les secrets sont représentés par références et motifs de masquage, pas par valeurs récupérables depuis l'UI.

CreateReportSnapshot capture l'état courant et un curseur {stream_id, sequence} dans une même vue transactionnelle, avec version de schéma et politique d'accès. Les entrées nécessaires au rapport sont scellées ou référencées durablement sous rétention, puis le rendu utilise ce snapshot. Aucune séquence arbitraire passée n'est acceptée comme si le journal était un event store complet. GetReportSnapshot retourne un rapport historique déjà conservé ; si ses entrées ont été purgées, missing_sources indique les limites et HistoryUnavailable bloque toute reconstruction qui les exige. Les nouvelles autorisations sont recontrôlées lors de lecture : un rapport historique ne contourne pas une révocation. La reproductibilité s'entend pour mêmes événements et même politique ; si l'accès change, la vue porte cette différence au lieu de révéler un ancien contenu privé.

## Flux, pression et reprise

Les abonnements utilisent un curseur de séquence et une fenêtre bornée. Un client lent reçoit un état de retard puis la réponse ResyncRequired (Error commun) si la fenêtre est perdue ; aucun événement de journal n'est créé pour ce client. La déconnexion d'un client n'arrête pas la mission. Les événements métier requis ne sont pas abandonnés pour préserver une animation GUI ; la télémétrie de diagnostic peut être échantillonnée ou réduite avec compteur de perte.

Si le journal n'est plus inscriptible, les nouveaux effets nécessitant une intention durable sont refusés. Les effets déjà dispatchés restent suivis et leurs résultats peuvent exiger une zone de secours qualifiée ; à défaut leur statut demeure incertain jusqu'à réconciliation. Aucune mémoire volatile ne devient source de vérité à elle seule. La reprise restaure les projections courantes depuis l'état autoritaire et leurs sources encore disponibles, puis rapproche Git, processus et fournisseurs selon leurs contrats. Les rapports historiques scellés sont relus, non recréés artificiellement depuis une projection actuelle.

## Rétention et sécurité

La rétention s'applique par classe : journaux métier, sorties d'outils, captures dirty, manifestes, rapports et exports. Les durées par classe sont celles de la politique `unleash.retention.default.v1` du [modèle de stockage](../../architecture/storage-model.md) (Q-12) : journal métier sans échéance automatique, payloads modèle 90 jours, logs et artefacts 30 jours, diagnostics 14 jours. Aucun objet requis par un run non terminal, un effet inconnu, une preuve active ou un pin n'est purgé automatiquement ; les autres références nécessaires à un contrat actif sont protégées ou leur suppression demande une décision explicite. La suppression retire effectivement les octets accessibles dans le périmètre contrôlé et laisse un tombstone minimal autorisé. Elle ne prétend pas effacer une copie déjà exportée chez un tiers.

Le masquage agit sur champs structurés avant stockage et sur flux libres avec taille maximale. Les secrets connus et leurs formes détectables sont filtrés, mais des transformations arbitraires restent une limite. Les exports réseau de télémétrie sont facultatifs, sans texte de prompt ni payload documentaire par défaut. Les métriques ont une cardinalité bornée et séparent coût estimé et usage communiqué par fournisseur.

## Discovery et temporalité R6

Les rapports exposent séparément DiscoveryReceipt immuable, DiscoveryUseObservation de la création, éventuelle completion_revalidation et fraîcheur courante. Les sources et origines typées suivent [F009](../F009-knowledge-engine/interfaces.md), y compris humain/replay et fichiers/documents sans Git. Une modification ultérieure pertinente réévalue l’applicabilité, sans effacer la preuve d’une recherche antérieure réellement observée. Un simple commit de son propre résultat identique ne transforme pas cette preuve en violation automatique. Les projections restent sous ACL et indiquent absence/inconnu/périmé.
