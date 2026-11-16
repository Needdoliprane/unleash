# Spécification — traçabilité, preuves et rapports

**Statut : proposition V1**, 16 novembre 2026. [F016](../features/F016-reporting-observability/README.md) porte les exigences exécutables ; ce document définit le vocabulaire et la chaîne de preuve commune. Les éléments décrits sont des contrats Unleash proposés, pas des APIs d'un SDK existant.

## Trois catégories irréductibles

Une **observation** est un fait capturé par un adaptateur contrôlé : processus lancé, code de sortie, OID présent, octets d'un fichier, réponse reçue, compteur fourni par une API. Elle précise sa source, son périmètre et ses limites. Une **claim** est une affirmation de l'agent ou d'un humain, avec auteur, date et références déclarées. Un **judgment** est une décision de N+1, N+2 ou d'une règle déterministe sur des observations et claims identifiées. Un jugement ne modifie pas les faits cités.

« Les tests passent » reste une claim si aucun test run observé ne correspond. « `cargo test` a terminé avec code 0 » est une observation sur une commande, un environnement et un snapshot. « Ce résultat satisfait l'étape de tests de ce workflow » est un jugement fondé sur le contrat effectif. Aucune de ces formulations ne signifie absence de bugs. Une omission de test reste `not_requested` ou `not_observed` ; elle n'est jamais convertie en `passed`.

## Chaîne d'identités

Chaque événement métier durable possède un identifiant, une séquence locale monotone, une version de schéma, un horodatage et un lien de causalité. Il relie selon pertinence goal, mission, workflow_definition_version, run, cycle, step, agent logique, role_version, session, invocation, parent invocation, modèle effectif, skill versions, scope, documents et artefacts. Une invocation persiste après disparition du worker.

`InvocationManifest` décrit les entrées préparées pour chaque appel ; leur transmission est observée séparément. Il regroupe événements, instructions système visibles, contrats, skills/roles versions, documents pin, résultats de recherche et extraits, messages, références aux compactions, outils disponibles et configuration de modèle demandée. Le [ContextManifest v2](../schemas/context-manifest.schema.json) de chaque appel en fixe les segments et le modèle effectif ; la transmission et la réception viennent du ModelCallRecord et ne se déduisent pas de la préparation. L'interface affiche les contenus autorisés ou leur motif de masquage. Le manifeste n'invente pas de raisonnement privé du modèle et ne le demande pas. Un résumé de décisions rédigé par l'agent peut être conservé comme tel ; il ne devient pas une transcription de chain-of-thought.

`ActionAttempt` relie intention, autorisation, dispatch, observations, erreurs et résolution. L'idempotency key de commande permet de reconnaître la redélivrance au harness ; elle ne garantit pas qu'un service extérieur sait dédupliquer. Les données inconnues restent inconnues. Un retry est un nouvel essai lié au précédent, pas un remplacement silencieux de sa chronologie.

## Preuves de code et de validation

Une `EvidenceArtifact` contient type, source, digest de contenu, taille, media type, stockage, ACL, rétention, disponibilité et références de snapshot. Pour un test : commande structurée et arguments expurgés, répertoire, processus identifié, environnement déclaré, toolchain, début/fin, statut de terminaison, sortie capturée, troncature et résultats structurés si disponibles. Une fin de processus perdue produit `termination_unknown`, pas un code 0 par défaut.

Les preuves deviennent périmées lorsque leur périmètre change. Le système compare snapshot et contexte de build, pas seulement le nom du fichier. Si une modification démontrée hors périmètre autorise réutilisation, la règle et le jugement de réutilisation sont conservés. Une vérification sur une branche feature ne prouve pas automatiquement le candidat de merge. Le rapport expose `verified_snapshot` et `delivered_snapshot`, y compris leur différence.

## Recherche avant création et reçu

`DiscoveryReceipt` est une preuve procédurale : intention, requête, scopes, ressources consultées, versions/générations, couverture et limites, résultat, choix `reuse` ou `create`, stratégie de coopération et justification. Il couvre code, APIs, tests, contrats, skills, rôles et ressources techniques. La copie d'un helper dans un domaine différent peut être une décision valable avec sa divergence explicitée.

Le contrôle de clôture relie les créations identifiées aux reçus valides. Une opération effectuée sans reçu antérieur ne peut pas être réparée par un horodatage fictif ; le dossier distingue recherche a posteriori et recherche préalable. Le harness signale les créations non classifiées lorsque l'outil ne permet pas d'identifier complètement leur sémantique. Le niveau d'assurance dépend des opérations observables et du contrat ; il n'existe pas de garantie d'absence absolue de duplication.

Un commit, rebase, merge, suppression, changement de registry ou crash peut périmer la vue actuelle et impose une réconciliation. La preuve historique de recherche préalable reste liée à son DiscoveryUseObservation : la propre création exacte et sa publication identique démontrée ne détruisent pas cette preuve. Toute autre mutation pertinente exige une découverte completion_revalidation liée à cet usage et à la cible/version livrée. Le rapport distingue conformité préalable et fraîcheur actuelle ; une recherche tardive ne remplace jamais une recherche absente avant création.

## Écriture durable et observabilité

État métier, événement et outbox sont écrits atomiquement dans la même base autoritaire lorsqu'ils concernent la même transition. La diffusion aux lecteurs peut être au moins une fois ; chacun déduplique avec un identifiant et conserve son curseur. Aucun I/O réseau, processus Git ou indexage lourd n'est effectué pendant la transaction. Un rapport dérivé peut être reconstruit ; l'intention et l'observation historiques ne sont pas régénérées depuis un résumé LLM.

Les traces et métriques techniques servent au diagnostic. [OpenTelemetry](https://opentelemetry.io/docs/specs/otel/overview/), consulté le 16 novembre 2026, distingue ces signaux ; Unleash propose un export compatible sans faire du collecteur la source de vérité métier. [W3C Trace Context](https://www.w3.org/TR/trace-context/), consulté le même jour, fournit un format de corrélation HTTP, pas une autorisation. `trace_id` est relié à `invocation_id` mais ne le remplace pas.

Les métriques V1 couvrent invocations actives par niveau, files d'attente, budgets auxiliaires, appels et retries fournisseur, consommation rapportée, coûts estimés distincts des coûts facturés, latence des outils, backlog d'événements, retard d'index, conflits Git, actions d'issue inconnue, preuve manquante et erreurs d'accès. Les labels excluent texte de prompt, secrets, noms arbitraires à cardinalité non bornée et corps de documents.

## Rapports reproductibles

`ReportSnapshot` capture l'état courant et son curseur {stream_id, sequence} dans une même vue transactionnelle ; il fixe versions de schéma, filtres et politique d'accès. Un rapport historique est relu depuis son snapshot conservé. Aucune date ou séquence arbitraire n'est demandable ; un rapport historique purgé donne HistoryUnavailable ou missing_sources explicite, jamais une reconstitution imaginaire de l'état. Il comprend objectif, définition de done effective, état réel, décisions importantes, livrables, vérifications demandées/exécutées/non demandées, écarts claims-observations, reçus discovery, dépendances, erreurs et incertitudes. Un résumé LLM est facultatif et ses propositions doivent citer les éléments du rapport ; l'export déterministe reste disponible.

GUI, CLI et TUI interrogent le même service de projection. Un client reconnecté reprend par curseur ; si sa fenêtre a été purgée, il reçoit la réponse ResyncRequired (Error commun, sans événement de journal) et recharge un snapshot cohérent. Les états canoniques — agent `dormant`/`blocked` et invocation `running`/`waiting_durable` ([state-machines](../architecture/state-machines.md)), processus `exited`/`suspect`/`lost` (F011, [sqlite-design](../architecture/sqlite-design.md)), daemon `recovering` ([runtime-lifecycle](../architecture/runtime-lifecycle.md)) — et l'indisponibilité fournisseur signalée par événement sont représentables sans déduire que toute absence d'activité est un crash. Une perte de connexion du client ne change pas l'état du daemon.

## Secrets, accès et rétention

La première défense est la minimisation : ne pas enregistrer des clés en clair dans les paramètres et ne pas inclure l'environnement intégral dans une preuve. Les champs structurés secrets sont masqués avant journal et export. Les flux libres sont filtrés, bornés et éventuellement conservés dans une zone distincte chiffrée selon politique ; leur filtrage ne garantit pas de détecter un secret transformé ou jamais connu du système. Par défaut, aucun journal brut général n'est exporté.

Les vues, téléchargements, exports et contextes modèles recontrôlent les ACL. Une suppression d'artefact laisse métadonnées minimales et motif `retained_metadata_only` ou `deleted_by_policy` si permis, sans maintenir un lien trompeur vers des octets disparus. Une exportation a son propre manifeste de destinataire/périmètre lorsqu'elle quitte la machine. La télémétrie réseau est désactivée par défaut ; son activation est un choix explicite de configuration.

La durabilité dépend de la politique de stockage et du matériel. Un journal plein ou non inscriptible bloque les nouveaux effets qui exigent un enregistrement préalable ; il n'est pas acceptable de continuer puis fabriquer un récit au retour de la base. Les effets déjà en cours sont drainés ou classés incertains et réconciliés. Les limites de taille et de rétention sont visibles avant d'effacer une preuve requise par un contrat actif.

## Provenance discovery sans agent ni Git — R6

DiscoveryReceipt emploie origin invocation/replay/control_plane et TypedSourceRef git_snapshot/resource_snapshot/file_snapshot/registry_version selon [shared-knowledge](shared-knowledge.md). Principal/cause sont authentifiés côté backend ; aucune session factice. DiscoveryUseObservation capture les préconditions vérifiées à l’autorisation puis au dispatch de la création exacte. GetDiscoveryReceipt joint au reçu immuable une projection current_freshness/use_state avec watermark. Une revalidation de clôture reste un nouvel objet, jamais une réécriture d’antériorité.
