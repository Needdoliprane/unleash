# F012 — exigences

Les IDs sont stables ; une modification de sens crée une révision documentaire et indique le critère affecté. Statut de toutes les lignes : comportement normatif proposé pour satisfaire les invariants confirmés du brief.

| Exigence | Règle vérifiable | Critère |
| --- | --- | --- |
| REQ-MODEL-001 | Les appels utilisent les API des fournisseurs : clés utilisateur pour OpenAI/Anthropic et les services cloud qui les exigent ; authentification propre au backend local, éventuellement sans secret sur loopback qualifié. Aucun état ni session d’abonnement CLI. | AC-F012-001 |
| REQ-MODEL-002 | Résoudre chaque paramètre par la priorité publiée, tout en intersectant permissions et limites. | AC-F012-002 |
| REQ-MODEL-003 | Rejeter les demandes incompatibles avec le modèle et qualifier chaque dialecte compatible. | AC-F012-003 |
| REQ-MODEL-004 | Finaliser et valider les arguments d’outil avant toute action. | AC-F012-004 |
| REQ-MODEL-005 | Classifier refus, troncature et schéma invalide séparément du succès métier. | AC-F012-005 |
| REQ-MODEL-006 | Réserver atomiquement avant réseau et conserver coût inconnu. | AC-F012-006 |
| REQ-MODEL-007 | Limiter essais et délais, distinguer échec certain et issue inconnue. | AC-F012-007 |
| REQ-MODEL-008 | Appliquer fallback seulement si capacités, données et budgets l’autorisent. | AC-F012-008 |
| REQ-MODEL-009 | Persister un manifest de contexte effectif, des versions et exclusions. | AC-F012-009 |
| REQ-MODEL-010 | Reprise et changement fournisseur s’appuient sur mémoire durable Unleash. | AC-F012-010 |
| REQ-MODEL-011 | Distinguer tokens absents, cache, reasoning et coût tarifé. | AC-F012-011 |
| REQ-MODEL-012 | Activer un profil après qualification et contrôle d’accès réel du modèle ; vérifier la clé/token lorsque requis, sans inventer une exigence de clé en local ni présumer un droit à partir de la documentation. | AC-F012-012 |
| REQ-MODEL-013 | Inclure LM Studio, Ollama et llama.cpp comme profils natifs V1, dialectes épinglés, auth adaptée et exception HTTP limitée au loopback qualifié ; conserver les moteurs externes et le cœur Rust. | AC-F012-013 |
| REQ-MODEL-014 | Qualifier les capacités par tuple serveur/build, modèle, template, quantification et configuration ; distinguer déclaration, observation et probe, refuser les capacités manquantes ou périmées avant effet. | AC-F012-014 |
| REQ-MODEL-015 | Appliquer `data_locality = local_only ou approved_remote` au placement réel et au fallback ; localhost ne prouve pas la localité, et une destination inconnue ou distante interdite ne reçoit aucun contexte. | AC-F012-015 |
| REQ-MODEL-016 | Ajouter admission et quotas locaux aux plafonds globaux 5/50 ; distinguer file, chargement froid/chaud, génération, manque de mémoire, annulation et issue inconnue, avec deadlines bornées. | AC-F012-016 |
| REQ-MODEL-017 | Conserver dans les manifests les identités modèle/serveur et métadonnées disponibles, dont digest/template/quantification ; ne fabriquer ni identité des poids ni usage/coût absent. | AC-F012-017 |
| REQ-MODEL-018 | Offrir même configuration, sélection par portée et inspection GUI/TUI ; appliquer `AutomationRun.model_policy=forbidden` aux modèles locaux et empêcher appel, chargement, préchauffage ou démarrage modèle pour ce run. | AC-F012-018 |

Les permissions, versions et résultats incertains sont des données explicites. Aucun critère ne peut être considéré satisfait uniquement parce qu’un modèle l’affirme. Chaque exigence est reliée à son test et à la preuve attendue dans [tests](tests.md).

REQ-MODEL-013 à 018 sont ajoutées pour les modèles locaux. La révision de 001/012 précise l’authentification sans supprimer les voies cloud. Les trois backends ne sont pas déclarés intégrés par la seule présence de ces exigences ; voir [spécification](../../specs/local-models.md) et [qualification prévue](../../testing/local-model-tests.md).

