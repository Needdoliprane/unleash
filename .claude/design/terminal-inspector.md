# Inspecteur de terminaux et processus

Le registre montre process_id, process_run_uuid, propriétaire logique/session, projet/worktree, cwd, argv affichable, ports connus, état observé, logs et politique de reprise. Le PID est une information de diagnostic, pas l’identité de commande Stop.

## Lecture et saisie

L’utilisateur peut lire stdout/stderr et l’historique PTY selon permissions. La saisie interactive exige une capability d’écriture et un propriétaire actif ; toute entrée est attribuée. Le terminal peut être réservé à un seul écrivain à la fois avec lease, tout en autorisant plusieurs lecteurs.

Un processus pertinent peut être découvert par un autre agent autorisé, qui demande réutilisation au lieu de le relancer. Les labels de service incluent projet/worktree/configuration et ports ; le nom « frontend » seul ne déduplique pas deux services différents.

## Reconnexion

Un process-host conserve PTY et spool quand le daemon redémarre, si le host a survécu. La GUI ne promet aucune reconnexion PTY après reboot. Si le host ou le processus est perdu, l’écran présente logs persistés, état incertain et options de relance selon contrat.

## Arrêt et incidents

Arrêter cible la génération exacte, vérifie l’identité et le mandat, demande arrêt puis observe. Un port occupé par un processus inconnu n’autorise pas de le tuer. Une commande réussie mais receipt absent demande réconciliation. Une sortie brute peut être redacted ; le badge le précise.

Tests : PID réutilisé, daemon crash, host crash, resize PTY, lecteur lent, logs continus et entrée pendant révocation. Référence F011.

