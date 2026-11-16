# R6 — Expert 05 — LLM API & Context Engineer

**Revue documentaire indépendante, 16 novembre 2026.** Périmètre : F012, routage/localité, manifeste de contexte, sessions, registres et recherches fournisseurs. Aucun appel facturé, serveur local ou test produit exécuté. Les trois constats concernent des contrats incomplets, sans prétendre démontrer un défaut d’implémentation.

Les priorités, le skill principal issu de `role.entry_skill`, les efforts natifs qualifiés, les coûts inconnus, les refus de fallback et l’annulation locale incertaine sont déjà explicités. Les backends locaux restent distincts des fournisseurs cloud. Le probe sans invocation, traité par R6-E01-01, n’est pas recompté.

## R6-E05-01 — Restrictions des données dérivées non définies

**Priorité élevée. Constat confirmé.** [model-routing, Assemblage](../../specs/model-routing.md) exige provenance et conservation ; [session-model, Compaction](../../architecture/session-model.md) relie résumés et sources ; [F012 technique](../../features/F012-model-api-router/technical-design.md) contrôle la destination des documents. Aucun de ces contrats ne définit comment une compaction, un extrait, un résultat d’outil ou une sortie modèle hérite des restrictions de ses sources. Le [schéma de contexte](../../schemas/context-manifest.schema.json) ne porte aucune politique effective par segment ; `trust` n’est pas une politique de destination.

**Scénario / risque.** Un document `local_only` est résumé localement. Une nouvelle invocation autorisée au cloud sélectionne seulement ce résumé, après retrait du document initial. Sans règle d’héritage, elle peut transmettre son contenu malgré la restriction d’origine. L’absence de fallback silencieux ne couvre pas cette nouvelle sélection.

**Hypothèse / objection.** Les restrictions sont attachables aux documents, comme le prévoit F012. Une politique uniformément stricte pour tout le projet éviterait ce cas, mais ce confinement permanent n’est pas imposé et empêcherait certains mélanges de sources permis.

**Correction.** Définir la filiation des dérivés et leur politique effective versionnée : destinations et accès au moins aussi restrictifs que leurs sources, obligations de conservation compatibles, conflits bloquants. Résumé et redaction ne déclassifient pas automatiquement. Toute exception exige une autorité explicite et une trace ; une provenance insuffisante reste bloquante pour la destination concernée.

**Alternative.** Attacher conservativement la restriction la plus forte à toute la session et à ses sorties, avec nouvelle session explicitement autorisée pour élargir la diffusion.

**Acceptation.** Étendre T-F012-009/015 : document canari local, résumé puis résumé de résumé, retrait/purge de la source, nouvelle invocation cloud et fallback. Aucun dérivé interdit envoyé ; motifs et filiation inspectables après redémarrage. À confirmer : granularité retenue et autorité de déclassification.

## R6-E05-02 — Historique distant absent du contrat de contexte effectif

**Priorité élevée. Constat confirmé.** [La recherche API](../../research/verified-api-capabilities.md) admet des identifiants de conversation/réponse ; [F012 AC-010](../../features/F012-model-api-router/acceptance.md) prévoit leur invalidation et la reprise portable. Pourtant, manifeste et inspecteur décrivent le payload préparé, sans mode explicite interdisant ou développant l’historique référencé. La [documentation officielle OpenAI](https://developers.openai.com/api/docs/guides/conversation-state), consultée pendant cette revue, confirme que `previous_response_id` partage du contexte antérieur ; les entrées précédentes restent facturées et les objets Conversation ont une conservation distincte.

**Scénario / risque.** Après exclusion d’un document, le payload courant ne contient plus ce document mais référence une réponse qui l’utilisait. L’inspecteur peut annoncer son exclusion alors qu’un historique distant le réintroduit. Estimation d’entrée, autorisations et rétention peuvent également être calculées sur un ensemble incomplet.

**Correction recommandée.** Pour V1, fixer le mode de base : historique reconstruit depuis Unleash, sans référence de continuation distante implicite. Les blocs natifs requis restent explicites et liés à leur fournisseur. Les paramètres de stockage suivent une politique qualifiée ; absence d’état applicatif distant ne signifie pas absence de toute conservation fournisseur.

**Alternative / objection.** Conserver ces optimisations avec un contrat séparant payload réseau et contexte effectif développé : références exactes, dépendances, usage estimé, politique de conservation et invalidation après exclusion/révocation. Stocker une copie locale ne suffit pas à garantir l’équivalence avec un historique distant mutable.

**Acceptation.** Étendre T-F012-009/010/011 : référence distante existante puis périmée, exclusion locale, reprise et changement fournisseur. Aucun segment exclu réintroduit ; budget fondé sur le contexte applicable ; provenance inconnue bloquée. À confirmer : mode V1 et options qualifiées par dialecte/compte.

## R6-E05-03 — Préférence du skill principal sans représentation normative

**Priorité modérée.** [REQ-MODEL-002 et AC-002](../../features/F012-model-api-router/acceptance.md) supposent des préférences concurrentes agent/rôle/skill/projet. [skill.schema](../../schemas/skill.schema.json) n’a aucun champ de préférence modèle ; son objet fermé rejette `model_policy`. Le catalogue propose `SetSettings`, mais aucun binding versionné ne relie cette préférence au skill exact.

**Preuve documentaire.** Validation en mémoire : `examples/skill.json` conforme ; ajout de `model_policy` rejeté par `additionalProperties`. Ce contrôle ne prouve aucun comportement runtime.

**Risque.** Deux équipes peuvent associer la préférence à un nom mutable, au skill publié ou à une configuration externe différente ; la résolution et le pinning divergent.

**Correction.** Définir un `ModelSelectionBinding` versionné, avec scope/identité exacte, préférence de profil, provenance et digest épinglé. Décrire son alimentation par `SetSettings`, sans augmenter les permissions. **Alternative :** champ optionnel de préférence dans la version immuable du skill.

**Acceptation.** AC-F012-002 : deux skills auxiliaires et un principal, homonymes et versions distinctes, ordre permuté, modification après admission ; même résolution historique, aucun effort incompatible transmis. À confirmer : propriété de cette configuration et articulation avec publication F007.

## Addendum — corrections documentaires intégrées

Les constats ci-dessus décrivent l'état avant correction. Les contrats ferment désormais 01 par politique/filiation transitive et déclassification bornée ; 02 par historique explicite sans continuation distante ; 03 par ModelSelectionBinding versionné externe au skill. ContextManifest passe en v2 avec fixture synthétique. Vérification exécutée : jsonschema 4.26.0, formats UUID/date présents (date-time absent du checker initial, repris par validation globale R6), trois variantes conformes et dix variantes invalides rejetées ; liens internes résolus. ProviderProbeContext et origin_ref sont conservés. Qualification fournisseur et tests produit : `not_run`.
