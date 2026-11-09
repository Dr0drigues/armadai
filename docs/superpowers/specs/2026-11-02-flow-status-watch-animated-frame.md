# Cadre du lot « watch animé » (mis à jour le 2026-11-09)

Étape `frame`, reprise après la pause du 2026-11-02. Rédigé sur la spec `2026-11-02-flow-status-watch-animated-design.md` (commit `d481f48`), branche `feat/flow-watch-animated`, master `e263db5`. Rien n'est enregistré dans `armadai-rules` : aucune règle nouvelle n'est décidée ; deux propositions sont en §5, à valider. Les décisions de Dimitri sont recopiées dans la spec (« Décisions de Dimitri ») et ne sont pas rediscutées ici.

## 1. Base de la PR

La PR va vers `master` (#538 est fusionnée, `e263db5`). La décision du 2026-11-02 « Vers feat/graph-improvements » est caduque (la spec le dit).

## 2. Socle disponible (lu au dépôt, worktree `armadai-watch`)

- `crates/armadai/src/ascii_graph.rs` : `flow_to_ascii(&FlowGraph) -> String`, pure, sans état d'exécution (la spec en décrit les libellés et préfixes ; le lot y ajoute une variante avec préfixe par étape).
- `flow_status/runs.rs` : `Run`, `StepView`, `Mark::sign()` (table ✓ ● ○ ? ✗ ~ -), `Mark::name()`.
- `flow_status/line.rs:18-27` : `line()` choisit `rfind(|r| !r.finished && !r.interrupted)`, **puis `.or(runs.last())`**. Le watch ne dessine que l'exécution ouverte : la règle partagée doit rendre `Option` (ouverte ou rien) ; le repli sur la dernière reste dans `line()`. À traiter au plan, sans changer la sortie de `--line`.
- `cli/flow.rs:180-201` : boucle `--watch` actuelle (1 s, `\x1b[2J\x1b[H` à chaque redessin via `anstream::print!`). `--json`, `--format md` et le cas sans `--watch` retournent avant la boucle : ils sont déjà hors de portée du lot.
- `--color` et `painted()` (table Mark -> style) : à réutiliser.
- `libc = "0.2"` est déclarée dans `armadai-core` sous `[target.'cfg(unix)'.dependencies]` (`crates/armadai-core/Cargo.toml:23-24`) ; **absente** de `crates/armadai/Cargo.toml` (lu). Le workspace n'a pas de table `[lints]` : aucune interdiction d'`unsafe` au niveau du dépôt.
- Non revérifié dans cette session (pas de shell) : le contenu de `Cargo.lock` (le cadre du 2026-11-02 mesurait `libc 0.2.191` au lock, dépendance directe d'`armadai-core`, et l'absence de `ctrlc`, `signal-hook`, `terminal_size`, `crossterm`, `ratatui`). À remesurer au plan par `cargo tree -i libc` et un diff du lock.
- Non mesuré non plus : que `anstream` retire `\x1b[2J\x1b[H` hors terminal en `--color auto` (la spec l'affirme) ; le lot ne dépend plus de cette retenue puisqu'il n'émet plus ces séquences hors terminal.

## 3. Règles qui s'appliquent (source : `armadai-rules`, sauf mention)

- Barrière : fmt, clippy `-D warnings`, `cargo test --no-fail-fast`, builds `armadai-core` et `armadai-model` ; CI `ubuntu-latest` seul, `RUSTFLAGS=-D warnings` (`.github/workflows/ci.yml`). Lire le code retour, jamais de `tail`.
- `crates/armadai` est binaire seul : `--bin armadai` ou `--test <nom>`. `env_lock()` non réentrant.
- Chaque test ajouté : mutation vue rouge, mutation nommée ; égalité de lignes entières, pas de `contains` ; fixtures à valeurs distinctes ; borne de temps relative au seuil du défaut, 5 passes (critère 6) ; valeurs de référence lues sous le verrou.
- Après une mutation : `touch` pour restaurer (jamais `cp -p`), puis `cargo build` avant tout essai au binaire (Dimitri, 2026-11-05) ; l'essai du binaire isole `HOME` et `ARMADAI_CONFIG_DIR`.
- Sortie machine versionnée (Dimitri, 2026-11-05) : le lot ne touche ni `--json` ni son champ de version ; critère 8 le vérifie.
- Clé de compilation opt-in (Dimitri, 2026-11-05) : sans objet, aucune clé de `project.yaml` ni écriture de `link` n'est ajoutée ; critère 9.
- Texte de toute ligne nouvelle de sortie (aide de `--watch`, ligne de doc, libellés) : proposé par le plan, validé par Dimitri à la relecture du plan (Dimitri, 2026-10-27).
- Pas de code de diagnostic nouveau ; manifeste inchangé.
- Rien de privé dans le dépôt public : pas de chemin absolu de machine dans le plan, la doc ni les fixtures (Dimitri, 2026-11-03).
- Environnement : target cargo privé par worktree ; pas de `git stash` ; `/usr/bin/grep`, `/bin/ls`, `/bin/ps` en chemin complet ; aucune tâche en arrière-plan ; vérifier `/bin/ps` avant de rendre la main.
- Git : Conventional Commits, code et commits en anglais, PR en français ; PR ouverte, pas de fusion sans autorisation du brief.
- `frozen/armadai-audit` : non concerné (aucun crate dont il dépend ne change).
- `armadai-model` reste pur (aucune dépendance ajoutée) ; `armadai-core` inchangé.

## 4. Contraintes propres au lot (décisions de Dimitri, dans la spec)

Citées avec leur source, pour que le plan les reprenne telles quelles.

- Terminal (Dimitri, 2026-11-02, « IsTerminal seul (Recommandé) ») : `std::io::IsTerminal` sur stdout décide l'animation ; la couleur reste à `anstream` et `--color` ; `NO_COLOR` retire la couleur pas l'animation. ANSI à la main, aucun crate de terminal (« ANSI à la main (Recommandé) »).
- Signal (Dimitri, 2026-11-02, cadre, « libc + handler SIGINT ») : `libc` déclarée dans `crates/armadai` sous `[target.'cfg(unix)'.dependencies]`, aucun paquet nouveau au lock, aucun crate de signal. `unsafe` dans un seul module restreint, chaque bloc commenté de son invariant, gestionnaire async-signal-safe seulement (spec §3). Le code retour de Ctrl-C est celui de `master` : à mesurer avant tout changement et à reporter au plan.
- Taille d'écran (Dimitri, 2026-11-02, « Variables puis ioctl (Recommandé) ») : `COLUMNS`/`LINES`, puis `ioctl(TIOCGWINSZ)`, puis un défaut fixe (valeur au plan).
- Tests hors terminal (Dimitri, 2026-11-02, « Pur + hors terminal (Recommandé) ») : fonctions pures en unitaire ou golden, un test `assert_cmd` hors terminal ; la boucle en terminal et Ctrl-C à l'essai réel, pas de pty.
- Découpe (Dimitri, 2026-11-02, « Animation d'abord (Recommandé) ») : le résumé d'activité par étape n'est pas dans ce lot. Surface : « --watch seul (Recommandé) » ; Claude Code seul ; exécutions : « Une dessinée + liste (Recommandé) ».
- Dimitri, 2026-11-05 (lot interface, Q3, « Ce lot d'abord (Recommandé) ») : le watch reprend la table de marques de L1 ; le critère « sans `--watch`, inchangé » devient « inchangé hors des nouveaux drapeaux ».
- Dimitri, 2026-11-09 : exécution dessinée = « La dernière ouverte (Recommandé) » (règle de `--line`, partagée, pas recopiée) ; repère = « ● + repère tournant (Recommandé) » (`| / - \`, cellule réservée).

## 5. Règles durables proposées (rien d'écrit dans `armadai-rules`)

Si Dimitri les accepte, `framer` les enregistre avec la source « Dimitri, AAAA-MM-JJ », puis `lead` relance `armadai link --target claude`.

- P1. `unsafe` : confiné à un seul module par fonctionnalité, chaque bloc commenté de son invariant ; un gestionnaire de signal ne fait que de l'async-signal-safe. Aujourd'hui seulement une contrainte de la spec du lot (§3).
- P2. Dépendance : une dépendance déjà au lock mais non déclarée par le crate qui l'utilise s'ajoute sans paquet nouveau ; tout paquet nouveau au lock passe par Dimitri. La décision « libc + handler SIGINT » (2026-11-02) en est le seul cas.
- Écartée : « aucune séquence d'échappement de positionnement hors terminal ». Contrainte de ce lot (spec §4.2) ; à promouvoir seulement si un lot suivant la rencontre (observée, pas décidée).

## 6. Contradictions

- Entre `armadai-rules`, `CLAUDE.md` et `.claude/CLAUDE.md` : aucune.
- Dans la spec : l'objectif 3 dit « hors terminal, la sortie est celle d'aujourd'hui », alors que §4.2 et le critère 3 retirent aussi les séquences de positionnement émises aujourd'hui sous `--color always` (anstream ne les retire qu'en `auto`). Voir Q1.

## 7. Valeurs que le plan fixe et fait valider (aucune question à Dimitri avant)

Pas d'animation (200 à 250 ms proposés), taille par défaut (80x24 proposé), seuil du repli « trop grand », noms des fonctions pures, texte de l'aide de `--watch`, ligne du wiki et du `CHANGELOG`, code retour de Ctrl-C mesuré sur `master`. Risque de plateforme : la CI ne compile que sous Linux ; les types de `TIOCGWINSZ` diffèrent entre Linux et macOS (essai réel sur macOS seulement).

## Réponses de Dimitri (2026-11-09, mot pour mot, relayées par `lead`)

- Q1 : « Jamais de positionnement (Recommandé) » : aucune séquence de positionnement ni d'effacement hors terminal, même en `--color always` ; le critère 3 reste tel que rédigé, la tension de §6 est levée.
- Q2 : « Animer quand même (Recommandé) » : hors `cfg(unix)`, animation sous `IsTerminal`, sans curseur masqué ni gestionnaire ; limite écrite dans la doc.
- Q3 : « Les deux (Recommandé) » : P1 et P2 de §5 sont enregistrées dans `armadai-rules` (section Barrière), source « Dimitri, 2026-11-09 ». `lead` relance `armadai link --target claude`.

Plus aucune question ouverte : le plan peut commencer. Ce qui suit est l'historique des questions posées.

## Questions pour Dimitri (posées, réponses ci-dessus)

**Q1. Hors terminal avec `--color always` (bloque le plan : tâche de la boucle, critères 3 et 8).** Aujourd'hui `--watch --color always | cat` laisse passer `\x1b[2J\x1b[H`. La spec les supprime hors terminal ; l'objectif 3 dit pourtant « la sortie est celle d'aujourd'hui ».
- (a) Ne jamais émettre de séquence de positionnement hors terminal, même en `always` (Recommandé : critère 3 tel que rédigé ; `--color` ne règle que la couleur ; un consommateur redirigé reçoit des images séparées par rien de destructeur).
- (b) Garder le comportement actuel sous `always` (aucune différence hors terminal ; critère 3 à réécrire).
- Une autre réponse.

**Q2. Windows et hors `cfg(unix)` (bloque le plan : tâche de la boucle ; non mesurable ici).** La spec laisse l'animation active mais sans curseur masqué ni gestionnaire.
- (a) Animer quand même sous `IsTerminal`, sans curseur masqué (Recommandé : conforme à la spec ; les terminaux Windows récents lisent l'ANSI ; limite écrite dans la doc).
- (b) Hors unix, garder le watch actuel (liste, polling 1 s) : zéro risque, une branche de plus.
- Une autre réponse.

**Q3. Règles P1 et P2 de §5 (ne bloque pas le plan).** Les enregistrer dans `armadai-rules` ?
- (a) P1 et P2 (Recommandé : elles dirigeront les revues de ce lot et du suivant).
- (b) P1 seule.
- (c) Aucune : elles restent des contraintes de ce lot.
- Une autre réponse.

Rien d'autre ne manque au cadre : la spec tranche déjà le reste, et §7 reste au plan.
