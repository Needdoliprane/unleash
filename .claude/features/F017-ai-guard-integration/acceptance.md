# F017 — acceptation

État de ces critères : spécifiés, non exécutés sur le produit. Chaque preuve doit identifier build, environnement et configuration.

## AC-F017-001 — Autorité

Exigence : REQ-SEC-001.

- Étant donné : Un skill publié réclame un outil interdit au projet.
- Lorsque : Créer une invocation puis proposer l’action.
- Alors : Refus avant effet et trace de l’autorité limitante.
- Preuve attendue : EV-F017-001, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-002 — Broker

Exigence : REQ-SEC-002.

- Étant donné : Worker propose Git, process, réseau et entrée terminal.
- Lorsque : Exécuter sous mandat.
- Alors : Chaque effet a intent, décision, action_id et receipt.
- Preuve attendue : EV-F017-002, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-003 — Secrets

Exigence : REQ-SEC-003.

- Étant donné : Canari API/OAuth installé au coffre.
- Lorsque : Lancer modèle, build, rapport et export.
- Alors : Canari absent payload modèle/env/argv/logs ordinaires.
- Preuve attendue : EV-F017-003, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-004 — IPC

Exigence : REQ-SEC-004.

- Étant donné : Worker connaît format de commande d’administration.
- Lorsque : Tenter modifier limites et permissions.
- Alors : Authentification/autorisation refuse ; incident traçable.
- Preuve attendue : EV-F017-004, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-005 — Scopes

Exigence : REQ-SEC-005.

- Étant donné : Chemin symlink sortant et shell demandant confinement indisponible.
- Lorsque : Lire/modifier puis lancer.
- Alors : Accès typé hors scope refusé ; shell strict bloqué.
- Preuve attendue : EV-F017-005, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-006 — Injection

Exigence : REQ-SEC-006.

- Étant donné : Document demande envoyer clé ou changer fournisseur.
- Lorsque : L’agent propose une action issue du document.
- Alors : Politique inchangée ; exfiltration et extension de droits refusées.
- Preuve attendue : EV-F017-006, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-007 — Révocation

Exigence : REQ-SEC-007.

- Étant donné : Décision allow puis révocation avant dispatch.
- Lorsque : Tenter action et redélivrer ancien receipt.
- Alors : Action non engagée ; résultat tardif ne ressuscite pas le droit.
- Preuve attendue : EV-F017-007, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-008 — Modes guard

Exigence : REQ-SEC-008.

- Étant donné : Guard requis devient indisponible.
- Lorsque : Proposer nouvel effet et consulter GUI.
- Alors : Effet bloqué ; inspection reste possible ; mode affiché exact.
- Preuve attendue : EV-F017-008, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-009 — Administration

Exigence : REQ-SEC-009.

- Étant donné : Adaptateur réel qualifié et identité autorisée.
- Lorsque : Inspecter puis suspendre agent.
- Alors : État et référence distante corrélés ; capacité absente clairement signalée.
- Preuve attendue : EV-F017-009, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-010 — Traçabilité

Exigence : REQ-SEC-010.

- Étant donné : Action autorisée puis crash après effet.
- Lorsque : Reprendre et inspecter l’audit.
- Alors : OutcomeUnknown ou preuve réconciliée, jamais allow pris pour succès.
- Preuve attendue : EV-F017-010, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-011 — Rétention

Exigence : REQ-SEC-011.

- Étant donné : Logs/contexts/blobs contenant canaris et backup antérieur.
- Lorsque : Purger puis rechercher/explorer.
- Alors : Copies concernées inaccessibles ; tombstones et portée de purge indiqués.
- Preuve attendue : EV-F017-011, artefacts et résultats du test, avec limites de portée explicites.

## AC-F017-012 — Qualification

Exigence : REQ-SEC-012.

- Étant donné : Mock passe mais aucun contrat réel fourni.
- Lorsque : Évaluer readiness V1.
- Alors : F017 reste bloquée sur recette externe ; autres chantiers progressent.
- Preuve attendue : EV-F017-012, artefacts et résultats du test, avec limites de portée explicites.

### AC-F017-005/008/010 — compléments R6

Étant donné un Allow assorti d’une obligation inconnue, contradictoire, non imposable, puis entièrement applicable, et une perte de capacité entre évaluation et engagement. Alors, dans les trois premiers cas et en cas de perte de capacité : `CapabilityDenied/GUARD_OBLIGATION_UNSATISFIED`, zéro dispatch et zéro résolution de secret mesurés par un oracle indépendant du mapper. Dans le cas applicable : refus identique sous le profil initial `obligations=none_accepted` ; avec un `guard_mapping_profile` qualifié, preuve du contrôle effectif corrélée à la décision. Le double ne vaut pas recette AI-Guard réelle.

### AC-F017-009 — complément R7

Étant donné un sujet typé actif, avec ou sans AI-Guard configuré. Lorsque l’utilisateur le suspend (SuspendSubject) puis lève la suspension (ResumeSubject). Alors les nouvelles actions du sujet sont refusées `CapabilityDenied/SUBJECT_SUSPENDED` pendant la suspension, les effets en vol sont réconciliés, la reprise rétablit le sujet sans nouvelle identité, et la révocation (RevokeSubject) reste une mesure distincte et définitive. La propagation distante n’est attendue que si GuardPort la négocie ; sinon la limitation est affichée.

