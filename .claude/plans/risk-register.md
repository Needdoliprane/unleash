# Registre des risques, hypothèses et gates

Statut : risques de conception, non vulnérabilités constatées dans une implémentation. Probabilité qualitative ; les tests cherchent à réfuter les garanties.

| ID | Risque / gravité | Hypothèse liée | Prévention / décision | Vérification et owner |
| --- | --- | --- | --- | --- |
| R-01 | Double effet après crash / critique | H-02 | Intent + unknown + réconciliation, ADR-01/02 | F018 fault points, Reliability |
| R-02 | PID réutilisé ou stale worker / critique | H-04 | Identité génération + fence, ADR-05 | F003/F011/F018, Systems |
| R-03 | Managers ou travaux mécaniques affamés / élevé | H-04/H-14/H-17 | Attentes durables, réserve souple d’une unité (Q-15), fairness et checkpoint coopératif déterministe, ADR-13 accepté ; aucune exemption 5/50 | Scénario I + saturation mixte F003/F019, Orchestration |
| R-04 | Coût incontrôlé / élevé | H-14/H-21 | Réservation, budget des managers/retries et inspections par checkpoint, ADR-11 | F012 concurrence, F019 supervision bornée, API |
| R-05 | Migration hot swap / critique | H-05 | Run immuable, frontière Q-03, ADR-04 accepté | Scénario K, Workflow |
| R-06 | Skills = prompts non vérifiables / élevé | H-07 | Contrats fermés observables, ADR-06/07 | F007/F008 mutations, Contracts |
| R-07 | Helpers non disponibles ou stale / élevé | H-08 | Snapshot/provenance/discovery, ADR-08 | Scénario D, Knowledge |
| R-08 | Captures dirty non atomiques / élevé | H-08 | Niveau de cohérence séparé | F009 modification concurrente |
| R-09 | Conflits Git fonctionnels / élevé | H-05 | Revue/intégration selon contrat, ADR-09 | Scénario G, Git |
| R-10 | Branche checkout corrompue par publication / critique | — (décision Q-10) | Jamais de mise à jour d’une ref checkout ; `blocked_target_checkout`, branche cible gérée ou libération du checkout puis réobservation (Q-10, ADR-09), CAS | F010 cible humaine occupée |
| R-11 | Fausse isolation shell / critique | H-11 | Profils de garanties, modes `standard`/`strict` (Q-05) et fail-closed, ADR-10 | F017 hostile build, Security |
| R-12 | Secrets/docs exposés / critique | H-12 | Coffre, destinations, ACL, rétention | F013/F017 canaris |
| R-13 | AI-Guard supposé disponible / élevé | H-11 | ADR-10 accepté côté Unleash ; contrat réel et gate externe | EXT-GUARD, F017 recette réelle |
| R-14 | GUI techniquement correcte mais austère / élevé | H-06 | Design system, tranche verticale M0 et critères de réexamen d’ADR-03 | AC-F014-009/010, UX |
| R-15 | Frontière cœur Rust / présentation ou dépendances mal maîtrisée / élevé | H-06/H-16 | Politique R2 confirmée ; catalogue d’opérations commun, WebView restreinte, inventaire natif/web | SBOM, build et tests de frontière, Systems + Security |
| R-16 | Connexion docs trop large / élevé | H-09/H-10 | Scope/Pin, consentements et limites affichés | F013 OAuth/ACL, Security |
| R-17 | Perte lors compaction / élevé | H-13 | Sources conservées, provenance et évaluation | F003/F012 Context Engineer |
| R-18 | SQLite writer saturé/WAL énorme / élevé | H-02 | Transactions courtes, readers bornés, checkpoint | F002 charge, Reliability |
| R-19 | Inflation roles/skills/agents / moyen | H-14 | Discovery, consolidation et archive | F004/F007 volumétrie |
| R-20 | API ou prix changent / élevé | H-14 | Profils versionnés, `ModelCatalogSnapshot` signé avec la release et daté, validité et état périmé visibles, tarif périmé ou absent bloquant, probes explicites (Q-14) | F012 fixtures et live contract |
| R-21 | Licence ou dépendance incompatible / élevé | H-15 | Apache-2.0 décidée (Q-06, ADR-14), allow-list `cargo deny`, SBOM et notices | EXT-OSS, Systems + Security |
| R-22 | Faux sentiment conformité documentaire / élevé | Toutes | Preuves prévues distinctes de tests exécutés | Revue indépendante, readiness DOC-09/DOC-10 et DOC-ING-001 à 008 (ADR-19) |
| R-23 | Tâche unique trop large ou microgestion / élevé | H-03 | Mandats, reports et limites de staffing | F004 évals et coûts |
| R-24 | Terminal perdu après crash / élevé | — (ADR-05, process-host accepté) | Process-host et niveaux de reprise | F011/F018 crash PTY |
| R-25 | Rétention empêche reprise/audit / élevé | H-13 | Politique Q-12 : journal métier sans échéance, objets requis jamais purgés, manifests et tombstones, sauvegardes quotidiennes et hebdomadaires vérifiées | F018 restore/purge |
| R-26 | Sélecteur encore valide mais mauvais résultat métier / critique | H-18/H-21 ; INV-059/060/062/064 | Identité métier avant effet, cardinalité, pré/postconditions et oracle indépendant ; un log ou continue du superviseur n'efface pas l'assertion contraire | T-F019-009/010 : même CSS, mauvais SKU/compte/quantité, Workflow + QA |
| R-27 | Script libre contourne l'autorisation d'effet sur un domaine pourtant autorisé / critique | H-18 ; INV-062 | Profil médié de référence, contrôle réel d'actions/descendants, secrets séparés, limites script déclarées ; refus de garantie si confinement/médiation indisponibles | T-F019-008/014, mutation même domaine et canal navigateur hostile, Security + Runtime |
| R-28 | Appel ou coût LLM caché dans un mode annoncé zéro modèle / critique | H-17/H-21 ; INV-059/064 | forbidden transitive avant chargement/probe, routeur et dépendances qualifiés, rapports mécaniques ; conception séparée mandatée et coûts mission visibles | T-F019-001/006/015, canaris texte/vision/embedding/local et compteur indépendant, API + QA |
| R-29 | Localhost utilisé comme preuve de localité ou OOM traité par fallback cloud / critique | H-20 ; INV-063 | Qualifier tuple et placement effectif ; local_only conservée, max_inflight et ressources distinctes, inconnues après annulation conservées ; aucun téléchargement/reroutage implicite | F012 REQ-MODEL-013 à 018 et campagne locale : proxy cloud, alias modifié, OOM, TLS/auth, API + Systems + Security |
| R-30 | Simplicité guidée masque autorités, coûts ou défauts de template / élevé | H-19/H-21 ; INV-058/061/064/065 | Templates versionnés, aperçu concret, préparation idempotente de rôles dans mandat, modes/coûts visibles et accès expert/TUI complet ; aucune permission déduite du défaut proposé | Premier parcours sans agent, template inadapté, répétition après déconnexion et parité GUI/TUI, Product + UX + QA |

## Politique de traitement

Un risque est clos seulement avec décision et preuve, ou accepté explicitement par l’autorité compétente avec périmètre. Une recommandation d’expert ne ferme pas une décision utilisateur ouverte ; les décisions tranchées par délégation en R7 sont acceptées et révisables par un ADR de supersession. Les risques critiques de données, autorité et reprise bloquent le jalon qui les expose.

Une probabilité jugée faible ne dispense pas de tester un cutpoint reproduisible. La V1 peut rendre certains résultats incertains visibles et demander un arbitrage ; elle ne peut appeler cela « reprise automatique garantie ». Les tests de charge doivent mesurer limites réelles avant toute recommandation de Redis ou distribution.

R-26 à R-30 raccordent les [hypothèses H-17 à H-21](../brief/assumptions.md) et [invariants INV-058 à INV-065](../brief/invariants.md) aux gates M0/M2/M3/M5/M7. Les 19 features restent dans la recette ; traiter ces risques ne permet pas de supprimer les scénarios antérieurs. Ni une documentation fournisseur, ni une simulation, ni un jugement LLM supplémentaire ne clôture une garantie runtime non testée.

