---
name: armadai-rules
description: "Règles du dépôt ArmadAI et leçons mesurées : barrière, tests et mutations, Git et PR, environnement, rapport. À charger avant d'écrire, tester, relire ou commiter du code dans ce dépôt, et avant de rédiger un brief ou un rapport."
metadata:
  armadai-slot: code-standards
---
# Règles du dépôt ArmadAI

Chaque ligne : la règle, sa raison, sa source. Aucune règle ici n'a d'autre source que celle citée ; une règle à ajouter passe par `framer` et Dimitri.

Sources abrégées : `CLAUDE.md` (racine du dépôt), `.claude/CLAUDE.md` (règles de l'équipe), `mémoire:<fichier>` (mémoire du coordinateur, `feedback_*.md`, hors dépôt), `plan dogfood` (faits mesurés en session que le plan `docs/superpowers/plans/2026-10-23-dogfood-team.md`, tâche 1, étape 1, énumère), `brief coordinateur` (règles du dépôt que le coordinateur joint à chaque brief, `repo-rules.md`, 2026-10-22/23, hors dépôt).

## Barrière

- Avant tout push : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model` ; c'est la configuration unique de la CI. Source : `CLAUDE.md`, Build & Test Commands ; `.claude/CLAUDE.md`, Barrière avant push.
- `frozen/armadai-audit/` est hors workspace : ne pas l'étendre, seulement le garder compilable (`cd frozen/armadai-audit && cargo check --all-targets && cargo test`) quand un crate dont il dépend change. Source : `CLAUDE.md`, Architecture.
- Lire le code retour de chaque commande de la barrière ; jamais de `tail` (ou autre tube) qui masque un échec. Raison : le code retour d'un tube est celui de sa dernière commande. Source : plan dogfood ; brief coordinateur.
- `armadai-core` et `armadai-model` doivent compiler seuls ; `armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars). Source : `CLAUDE.md`, Build & Test Commands et Where things live.

## Tests et mutations

- `crates/armadai` est un crate binaire seul : `cargo test --lib` y rend « 0 passed » sans erreur ; utiliser `--bin armadai` ou `--test <nom>`, toujours `--no-fail-fast` ; `--exact` exige le chemin complet du test. Source : `CLAUDE.md`, Non-obvious facts ; `.claude/CLAUDE.md`, Pièges de mesure.
- `armadai_core::test_support::env_lock()` n'est pas réentrant : deux gardes sur un thread bloquent sans message. Source : `CLAUDE.md`, Non-obvious facts ; `.claude/CLAUDE.md`, Pièges de mesure.
- Pour chaque test ajouté ou changé : casser le code qu'il protège, constater le rouge, restaurer, reporter la mutation avec la sortie vue. Raison : des tests incapables d'échouer ont été trouvés à répétition, un test vert sans mutation ne prouve rien. Source : `.claude/CLAUDE.md`, Mesure, ne raisonne pas ; `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Pour chaque test, nommer la mutation qu'il attraperait ; si aucune ne vient, c'est un défaut. Raison : huit revues distinctes ont trouvé un test incapable d'échouer. Source : `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Un `contains` sur un fragment est suspect : quelle autre sortie contiendrait cette chaîne ? Si elle existe, comparer des lignes entières. Une boucle de test sur des sites est suspecte : vérifier qu'elle couvre chaque site. Une capacité ajoutée se teste par le cas où le signal doit apparaître, pas seulement par son absence (`== None`). Source : `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Un échec de mutation rapporté par l'auteur du correctif se fait rejouer par un tiers. Source : `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Après avoir fermé une classe de défaut, vérifier l'appelant et la couche au-dessus (une vague de correctifs réintroduit sa propre classe un étage plus haut) ; quand c'est possible, supprimer la primitive tentante plutôt que corriger son mauvais usage. Source : `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Fixtures à valeurs distinctes deux à deux, assertions à égalité exacte plutôt qu'un `contains` qu'autre chose satisferait. Raison : une permutation ou une valeur par défaut des deux côtés passe sinon. Source : `mémoire:feedback_tests_qui_ne_peuvent_pas_echouer.md`.
- Après une mutation, restaurer le fichier avec une mtime neuve (`touch`), jamais `cp -p`/`copy2`. Raison : cargo garde la rlib mutée et les mutations se composent en silence. Source : `mémoire:feedback_harnais_de_mutation_qui_ment.md`.
- Toute valeur de référence d'un test (instant de départ, variable d'environnement, cwd, fichier partagé) se lit sous le verrou qui la protège. La vérification n'est jamais un seul run vert : répéter 5 à 8 passes. Raison : vert seul, rouge en suite complète (3 occurrences), révélé seulement par la répétition. Source : `mémoire:feedback_lire_sous_le_verrou.md`.
- Une borne de temps se choisit par rapport au seuil du défaut, pas au temps nominal, et la stabilité se vérifie par répétition (5 passes). Raison : 3 flakes sous charge parallèle. Source : `mémoire:feedback_bornes_temporelles_dans_les_tests.md`.
- Un correctif proposé par une revue est une hypothèse : le muter avant de l'adopter ; quand auteur et relecteur se contredisent, la mesure tranche. Source : `.claude/CLAUDE.md`, Mesure, ne raisonne pas ; `mémoire:feedback_le_remede_d_une_revue_est_une_hypothese.md`.

## Git et PR

- Conventional Commits (`feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`), un seul type par sujet. Source : `CLAUDE.md`, Git Conventions ; `.claude/CLAUDE.md`, Commits.
- Code, commentaires et messages de commit en anglais ; corps de PR et échanges avec Dimitri en français. Source : `CLAUDE.md`, Language ; `.claude/CLAUDE.md`, Commits.
- Le trailer `Co-Authored-By` reprend la ligne d'attribution fournie par la session ; aucun nom de modèle en dur. Source : `.claude/CLAUDE.md`, Commits.
- Branches : `master` seul tronc, `feature/*` (ou branche de travail) et `release/*` ; pas de `develop`. Source : `CLAUDE.md`, Git Conventions.
- Ouvrir la PR et s'arrêter ; fusion en squash sur `master` seulement si le brief de Dimitri l'autorise, sur CI verte (6 vérifications) et après une revue indépendante. Raison : des PR à CI verte portaient des régressions que seule la revue indépendante a trouvées. Source : `CLAUDE.md`, Git Conventions ; `.claude/CLAUDE.md`, Ouvre la PR ; `mémoire:feedback_independent_review.md`.
- Avant une fusion : vérifier les PR Dependabot ouvertes et que les 6 vérifications de la CI passent (fmt, clippy, test, build, conventional commits, audit). Source : `CLAUDE.md`, Git Conventions.
- Après un clone, activer les hooks : `git config core.hooksPath .githooks` (le hook `commit-msg` impose les Conventional Commits). Source : `CLAUDE.md`, Git Conventions.
- L'auteur d'un changement n'est jamais son relecteur ; le relecteur lit le diff en lecture seule. Source : `mémoire:feedback_independent_review.md`.

## Environnement

- Aucune tâche en arrière-plan (`&`, `run_in_background`, `nohup`) ; attendre chaque commande au premier plan. Source : `.claude/CLAUDE.md`, Règles communes.
- Écrire `/usr/bin/grep`, jamais `grep` : dans le shell des agents de Claude Code, `grep` est une fonction du snapshot de shell du harnais (`type grep` : « grep is a shell function from …/.claude/shell-snapshots/snapshot-zsh-….sh »), qui réécrit les appels avant de les passer à `command grep`. Dans le zsh interactif de Dimitri, `grep` est `/usr/bin/grep` : le détournement vient du snapshot. Mesure du 2026-10-26 par `lead` (`type grep`, `type ls`). Source : Dimitri, 2026-10-26 (« Oui, les deux (Recommandé) »).
- Écrire `/bin/ls`, jamais `ls` : dans le shell que voient les agents, `ls` est un alias de `eza --color=auto` (`type ls`, mesuré le 2026-10-26 par `lead`), et hors terminal interactif `eza --color=auto -a` se bloque. Mesure du 2026-10-26 : un agent a lancé `ls -a` dans deux commandes shell, `eza` est resté bloqué environ 2 h 10 (`ps` : deux `zsh -c` avec leur enfant `eza`). Source : Dimitri, 2026-10-26 (« Oui, ajouter la règle (Recommandé) »).
- Si le harnais a mis une de tes commandes en arrière-plan d'office (au bout de 120 s), vérifier avec `/bin/ps` (jamais `ps`, alias de `procs` dans ce shell, `type ps`) avant de rendre la main qu'aucun de tes processus ne reste, et le rapporter ; ne jamais affirmer que rien ne tourne sans cette vérification. Mesure du 2026-10-26 : l'agent du cas `eza` a rendu la main en affirmant que rien ne tournait, alors que les deux processus bloqués vivaient encore. Source : Dimitri, 2026-10-26 (« Oui, ajouter la règle (Recommandé) »).
- Un target cargo privé par worktree (`CARGO_TARGET_DIR=<worktree>/target`). Raison : deux copies d'un même crate dans un `CARGO_TARGET_DIR` partagé faussent les tests. Source : plan dogfood ; brief coordinateur.
- Jamais `git stash`. Raison : le stash est partagé avec le dépôt principal. Source : plan dogfood (la règle) ; brief coordinateur (la raison).
- Pour tout essai du binaire `armadai`, isoler `HOME` et `ARMADAI_CONFIG_DIR` dans un dossier temporaire, pour qu'il n'écrive ni dans `~/.config/armadai` ni dans `~/.claude` (la mémoire du coordinateur vit sous `~/.claude/projects` et n'est pas concernée). Source : brief coordinateur.
- rust-analyzer est peu fiable ici : tout vérifier au compilateur. Source : `.claude/CLAUDE.md`, Règles communes.
- `graphify-out/` et `.graphifyignore` sont locaux et ne sont jamais commités ni poussés. Source : `mémoire:feedback_graphify_usage.md`.

## Rapport

- Ne présenter comme vérifié que ce qui a été vu dans une sortie réelle. Source : `.claude/CLAUDE.md`, Règles communes ; plan dogfood.
- Après avoir corrigé une erreur de compilation, relancer la même commande avant d'affirmer quoi que ce soit sur l'état. Raison : cargo s'arrête au premier crate en échec. Source : `mémoire:feedback_relayer_un_fait_perime.md`.
- Un fait tiré d'un rapport, d'une issue ou de la mémoire se revérifie au dépôt (`grep`, `git log -S`, `git merge-base`) avant d'entrer dans un brief, ou s'attribue explicitement (« le rapport dit X, à vérifier »). Source : `mémoire:feedback_relayer_un_fait_perime.md`.
- Rapporter honnêtement : si le périmètre est plus gros que prévu, livrer le reste en entier et dire précisément ce qui est laissé et pourquoi ; un point du brief jugé faux se dit, avec la mesure. Source : `.claude/CLAUDE.md`, Règles communes.
