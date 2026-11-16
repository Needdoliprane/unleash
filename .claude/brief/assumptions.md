# Hypothèses proposées et falsifiables

Ces hypothèses guident la conception sans modifier les invariants. Toute validation future produit une décision datée avec son auteur et sa preuve. Depuis la [révision R7](revision-2026-11-16-r7.md), H-01 à H-21 restent des hypothèses réfutables : H-03 est amendée ; H-06 et H-16 deviennent « choix retenu, réfutable par le spike ». Les sources de capacités externes sont dans [research](../research/sources.md).

| ID | Hypothèse de travail | Pourquoi | Comment la réfuter / conséquence |
| --- | --- | --- | --- |
| H-01 | Une seule autorité daemon par identité utilisateur OS sur la machine, indépendante du profil/data_root ; plusieurs projets | Un verrou de rendez-vous canonique et un seul ledger rendent le plafond global contrôlable | Plusieurs racines ne créent pas plusieurs schedulers ; changement offline après neutralisation, mécanisme OS à qualifier |
| H-02 | SQLite état transactionnel + journal + outbox, pas event sourcing intégral | Migration et requêtes locales plus simples | Tests de replay/récupération insuffisants : renforcer projections sans rejouer effets |
| H-03 | Une session durable par agent logique, au plus une session primaire ouverte ; la séparation entre missions est portée par le context_manifest de chaque invocation, sélectionné depuis le mandat et les sources de la mission avec leurs politiques héritées ; aucune tête de contexte durable par mission en V1 | Évite le mélange inter-projets sans multiplier les sessions ni les têtes concurrentes | Évaluations montrent contamination : ouvrir une session liée par mandat ou mission (nouvelle session, filiation et politiques conservées), sans réutiliser un résumé hors de sa politique |
| H-04 | Invocation = tranche active bornée comportant éventuellement plusieurs appels API et outils | Compte toute activité décisionnelle réelle | Boucle monopolise : checkpoint/limite d’appels/temps, sans dépassement de slots |
| H-05 | Run workflow immuable jusqu’à terminaison complète | Résout ambiguïté de migration | Besoin d’adoption en milieu de mission : CancelRun puis successeur explicite (Q-03) ; aucune migration en place |
| H-06 | Choix retenu, réfutable par le spike : Tauri 2 et le renderer React/TypeScript retenus par ADR-03 satisfont design, accessibilité, IME, charge et frontière WebView sans déplacer le métier | Hôte Rust et HTML/CSS autorisés ; composants de présentation réutilisés ; types générés depuis les DTO Rust | Défaut éliminatoire mesuré en M0, non corrigeable par remplacement de composant et intrinsèque à la WebView ou à Tauri : rouvrir ADR-03 avec egui/eframe ou Iced, sans retirer de fonction |
| H-07 | Les contrats Unleash restent un langage fermé de prédicats sur preuves | Exécution contrôlable, sans plugins arbitraires cachés | Besoin nouveau contrôle : ajout Rust versionné, testé, soumis politique |
| H-08 | Analyseurs externes LSP peuvent enrichir l’index multilangage | Ne pas réimplémenter tous compilateurs | Analyseur absent : recherche/snapshots/syntaxe à confiance réduite, pas fausse référence résolue |
| H-09 | Lecture documentaire et polling local suffisent au Watch de base | Pas endpoint public obligatoire sur laptop | Délai trop grand : webhook optionnel avec relais explicitement configuré |
| H-10 | Notion peut être connecté localement par token d’intégration utilisateur | Aucun secret client global distribué | Décidé pour la V1 (Q-09). Si un workspace interdit les intégrations internes : token remis par le propriétaire ; si Notion publie un OAuth public sans secret : méthode ajoutée, jamais de retrait du token |
| H-11 | Mode `standard` par défaut : broker local et scopes des outils typés, sans confinement OS du code lancé sous le même UID ; mode `strict` : ajoute ce code au modèle de menace et exige un confinement OS prouvé (Q-05) | Distinction honnête des frontières de sécurité | Isolation non qualifiée en mode `strict`, ou exigée par la politique d’une action : refus avant exécution (`IsolationUnavailable`), jamais de repli silencieux vers `standard` |
| H-12 | Permissions sont intersection des autorités, jamais simple dernier override | Empêche extension par rôle/skill | Besoin délégation : mandat signé/révision limité, émis par autorité supérieure |
| H-13 | Les résumés de contexte sont des dérivés versionnés, pas vérité source | Permet reconstruction et détection de pertes | Évaluation révèle omission : invalider résumé, réduire taille mission |
| H-14 | Projet ciblé par budgets et quotas, managers et workers partagent mêmes comptes | Empêche coûts cachés du management | Quotas distincts : routage par pools explicites sans masquer budget global |
| H-15 | L’open source n’implique ni télémétrie obligatoire ni AI-Guard propriétaire obligatoire | Conserve autonomie d’exploitation | Décision économique contraire : arbitrage explicite, ne pas appeler cela déjà accepté |
| H-16 | Choix retenu, réfutable par le spike : Ratatui avec Crossterm permet une TUI Rust à parité fonctionnelle complète | Pile retenue par ADR-15 ; bibliothèques de rendu et d’entrée terminal réutilisées ; même contrat backend que la GUI | Qualification clavier, Unicode, accessibilité, restauration ou charge échoue : adapter le backend terminal ou comparer une autre bibliothèque sans réduire la parité |

## Hypothèses quantitatives

Les objectifs de latence, taille de queues, débit et rétention sont des critères de qualification proposés. Ils ne sont pas des résultats mesurés sur un produit inexistant. Les tests doivent enregistrer machine, filesystem, charge, build et profil d’API. Un Mac Studio puissant ne dispense pas de mesurer une machine de référence minimale.

Les modèles listés officiellement ne sont pas présumés accessibles par les clés de Louis. Un appel de capacité et une requête minimale dans le compte réel sont nécessaires avant activation. Aucun test payant n’a été lancé pendant cette rédaction.

## Hypothèses ajoutées R3–R5

| ID | Proposition | Vérification / alternative |
| --- | --- | --- |
| H-17 | AgentInvocation, ReplayWorker et ProviderProbe partagent le plafond global 5/50, avec sous-compteurs et ressources navigateur distincts | Charge mixte et starvation ; alternative sous-pools sous un plafond total commun, jamais 50+50 cachés |
| H-18 | Automatisation médiée et plan fermé servent de référence ; script utilisateur exportable avec niveau de garantie déclaré | Tests de contournement même domaine, dérive et reprise ; alternative script accompagné sous autorité explicite |
| H-19 | Le mode guidé propose templates et création idempotente des objets ; le mode expert montre les mêmes objets | Parcours novice sans jargon/agent, reprise d’onboarding, parité TUI ; alternative configuration détaillée toujours accessible |
| H-20 | Les API locales sont des transports du même ModelRouter, avec politique de placement et ressources explicite | Tester proxy localhost vers cloud, saturation et capacités ; alternative autre profil qualifié sans fuite de données |
| H-21 | La supervision par blocs réutilise task leader_decision et choice explicites entre opérations action | Vérifier nombre d’appels borné par checkpoints, limites des logs et transitions permises ; alternative replay sans modèle ou décision humaine |

### Précision H-17 — R6

Le pool global compte également provider_probe actif, afin que la qualification initiale d’un fournisseur sans agent ne crée pas une exemption au plafond. Les trois propriétaires possibles sont invocation agente, worker mécanique et probe de configuration. La portée confirmée 5 par défaut /50 maximum reste inchangée ; cette allocation mécanique, sans exemption, est décidée par Q-15 ([ADR-13](../architecture/adr/ADR-13-scheduler-budget.md), accepté R7) ; seules les valeurs par défaut restent à mesurer en recette.
