# Exigences F002

| ID | Exigence atomique proposée | AC | Test | Preuve |
|---|---|---|---|---|
| REQ-PERSIST-001 | État, événements, outbox, receipt et réservations d'une même décision sont committés dans une seule transaction locale, sans I/O externe lente. | AC-F002-001 | T-F002-001 | EV-F002-001 |
| REQ-PERSIST-002 | Toute base active utilise un profil local WAL/durabilité qualifié et un build SQLite dont version/source_id et correctifs sont contrôlés. | AC-F002-002 | T-F002-002 | EV-F002-002 |
| REQ-PERSIST-003 | Les événements sont livrés au moins une fois ; chaque consommateur interne déduplique son application par inbox transactionnelle. | AC-F002-003 | T-F002-003 | EV-F002-003 |
| REQ-PERSIST-004 | Chaque commande épingle authority_generation dans son digest ; acteur/command_id, contrôle de lignée avant lookup et tombstone de clé conservé empêchent une seconde mutation après réponse perdue, restore, purge ou payload divergent. | AC-F002-004 | T-F002-004 | EV-F002-004 |
| REQ-PERSIST-005 | Migrations et versions de payload sont explicites ; schéma trop récent ou migration interrompue ne sont jamais ouverts en écriture ordinaire. | AC-F002-005 | T-F002-005 | EV-F002-005 |
| REQ-PERSIST-006 | Une référence d'artefact est publiée après préparation durable sous pin ; le writer exclut références/pins live et suppression deleting, avec intention avant unlink et tombstone après synchronisation ; les reprises conservent cette exclusion. | AC-F002-006 | T-F002-006 | EV-F002-006 |
| REQ-PERSIST-007 | Contention, disque plein, I/O error et corruption produisent des erreurs distinctes et ne génèrent aucun faux receipt de succès. | AC-F002-007 | T-F002-007 | EV-F002-007 |
| REQ-PERSIST-008 | Réservations de capacité et de budgets global/projet/fournisseur sont atomiques ; un coût inconnu garde sa réserve. | AC-F002-008 | T-F002-008 | EV-F002-008 |
| REQ-PERSIST-009 | Chaque propriété possède authority_generation et epoch/fence monotones dans cette lignée ; une autre lignée ou un ancien propriétaire sont refusés. | AC-F002-009 | T-F002-009 | EV-F002-009 |
| REQ-PERSIST-010 | Une sauvegarde protège snapshot/objets/effets par barrière et pins avant copie ; restore neutralise les anciens exécuteurs, importe le delta postérieur et ses blocages, puis crée nouvelle lignée/secrets/flux sans adopter leurs hosts ni leurs commandes anciennes. | AC-F002-010 | T-F002-010 | EV-F002-010 |

Les numéros de version ne remplacent pas la provenance d'un correctif rétroporté. Les garanties de durabilité supposent un filesystem et un matériel respectant leurs primitives de synchronisation ; leur qualification figure dans les preuves d'environnement.
