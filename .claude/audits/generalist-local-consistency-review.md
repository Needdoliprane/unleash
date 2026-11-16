# Revue contradictoire indépendante — généralisation, automatisations et modèles locaux

Date : 16 novembre 2026. Périmètre : consolidation R3–R5 du dossier OVPT Unleash. **Statut final de revue documentaire : quatre raccords relevés, corrigés puis relus ; aucune contradiction restante établie dans le périmètre inspecté.** L’état initial et les alternatives sont conservés ci-dessous. Cette revue porte sur des contrats proposés et des fixtures de conception. Elle ne qualifie aucun produit, moteur navigateur, modèle local, plateforme ou parcours utilisateur.

## Constats initiaux prioritaires

### GLC-01 — Origine durable des actions mécaniques insuffisamment raccordée au modèle de stockage

**Constat confirmé au premier passage ; importance moyenne.** [Le modèle d’artefact](../architecture/automation-artifact-model.md), § « Interdiction transitive des appels modèle », et [le design F019](../features/F019-reusable-automations/technical-design.md), § « Enforcement de la politique modèle », exigent une origine vérifiable pour toute action et tout descendant. Une origine manquante doit empêcher le dispatch. Le replay ne crée aucune invocation agente.

Dans l’état initial relu, la ligne `action_id` du [modèle de stockage](../architecture/storage-model.md) décrivait seulement une relation à une invocation. La table `actions` du [dictionnaire SQLite](../architecture/sqlite-design.md) rendait `invocation_id` et `attempt_id` optionnels sans désigner leur remplacement obligatoire pour une action mécanique. Les extensions R3–R5 détaillaient missions, runs et réservations, mais pas ce raccord. Le [modèle événementiel](../architecture/event-model.md), § « Origine d’exécution et jobs de conception distincts », imposait bien une origine dans les manifests/payloads ; le constat est donc une divergence de précision entre contrats canoniques, pas la démonstration d’un contournement exécuté.

**Conséquence :** un implémenteur suivant le dictionnaire peut enregistrer une action sans invocation et sans relation structurée au run/à l’activation qui lui interdit les modèles. La reprise, l’attribution des effets et la preuve du zéro modèle ne disposent alors pas du même contrat de liaison que celui exigé par F019.

**Correction recommandée :** rendre l’origine d’exécution explicite et typée ; relier l’action à son run, son activation et son worker lorsqu’ils s’appliquent, avec intégrité référentielle et exclusivité de provenance. Conserver le rattachement de l’effet lors des changements de tentative et distinguer les opérations du plan de contrôle des invocations agentes et des replays.

**Alternative :** porter une référence obligatoire à un manifest d’origine typé et immuable, vérifiée transactionnellement, plutôt que multiplier des colonnes optionnelles. Les champs projet/mission/run nécessaires restent interrogeables et la même origine est présentée au broker et au routeur modèle. Créer un faux agent pour satisfaire les anciennes relations contredirait le parcours sans agent.

**Relecture du 16 novembre 2026 : corrigé dans les documents.** Le dictionnaire SQLite décrit désormais `origin_kind=invocation/replay/control_plane`, les FK requises, les combinaisons exclusives, les liens worker/activation/run, la cause durable des opérations de contrôle et la dérivation de l’origine depuis le canal authentifié. La ligne `action_id` du modèle de stockage reprend ces trois origines. [SubmitObservation F005](../features/F005-workflow-engine/interfaces.md) accepte explicitement un principal agent ou worker mécanique et rend l’invocation conditionnelle. La réparation requiert toujours un mandat distinct. Ces précisions ferment le raccord documentaire ; aucune migration ou protection runtime n’a été testée.

### GLC-02 — Deux axes de mode utilisent encore un même nom dans des documents centraux

**Constat confirmé au premier passage ; importance faible.** [Le contrat d’artefact](../architecture/automation-artifact-model.md), § « Quatre modes explicites », distingue `automation_mode` pour `learn/replay/supervised/repair` et `execution_mode` pour le profil navigateur `mediated/project_script`. [Le modèle de session](../architecture/session-model.md), § « Run mécanique et supervision optionnelle », décrivait pourtant `AutomationRun` avec `execution_mode` puis discutait immédiatement du mode `supervised`. [La spécification de mission](../specs/goal-management.md), § « Missions généralistes et démarrage guidé », introduisait aussi `execution_mode` sans définir l’axe.

**Conséquence :** les projections et le parcours guidé peuvent désigner deux dimensions différentes par le même champ, alors que sélection d’un mode de décision et choix d’un profil de driver doivent être indépendants et épinglés.

**Correction recommandée :** employer `automation_mode` dans les projections du run lorsque le champ désigne le choix replay/supervision, et qualifier explicitement le profil navigateur lorsque `execution_mode` est utilisé. Pour la mission, nommer l’axe voulu ou référencer le contrat canonique sans créer de champ homonyme ambigu.

**Alternative :** conserver un objet `execution_profile` contenant deux propriétés distinctes et nommées, avec conversion explicite vers les DTO déjà proposés. Aucun changement de mode en vol n’est impliqué.

**Relecture du 16 novembre 2026 : corrigé dans les documents.** `session-model` décrit maintenant `automation_mode` et nomme séparément `execution_mode=mediated/project_script` ; `MissionDefinition` utilise `automation_mode` lorsque applicable. Les projections F019 et le catalogue client emploient ce même axe.

### GLC-03 — La table de version ne représentait pas les deux workflows d’une version bimode

**Constat confirmé lors de la relecture des extensions SQLite ; importance moyenne.** [Le contenu d’une version publiée](../architecture/automation-artifact-model.md), [PublishAutomationVersion](../features/F019-reusable-automations/interfaces.md) et AC-F019-001 prévoient une version d’artefact avec un workflow exact par mode supporté ; la même version peut fournir replay/`forbidden` et supervised/`allowed_bounded`.

La nouvelle ligne `automation_versions` du [dictionnaire SQLite](../architecture/sqlite-design.md), § « Tables et contraintes ajoutées R3–R5 », portait pourtant un seul `workflow_version_id` et un seul `model_policy`, sans clé de mode ni mapping documenté. La table `automation_runs` sélectionnait un mode sans préciser comment le résoudre dans cette cardinalité.

**Conséquence :** deux implémentations peuvent représenter différemment la même version publiée : une imposerait un workflow/policy unique, l’autre conserverait les deux graphes promis par le contrat. Le problème porte sur la représentation du bundle ; il ne démontre aucun changement de politique réel en cours de run.

**Correction recommandée :** associer les modes d’exécution à la version avec une clé `(automation_version_id, automation_mode)` et leurs références exactes de workflow/policy ; référencer cette association depuis le run.

**Alternative :** un mapping typé immuable dans le manifest publié peut porter ces associations, à condition que les colonnes soient clairement des projections résolues et que la sélection soit contrôlée avant admission. Les phases de conception `learn/repair` restent des jobs distincts, pas des modes exécutables inventés dans cette association.

**Relecture du 16 novembre 2026 : corrigé dans les documents.** `automation_versions` porte désormais le bundle commun ; `automation_version_modes` a pour clé `(automation_version_id, automation_mode)` et référence workflow/policy/driver/contrat exacts. `automation_runs` référence ce mode par FK composite, avec vérification transactionnelle de l’égalité de sa résolution. Les sections « Modes publiés et jobs de conception » du modèle d’artefact et des interfaces F019 distinguent les deux modes publiés des jobs `learn/repair`, refusent un mode absent et conservent le run engagé. La cardinalité documentaire est cohérente ; aucune migration SQL n’est prétendue exécutée.

### GLC-04 — Récurrence proposée dans le parcours, absente des opérations et de la recette

**Constat confirmé lors de la revue transversale ; importance moyenne.** [L’expérience guidée](../design/guided-automation-experience.md), § « Planifier si cela correspond au besoin » et ligne « Configurer une récurrence », annonçait un formulaire GUI/TUI de fréquence, fuseau, données, version, chevauchement et échéances manquées. [La spécification d’automatisation](../specs/reusable-automations.md), § « Récurrence et changement de procédure », décrivait le timer créant les runs correspondants.

Le [catalogue commun](../architecture/client-operation-catalog.md), les 58 groupes de la [matrice de parité](../design/gui-tui-parity.md) et les exigences/interfaces/tests F019 ne définissaient aucune opération de création, modification, suspension ou inspection de cette configuration. `ConfigureWatch` concerne les documents ; `SetSettings` ne décrit pas de contrat de déclenchement récurrent. Le scheduler d’admission global ne remplace pas ce contrat métier.

**Conséquence :** le parcours demande une capacité que les clients ne peuvent raccorder à une opération commune ni à une preuve de recette identifiée. La répétition manuelle d’une procédure ne couvre pas la planification horaire.

**Correction possible A :** spécifier une configuration persistante typée, réutiliser le timer et `CreateRun` communs, définir ses opérations/révisions/mandats et leur couverture de parité/recette. Aucun agent permanent ni second scheduler autonome n’est nécessaire.

**Alternative B :** qualifier cette planification comme piste de conception distincte, non demandée explicitement par le brief R3–R5 et non revendiquée dans le parcours V1 courant ; supprimer la promesse normative correspondante sans retirer la réutilisation manuelle ni les capacités antérieures confirmées. Le choix doit être identique dans les deux documents concernés.

**Relecture du 16 novembre 2026 : corrigé par délimitation explicite du périmètre, alternative B retenue.** La spécification et le parcours guidé conservent le nouveau lancement explicite avec d’autres données ; la ligne GUI/TUI devient « Préparer une réutilisation ». Le cas GEN-05 du [périmètre généraliste](../brief/general-purpose-scope.md) décrit le rapport par période avec lancement explicite. [Q-21](../brief/open-questions.md) conserve la planification calendaire comme option non confirmée, avec opérations et preuves à définir si elle est adoptée. Le brief R3–R5 et DOC-07 de la readiness checklist signalent cette décision. Il ne s’agit pas d’une fonctionnalité demandée arbitrairement reportée à une V2 : réutilisation, scheduler d’admission, timers internes et réparation préautorisée restent conservés. La question de produit demeure ouverte ; la promesse sans contrat a disparu des parcours normatifs.

## Contrôles documentaires réellement exécutés

Lecture croisée du [brief autoritaire R3–R5](../brief/revision-2026-11-16-r3-r5.md), des [invariants](../brief/invariants.md), des contrats d’artefact/exécution/navigateur, des spécifications d’automatisation et de modèles locaux, du parcours guidé, des sept documents F019, des exigences/acceptations F004 et F012, des scénarios W–Z et des plans de qualification liés. Les raccords ciblés au stockage, aux sessions, au DSL, aux états, à la clôture et au scheduler ont été consultés ; il ne s’agit pas d’une nouvelle revue exhaustive des dix-huit features initiales.

Un contrôle en mémoire avec `jsonschema.Draft202012Validator` a été exécuté en lecture seule sur les fichiers présents :

- Métaschémas et conformité de cinq fixtures : workflow de rapprochement, workflow supervisé, ObservationReport, rôle superviseur et skill superviseur. Aucun écart de forme observé.
- Cinq contre-exemples sous `forbidden` : tâche agente directe, dans `sequence`, `choice`, `repeat` et `parallel`. Tous rejetés par le schéma v1.1.
- Quatre budgets positifs sous `forbidden` : appels, tokens de sortie, coût modèle et exécutions agentes. Tous rejetés.
- Quatre contradictions d’observation : effet inconnu avec poursuite, effet inconnu sans référence d’action, `expected_failure` avec poursuite et effet connu sans référence d’action. Toutes rejetées.
- Présence des dossiers F001 à F019 et de leurs sept fichiers canoniques vérifiée ; les 58 groupes PAR possèdent 58 identifiants distincts. Ce contrôle d’inventaire ne remplace pas une comparaison exhaustive du contenu initial.
- Après la correction de la fixture supervisée, les cinq fixtures ont été relues et leur conformité de forme revérifiée. Binding de rôle exact, entry skill membre du rôle, contrat référencé et inputs de la tâche compatibles avec le skill ont aussi été contrôlés. La garde de fin lit maintenant le résultat du bloc racine terminé, sans dépendre directement de la projection d’une branche interne.

Ces contrôles établissent seulement le comportement des schémas documentaires sur ces exemples. Ils ne vérifient ni compilateur sémantique, ni autorisations réelles, ni émission réseau, ni moteur de workflow. Les preuves EV produit restent planifiées/non exécutées.

## Points examinés sans contradiction établie au premier passage

| Tension recherchée | Lecture constatée et limite |
| --- | --- |
| Version épinglée / réparation / supervision | La réparation est un travail distinct, mandaté à la demande ou par politique préautorisée. Les branches de supervision sont publiées ; un avis ne remplace ni script, ni locator, ni données du run engagé. Les coûts restent attribués à la mission. |
| Zéro modèle / rapport / manager / embedding / local | Le replay interdit les appels directs et indirects, conserve les managers dormants et produit le rapport par template. Le périmètre exclut explicitement les traitements internes opaques du service tiers. La réalité de l’enforcement reste à qualifier. |
| Effet inconnu / décision `continue` | Le rapport conserve l’inconnu ; schéma, contrat sémantique, broker et barrières de succession ont des responsabilités distinctes. Un log ou un jugement ne suffit pas à convertir l’effet en réussite. |
| Capacité agents / workers mécaniques | Le pool global partagé vaut cinq unités par défaut et cinquante maximum ; les compteurs navigateur, modèle et processus sont additionnels en ressources, pas un deuxième pool de cinquante exécutions. |
| Sans dépôt / sans agent / clôture | OwnerRef utilisateur et clôture `mechanical` ou `human` permettent le cas généraliste. Le profil de développement conserve ses rôles, Git et ses critères propres. Le raccord d’origine des actions est couvert séparément par GLC-01. |
| Localité / authentification / capacités | La localité porte sur le placement d’inférence, pas l’URL loopback. Authentification, transport, capacités et placement sont qualifiés séparément ; la documentation ne présente pas un serveur local déclaré comme automatiquement conforme. |
| Consentement d’apprentissage / replay | La version publiée ne transfère pas les grants de l’apprentissage ; chaque effet reste limité par le mandat actuel. Les autorisations préexistantes applicables sont réutilisables, sans confirmation systématique ajoutée. |
| Prototype documentaire / produit opérationnel | Les fixtures, DTO, ports, tests et choix de moteur sont annoncés comme propositions. Les démonstrations synthétiques ne sont pas présentées comme achats réels, qualification Carrefour ou intégrations locales réussies. |
| TUI / CLI / novice | Le parcours guidé exige les mêmes objets et opérations dans la TUI. L’ouverture autorisée d’une image ou d’un navigateur externe n’est pas assimilée à un passage obligatoire par la GUI Unleash. La revue transversale ne relève pas d’habillage CLI à la place des fonctions TUI ; le trou propre à la récurrence est corrigé selon GLC-04. |

## Questions et limites assumées

Les mécanismes d’isolation OS, le comportement des moteurs réels, le placement vérifiable des modèles et l’utilisabilité du parcours novice sont des qualifications explicitement futures. Leur absence de preuve produit n’est pas reclassée en contradiction documentaire. La garantie sur un script libre demeure conditionnée à sa qualification ; le dossier ne promet pas de déduire l’absence d’inférence de n’importe quel programme.

La compatibilité sémantique complète des opérations de démonstration reste ouverte : leurs adaptateurs ne sont pas implémentés et leur seule conformité JSON n’en prouve pas le résultat. Q-21 est une question optionnelle de périmètre, pas une preuve de capacité ou une contradiction persistante. La revue ne déclare aucun `passed` produit et ne réexécute pas les quatorze audits initiaux. Aucune source externe nouvelle n’a été inventée ou utilisée pour attribuer une capacité à un moteur.

## Relecture après consolidation

Le catalogue d’opérations, la matrice GUI/TUI, les plans, le scheduler et les frontières de sécurité ont été relus après le signal de fin de leur rédaction. Les quatre corrections ont ensuite été inspectées dans les fichiers réels ; elles ne sont pas closes sur la seule annonce de l’intégrateur.

| Constat | État après relecture | Ce qui a été vérifié |
| --- | --- | --- |
| GLC-01 Origine des actions | Corrigé documentairement | Origine typée obligatoire, FK et cohérence d’appartenance, cause de contrôle, dérivation par canal/mandat, soumission d’observation sans invocation artificielle. |
| GLC-02 Noms de modes | Corrigé documentairement | `automation_mode` dans run/mission et profil navigateur séparé dans le modèle de session. |
| GLC-03 Version bimode | Corrigé documentairement | Association immuable par mode, FK composite du run, résolution épinglée et phases de conception distinctes. |
| GLC-04 Récurrence | Corrigé par périmètre explicite | Rejeu manuel maintenu, promesse de calendrier retirée du parcours normatif, Q-21 et GEN-05 concordants. |

> **Note R7 (16 novembre 2026) :** des résidus de GLC-02/GLC-03 subsistaient dans le modèle d’artefact et la spécification navigateur. La règle canonique est : `automation_mode` ne prend que `replay` ou `supervised`, seules valeurs publiables ; `learn` et `repair` sont des jobs de conception ([modèle d’artefact](../architecture/automation-artifact-model.md), [stockage](../architecture/sqlite-design.md)). Q-21 est écartée de la V1 ([registre](../brief/open-questions.md)).

**Avis final limité à cette revue :** les ajouts R3–R5 inspectés peuvent être conservés comme proposition d’architecture cohérente avec leurs limites déclarées. Aucun constat restant ne justifie de retirer une feature initiale ou d’ajouter un second moteur. Les gates d’implémentation, de sécurité, de compatibilité réelle et d’expérience utilisateur restent à exécuter et à prouver.
