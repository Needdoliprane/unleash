# F008 — Tests et preuves attendues

**Statut produit : `not_run` pour tous les T/EV de ce document.** Les validations documentaires de schémas/fixtures sont distinctes d'une exécution du moteur Rust.

| Test | AC | Procédure | Preuve attendue |
|---|---|---|---|
| T-F008-001 | AC-F008-001 | Output conforme mais mensonger ; schéma distant interdit. | EV-F008-001 : pass structure distinct et refus/unknown de preuve. |
| T-F008-002 | AC-F008-002 | Validateur non catalogué, code dans paramètres, excès taille ; changement evaluator entre checkpoint et reprise. | EV-F008-002 : refus, zéro exécution libre, versions préépinglées conservées ou blocage explicite. |
| T-F008-003 | AC-F008-003 | Révoquer grant puis proposer auto-augmentation N+2. | EV-F008-003 : calcul d'intersection et refus d'action. |
| T-F008-004 | AC-F008-004 | Substituer ID de claim à observation ; auteur/producteur forgé ; proposition qualitative, prédicat ou version inconnus. | EV-F008-004 : types/provenance contrôlés, qualitatif inconnu et aucun interpréteur libre. |
| T-F008-005 | AC-F008-005 | Attendu préadmis puis suite requise en échec/filtre bénin réussi ; muter séparément subject/revision/scope/snapshot, outil, argv, cwd, environnement, suite, parseur, binding/action. | EV-F008-005 : comparaison à l'attendu autoritaire et aucune sélection de preuve favorable ; refus/unknown motivés. |
| T-F008-006 | AC-F008-006 | Observation absente, adaptateur indisponible, purge. | EV-F008-006 : matrice pass/fail/unknown et blocages. |
| T-F008-007 | AC-F008-007 | Prédicats process.exit_code/test.expectation_satisfied V1 ; valeur vraie/fausse, texte paraphrasé, preuve omise, exit 1, zéro cas, cas ignoré ou parsing incomplet. | EV-F008-007 : matrice de vérité bornée, contradictions conservées, unknown sans juge, échec de test distinct de vérité du claim. |
| T-F008-008 | AC-F008-008 | Origins invocation/replay/control_plane ; sources code/fichier/registre ; usage historique puis effet propre exact, retouche ultérieure, revalidation et préalable absent. | EV-F008-008 : F009 appliqué, aucun agent fictif, aucune antidatation et limites/ACL/bornes visibles. |
| T-F008-009 | AC-F008-009 | Fixtures déterministes avec fournisseur juge désactivé. | EV-F008-009 : publication et verdict sans juge obligatoire. |
| T-F008-010 | AC-F008-010 | Lecture hors ACL, redaction, purge et replay. | EV-F008-010 : refus, tombstone et limite explicite. |

La campagne de mutation doit modifier les champs de preuve un par un pour vérifier que le verdict dépend des propriétés déclarées. La campagne de concurrence change l'état du sujet entre assemblage de bundle et acceptation ; le verdict ancien doit être refusé. Les validateurs sont testés sur des contre-exemples métier pertinents, et pas uniquement sur leur propre sérialisation. Les propriétés testées et leurs limites sont consignées dans le rapport afin de ne pas transformer les résultats en promesse d'absence de bugs. Tous ces tests sont planifiés pour le produit, distincts de la vérification documentaire du dossier.

## Corpus R6 et oracle

Les fixtures synthétiques [attendu](../../examples/expected-test-execution.json), [binding](../../examples/expected-execution-binding.json), [observation](../../examples/test-execution-observation.json), [claim](../../examples/contract-claim.json), [skill](../../examples/test-verification-skill.json) et [profil](../../examples/test-verification-contract-profile.json) décrivent un cas nominal proposé. Leurs IDs, outils, versions et digests de sources externes sont fictifs ; ils n'attestent ni disponibilité d'un outil ni exécution de tests. Le digest d'attendu, du contrat et des règles est calculable sur les objets joints. Leur future mise en scène crée les enregistrements autoritaires correspondants, avec dispatch après `sealed_cursor`.

| Variation supplémentaire | Oracle attendu, produit `not_run` |
|---|---|
| Changer seulement le texte du claim | Vérité typée et verdict identiques, aucun appel modèle |
| Claim qualitatif, projection absente, parsing incomplet | Truth unknown ; règle obligatoire bloque/échoue selon on_unknown |
| Claim affirme false et suite attendue échoue | Contrôle de vérité pass, règle de réussite de test fail |
| Observation d'échec omise de observation_ids | Reconciler consulte le journal de l'action liée ; échec non masqué |
| Injecter attendu via output, remplacer slot après observation, mauvaise authority_ref | Admission/référence refusée ; aucun pass obtenu de cette attente |
| Retry autorisé, preuve de la tentative précédente | Expectation identique, nouveau binding/action courant ; ancienne preuve ne satisfait pas l'action courante |
| Crash après scellement avant dispatch ; catalogue v1 seul disponible | Scellement repris sans substitution ; nouvelle règle v2 indisponible bloque |
| Deux observations authentifiées incompatibles du même fait | unknown/EvidenceAmbiguous, aucune sélection du résultat favorable |
| Profil exige vérité, remplacer claim_predicate_matches par liens ou informative | Publication/binding refusés par la règle obligatoire et son digest |
| Discovery valide à l'effet puis cible modifiée par le même auteur | Nouvelle completion_revalidation nécessaire ; même auteur n'est pas exemption |

Les contrôles de schéma testent aussi phase input interdite aux nouvelles règles, slot ressemblant à `/output/...`, prédicat/version inconnus, types de valeurs, champs libres et états de résultat incohérents. Les tests d'autorité/FK, égalité des identités, ordre des séquences et calcul des verdicts restent des tests de domaine à implémenter, jamais déduits du seul pass de schéma.
