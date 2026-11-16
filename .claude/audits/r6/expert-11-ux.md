# R6 — Expert 11 : UX / GUI / TUI

**Statut : vérifié sur documentation, 16 novembre 2026.** Aucun produit, test utilisateur ou rendu n’a été exécuté. Revue des documents demandés et des contrats F004/F019 associés. Les dix espaces, la parité riche, le backend commun et le départ sans agent/dépôt/modèle sont correctement posés. Les corrections R6 de clôture humaine, de probe fournisseur et de registre d’opérations ne sont pas recomptées. Q-21 reste optionnelle.

## R6-E11-01 — Retrouver un setup sauvegardé avant toute création de travail

**Priorité moyenne ; raccord de lecture incomplet.** [UI-OVERVIEW](../../design/gui-screen-specifications.md) affiche les drafts guidés ; [AC-F014-015](../../features/F014-gui/acceptance.md) et [AC-F015-017](../../features/F015-cli-tui/acceptance.md) exigent leur reprise. Cependant, les quatre lectures [F004](../../features/F004-hierarchical-orchestration/interfaces.md) comprennent un accès par référence exacte et une liste de templates, sans préciser comment découvrir les drafts sauvegardés. Le [catalogue](../../architecture/client-operation-catalog.md) cite « draft » dans Overview, sans contrat d’énumération, notamment avant la création du projet.

**Scénario.** Une personne sauvegarde ses sources et paramètres dans la GUI, ferme celle-ci puis ouvre une TUI neuve. Aucun goal/run n’existe avant Apply. Elle ne connaît pas le draft_id et ne peut s’appuyer sur l’historique local de cet autre client. Le stockage durable ne suffit pas à garantir un chemin de reprise découvrable.

**Solution proposée.** Préciser dans GetOverview une collection autorisée et paginée de préparations, y compris hors projet encore créé : référence, libellé, révision sauvegardée, date, template et lien de reprise. La TUI propose la même sélection sans copier d’ID. **Alternative :** une requête commune explicite de recherche des drafts ; mettre alors à jour le décompte des lectures et la parité.

**AC / limites.** Sauvegarder en GUI puis retrouver/ouvrir en TUI sans cache local ni agent, dépôt ou modèle ; retrouver les mêmes valeurs et diagnostics. Vérifier ACL et noms masqués. Un brouillon local jamais soumis peut rester propre au client : le finding concerne seulement les drafts sauvegardés. Raccorder PAR-ONBOARDING-01 et T-F014-015/T-F015-017.

## R6-E11-02 — Distinguer reprise et nouvelle application d’un setup

**Priorité moyenne ; ambiguïté d’état/action.** [F004/interfaces](../../features/F004-hierarchical-orchestration/interfaces.md) et le [catalogue](../../architecture/client-operation-catalog.md) garantissent qu’un même command_id/payload restitue les identités préparées. Ils ne fixent pas le résultat de deux ApplyGuidedSetup de la même révision avec des command_id différents, ni la représentation d’une préparation déjà appliquée dans [le parcours guidé](../../design/guided-automation-experience.md).

**Scénario.** La GUI prépare une mission et reçoit son résultat. La même préparation est ensuite ouverte en TUI ; une action « Appliquer » crée légitimement une nouvelle commande de transport. Selon l’interprétation, le backend réutilise le travail ou en crée un second. Je ne peux pas confirmer une duplication effective ; l’oracle documentaire manque, au-delà du retry idempotent déjà couvert.

**Solution proposée.** Définir qu’une révision appliquée référence durablement son résultat ; une nouvelle application identique retrouve ce résultat ou renvoie un refus typé avec son lien. Afficher « Ouvrir le travail préparé », ses étapes restantes et le démarrage distinct. Pour préparer un autre travail, créer explicitement un nouveau draft. **Alternative :** applications multiples autorisées, mais distinguées par une identité persistante et une action utilisateur explicite.

**AC / limites.** Deux clients, deux command_id, même draft/révision : résultat déterministe et compréhensible, aucune création supplémentaire involontaire. Reconnexion après commit retrouve l’application ; les anciennes références restent inspectables. Le choix ne doit lancer ni StartRun ni modèle et ne demande aucune confirmation répétée. Compléter T-F004-012 et T-F015-003/017.

## R6-E11-03 — Raccorder « corriger le CSV » à une interaction intégrée

**Priorité moyenne ; parcours de correction insuffisamment spécifié.** [L’expérience guidée](../../design/guided-automation-experience.md) demande de corriger un CSV et de préserver les données initiales. [PAR-AUTOMATION-03](../../design/gui-tui-parity.md) décrit tableau, diagnostics et éditeur de mapping. [F019/interfaces](../../features/F019-reusable-automations/interfaces.md) expose import/validation et une révision immuable ; [le design terminal](../../design/cli-tui-design.md) rend l’éditeur externe facultatif. Le passage d’une erreur de valeur à une nouvelle source corrigée reste indéterminé.

**Scénario.** Une quantité négative viole le schéma, tandis que parsing et mapping sont corrects. Corriger le mapping ne résout rien. Un novice doit savoir où saisir la bonne quantité, ce qui sera conservé et quelle révision sera exécutée.

**Solution proposée.** Décrire une copie de travail bornée, l’édition locale de cellule et le diff, puis ImportAutomationData et validation complète d’une nouvelle révision. Préserver provenance, identités métier et révision initiale. **Alternative :** formulaire intégré par ligne/champ erroné, sans imposer un tableur complet.

**AC / limites.** Même correction au clavier en GUI/TUI, sans éditeur externe obligatoire ni LLM ; nouvelle révision visible, original et run engagé inchangés, zéro effet avant validation. Annulation conserve la source initiale. Raccorder T-F019-005 et T-F014-015/T-F015-017 ; ne pas assimiler correction des entrées et réconciliation d’effets déjà inconnus.
