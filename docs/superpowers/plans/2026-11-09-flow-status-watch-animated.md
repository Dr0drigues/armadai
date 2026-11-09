# Plan : `flow status --watch` animé (ASCII)

Spec : `docs/superpowers/specs/2026-11-02-flow-status-watch-animated-design.md` (commit `d481f48`). Cadre : `docs/superpowers/specs/2026-11-02-flow-status-watch-animated-frame.md` (commit `ec1439c`). Worktree `armadai-watch`, branche `feat/flow-watch-animated`, partie de `master` `e263db5`. La PR ira vers `master`.

Cinq tâches, toutes du domaine `cli-specialist`, à exécuter dans l'ordre : chaque tâche passe par une exécution du flow `task`. Le lot ne touche ni `armadai-model`, ni `armadai-core`, ni `link`.

## 1. Contraintes globales (valent pour chaque tâche)

- Règles `armadai-rules` (P1 et P2 comprises, Dimitri, 2026-11-09). La barrière complète passe à chaque commit : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`. Lire le code retour de chaque commande, sans tube.
- Le crate `crates/armadai` est un binaire seul : lancer les tests avec `--bin armadai` ou `--test flow_status_cli`, toujours avec `--no-fail-fast`. Utiliser `CARGO_TARGET_DIR=<worktree>/target`.
- Pour chaque test ajouté : appliquer la mutation nommée dans la tâche, constater le rouge, restaurer avec `touch`, puis lancer `cargo build` avant tout essai au binaire. Reporter la sortie vue. Comparer des lignes entières ou des sorties entières (`assert_eq!`), pas des `contains`, et utiliser des fixtures dont les valeurs sont distinctes deux à deux.
- Rien de redéfini : la table de marques reste `Mark::sign()`, le calcul des exécutions reste `runs()`, le dessin reste celui de `flow_to_ascii`. La règle « dernière exécution ouverte » est partagée avec `--line` (tâche 1), la table Mark -> style reste celle de `painted()`.
- Sorties inchangées : `flow status` sans `--watch`, `--list`, `--json`, `--line`, `--format md` et `armadai graph --format ascii` ne changent pas d'un octet. Les golden existants (`tests/flow_status_cli.rs`, `ascii_graph.rs`, `render.rs`, `line.rs`) restent tels quels. **Un golden existant modifié est un défaut**, sauf mention contraire dans la tâche.
- `unsafe` : seulement dans `crates/armadai/src/cli/term.rs` (tâche 4), et chaque bloc porte un commentaire `// SAFETY:` qui énonce son invariant. Le gestionnaire de signal n'appelle que `write`, `signal` et `raise`, qui sont async-signal-safe.
- Dépendances : `libc = "0.2"` s'ajoute sous `[target.'cfg(unix)'.dependencies]` de `crates/armadai/Cargo.toml`, et rien d'autre. `Cargo.lock` ne gagne aucun `[[package]]`.
- Aucune séquence de positionnement ni d'effacement (`\x1b[H`, `\x1b[2J`, `\x1b[J`, `\x1b[K`, `\x1b[?25l/h`) n'est écrite hors terminal, même avec `--color always` (Dimitri, 2026-11-09, Q1 du cadre).
- Hors `cfg(unix)`, l'animation reste active sous `IsTerminal`, sans curseur masqué et sans gestionnaire de signal (Dimitri, 2026-11-09, Q2 du cadre). Ce code n'est compilé ni ici ni par la CI (Linux seul) : il doit rester minimal.
- Les textes nouveaux sont ceux du §9, une fois validés par Dimitri. Aucun autre texte de sortie n'est ajouté.
- Les tests restent purs et hors terminal (Dimitri, 2026-11-02, « Pur + hors terminal (Recommandé) »). Pas de pty : la boucle en terminal et Ctrl-C se vérifient à l'essai réel (§10).
- `rust-analyzer` n'est pas fiable dans ce dépôt : vérifier au compilateur, en particulier l'API d'`anstream` 1.0 et les types de `libc` sous macOS.

## 2. Structure

```mermaid
flowchart LR
  flowrs["cli/flow.rs<br/>execute, View, animate"] -->|"open_run(runs)"| line["flow_status/line.rs"]
  flowrs -->|"frame(project, runs, session, tick, size)"| watch["flow_status/watch.rs<br/>(nouveau, pur)"]
  flowrs -->|"size(), HiddenCursor"| term["cli/term.rs<br/>(nouveau, seul unsafe)"]
  watch -->|"open_run"| line
  watch -->|"flow_to_ascii_marked"| ascii["ascii_graph.rs"]
  watch -->|"render, run_heading, run_body"| render["flow_status/render.rs"]
  watch -->|"Mark::sign"| runs["flow_status/runs.rs"]
  watch -->|"flow_graph(id, flow)"| model["armadai-model"]
  term -->|"ioctl, signal, raise, write"| libc["libc (cfg unix)"]
  graphcli["armadai graph --format ascii"] -->|"flow_to_ascii (inchangé)"| ascii
```

Seuls `watch.rs` et `term.rs` sont nouveaux. Les deux modules sont séparés pour une raison : tout ce que l'image contient est pur et se teste sans terminal (`watch.rs`) ; tout ce qui touche au terminal et au signal est confiné, avec son `unsafe`, dans `term.rs`. `cli/flow.rs` assemble les deux et n'écrit les séquences de positionnement que si stdout est un terminal.

## 3. La boucle `--watch`

```mermaid
flowchart TD
  A(["flow status --watch"]) --> B{"stdout.is_terminal()"}
  B -->|"non"| P["boucle actuelle, sans séquence :<br/>print_painted(view.text()) si changé, pas de 1 s"]
  B -->|"oui"| C["HiddenCursor::new()<br/>écrit ESC[?25l, pose on_sigint"]
  C --> D["View::log() + metadata<br/>(session au plus 1 fois/s, taille du journal)"]
  D --> E{"session ou taille<br/>du journal changée ?"}
  E -->|"oui"| F["View::snapshot()<br/>read_log + runs, mis en cache"]
  E -->|"non"| G{"une étape Running, ou<br/>taille d'écran changée ?"}
  F --> H
  G -->|"non"| S["sleep(PACE)"]
  G -->|"oui"| H["term::size()<br/>COLUMNS/LINES, ioctl, 80x24"]
  H --> I["watch::frame(project, runs, session, tick, size)<br/>dessin + ligne de compte, sinon liste bornée"]
  I --> Q{"image différente<br/>de la précédente ?"}
  Q -->|"non"| S
  Q -->|"oui"| J["peinture des marques ; ESC[2J à la 1re image ;<br/>ESC[H, chaque ligne + ESC[K, ESC[J ; write_all + flush"]
  J --> S
  S -->|"tick += 1"| D
  J -.->|"erreur d'écriture : ?"| K["Drop HiddenCursor :<br/>ESC[?25h, SIGINT remis à SIG_DFL<br/>seulement si on_sigint était posé"]
  X(["Ctrl-C"]) -.-> L["on_sigint (posé seulement si SIGINT était SIG_DFL) :<br/>write(ESC[?25h), signal(SIGINT, SIG_DFL), raise(SIGINT)"]
  L -.-> M(["mort par SIGINT, même code qu'avant le lot"])
```

Le journal n'est relu (`snapshot`) que si sa taille ou la session change ; le reste du temps, le pas ne fait qu'un `stat`. On calcule une image seulement quand le journal change, quand une étape tourne ou quand la taille d'écran change. On ne l'écrit que si elle diffère de la précédente (tâche 6). Sans étape `Running`, l'image reste fixe et aucun octet n'est écrit. L'image tient toujours dans `lignes − 1` (tâche 6). Si SIGINT est ignoré au lancement, le gestionnaire n'est pas posé (tâche 6). Un Ctrl-C ne passe jamais par `Drop` : le gestionnaire restaure le curseur lui-même, puis laisse le signal tuer le processus comme aujourd'hui. Le pas `PACE` vaut 250 ms, au repos comme pendant qu'une étape tourne (Q2, décidée).

## 4. Tâche 1 : partager la règle « dernière exécution ouverte » (`cli-specialist`)

**Fichiers** : `crates/armadai/src/flow_status/line.rs`.

**Interface nouvelle**
```rust
/// The run a live view follows: the last one still open (neither finished nor interrupted).
pub fn open_run(runs: &[Run]) -> Option<&Run>
```
`line()` devient `open_run(runs).or(runs.last())`. Le repli sur la dernière exécution reste dans `line()`, ce qui laisse la sortie de `--line` inchangée (écart relevé par `framer`).

**Étapes**
- [ ] Avant tout changement, vérifier que `git diff --stat e263db5 HEAD -- crates/` est vide : le code du worktree est alors celui de `master`.
- [ ] Ajouter `open_run`, puis faire passer `line()` par elle.
- [ ] Ajouter les tests, appliquer les mutations, lancer la barrière.

**Tests** (dans `line.rs`)
- `open_run_is_none_without_an_open_run` : une exécution finie et une interrompue donnent `None`. Mutation : ajouter `.or(runs.last())` dans `open_run` → rouge.
- `open_run_is_the_last_open_one` : avec `[open "one", open "two", finished "three"]`, la fonction rend `"two"`. Mutation : `find` au lieu de `rfind` → rouge.
- Les tests actuels de `line()` restent inchangés. Mutation : retirer `.or(runs.last())` de `line()` → `without_an_open_run_the_last_run_is_shown` passe au rouge.

**Commit** : `refactor(flow): share the last open run between --line and --watch`

## 5. Tâche 2 : le dessin avec un préfixe et un suffixe par étape (`cli-specialist`)

**Fichiers** : `crates/armadai/src/ascii_graph.rs`.

**Interface nouvelle**
```rust
/// `flow_to_ascii` with each node's label written `{before}{label}{after}`,
/// `(before, after) = wrap(step id)`; the label keeps its `▶ ` / ` ■`.
pub fn flow_to_ascii_marked(g: &armadai_model::FlowGraph, wrap: &dyn Fn(&str) -> (String, String)) -> String

pub fn flow_to_ascii(g: &armadai_model::FlowGraph) -> String {
    flow_to_ascii_marked(g, &|_| (String::new(), String::new()))
}
```
Rien d'autre ne change dans `ascii_graph.rs`.

**Étapes**
- [ ] Déplacer le corps de `flow_to_ascii` dans `flow_to_ascii_marked`, en appliquant `wrap(&n.id)` autour du libellé déjà préfixé par `▶ ` ou suffixé par ` ■`.
- [ ] **Mesure à reporter** : `ascii-dag` garde-t-il les espaces en fin de libellé (`"✓ design  "`) ? Le golden ci-dessous le montre. S'il les retire, la cellule du repère n'est plus réservée et le critère 1 tombe. Dans ce cas, s'arrêter et le rendre à `lead`, sans choisir de remplaçant (U+00A0 est une piste) : ce choix change ce que Dimitri voit.
- [ ] Ajouter le test, appliquer les mutations, lancer la barrière.

**Tests**
- `a_marked_drawing_wraps_each_label` : `pipeline` (`a`, `b`, `c`) avec `wrap` = (`"✓ "`, `"  "`) pour `a`, (`"● "`, `" |"`) pour `b`, (`"○ "`, `"  "`) pour `c`. On compare la sortie entière à un golden, relu ligne à ligne : mêmes boîtes et mêmes flèches que `PIPELINE`, libellés élargis. Mutations : appliquer `wrap` à l'index du nœud voisin → rouge ; oublier `after` → rouge.
- Les golden `every_step_is_named` et `an_edge_to_an_unknown_step_is_listed`, inchangés, protègent `armadai graph --format ascii` octet pour octet (critère 8). Mutation : faire rendre `(" ", "")` par défaut dans `flow_to_ascii` → rouge.

**Commit** : `refactor(graph): let flow_to_ascii wrap the label of each step`

## 6. Tâche 3 : l'image pure du watch (`cli-specialist`)

**Fichiers** : `crates/armadai/src/flow_status/watch.rs` (nouveau), `flow_status/mod.rs` (`pub mod watch;`) et `flow_status/render.rs` (extraction, sans changement de sortie).

**Extraction dans `render.rs`** (les golden actuels de `render` gardent la sortie)
```rust
pub(super) fn ordered(runs: &[Run]) -> Vec<&Run>       // already there, made visible
pub(super) fn run_heading(run: &Run) -> String         // "{flow}  {status_word}"
pub(super) fn run_body(run: &Run) -> String            // the step lines, `a → b : n`, the transitions, each ending in '\n'
```
`render()` devient `header + Σ "\n" + run_heading + "\n" + run_body`.

**Interface de `watch.rs`**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size { pub cols: usize, pub rows: usize }

/// The spinner of the running step: `| / - \`, one per tick.
pub fn spinner(tick: u64) -> char

/// The image of `--watch` in a terminal, as plain text (no colour, no escape).
/// The last open run of a known flow is drawn (its heading, the drawing,
/// then its body), and the other runs are listed under it as `render` lists them.
/// Without such a run, or when the drawn part does not fit `size`, the
/// image is `render(runs, session)`.
pub fn frame(project: &armadai_model::Project, runs: &[Run], session: &str, tick: u64, size: Size) -> String

/// Whether `text` (the session header included) fits: no line wider than
/// `cols` (counted in chars), at most `rows - 1` lines (the last row keeps the cursor, so nothing scrolls).
fn fits(text: &str, size: Size) -> bool
```

**Forme de l'image** (Q1 décidée : le bloc complet suit le dessin) :
```
session <id> (UTC)

<flow>  in progress
<flow_to_ascii_marked sans sa première ligne `flow: <id>`>

<run_body(run)>                      ← bloc complet : étapes, liens, transitions (Q1)

<run_heading(autre)>
<run_body(autre)>                    ← chaque autre exécution, dans l'ordre de `ordered`
```
- Exécution dessinée : `line::open_run(runs)`, à condition que `project.flows.get(&run.flow)` existe. Une exécution `a | b` n'a pas de flow connu et reste donc en liste.
- `wrap(step)` : le préfixe vaut `"{sign} "` si l'étape a un `StepView`, sinon deux espaces (aucun état inventé, largeur constante). Le suffixe vaut `" {spinner(tick)}"` sur une étape `Mark::Running`, sinon deux espaces.
- On applique `fits` au texte complet de la partie dessinée, en-tête de session compris. Les autres exécutions listées dessous ne comptent pas (spec §4.1). Le risque de hauteur est à regarder à l'essai réel.
- La fonction reste `#[allow(dead_code)] // wired by task 4` tant que la tâche 4 ne l'appelle pas. La tâche 4 retire cette ligne.

**Tests** (dans `watch.rs`, projet construit en mémoire comme `ascii_graph::tests::template`, flow `deliver` de `flow_status_cli.rs` : `design`, `build`, `guard`, `validate`, avec le retour `guard → build`)
- `spinner_turns` : `spinner(0..5)` donne `| / - \ |`. Mutation : rendre `'|'` en constante → rouge.
- `a_running_run_is_drawn_with_its_marks` (critères 1 et 2) : on compare l'image entière à un golden pour `design ✓`, `build ✓`, `guard ●`, `validate ○`, `tick 0`, taille 200×100. Le golden est relu contre `armadai graph --format ascii deliver` (mêmes boîtes, mêmes arêtes, `guard → build (rejected)` sous `links not drawn:`). Mutations : permuter les signes `✓` et `○` → rouge ; supprimer le préfixe → rouge ; changer `Mark::Pending.sign()` → rouge, ce qui montre que l'image suit la table.
- `two_ticks_differ_only_by_the_spinner` (critère 1) : on compare `frame(tick 0)` à `frame(tick 1)`. Même nombre de lignes, mêmes largeurs, un seul caractère différent (`|` puis `/`), situé sur la ligne de la boîte `guard`. Mutations : repère constant → rouge ; repère sur toutes les boîtes → rouge (plusieurs différences) ; suffixe vide sur les étapes qui ne tournent pas : rouge dans `a_running_run_is_drawn_with_its_marks` et `every_mark_reads_in_plain_text`, **pas** ici (les deux ticks ont les mêmes largeurs). Un second cas, `fan-out` (deux boîtes sur une ligne), vérifie que seule la boîte qui tourne change.
- `every_mark_reads_in_plain_text` (critère 2) : un flow de sept étapes, une par `Mark`. Chaque ligne de boîte, comparée en entier, commence par `[{sign} `. Mutation : préfixe pris dans une table locale au lieu de `Mark::sign()`, avec une valeur différente → rouge.
- `the_last_open_run_is_drawn_and_the_others_listed` (critère 7) : `[open pipeline, open deliver, finished hotfix, interrupted bugfix]`, comparé à un golden entier. On attend `deliver` dessiné, et `pipeline`, `hotfix`, `bugfix` en blocs `render`. Mutations : `find` au lieu de `rfind` dans `open_run` → rouge ; oublier les autres exécutions → rouge.
- `a_group_or_no_open_run_is_the_list` (critère 7) : `frame` vaut `render(runs, session)` pour un run `deliver | ship` ouvert, puis pour des runs tous finis. Mutation : dessiner `runs.last()` → rouge.
- `a_screen_too_small_gives_the_list` (critère 7) : `cols = largeur − 1` donne `render(...)` ; `cols = largeur` avec `rows = hauteur + 1` donne le dessin ; `rows = hauteur` donne `render(...)`. Mutations : supprimer `fits` → rouge ; `<` au lieu de `<=` → rouge ; oublier le `− 1` des lignes → rouge.
- `a_cycle_flow_names_every_link` (critère 7) : `plan-build-verify` en cours. L'image contient la ligne `links not drawn:` et la ligne entière `  check → build (failed)`, sans panique. Mutation : retirer la liste des liens non dessinés dans `draw` → rouge.

**Commit** : `feat(flow): build the frame of the animated watch`

## 7. Tâche 4 : la boucle en terminal, le curseur, le signal, la taille (`cli-specialist`)

**Fichiers** : `crates/armadai/Cargo.toml`, `crates/armadai/src/cli/term.rs` (nouveau, le seul `unsafe` du lot), `cli/mod.rs` (déclaration), `cli/flow.rs`, `tests/flow_status_cli.rs`.

**Étape 1, mesures avant tout changement de la boucle** (à reporter dans le rapport de tâche, puis dans ce plan au §9)
- [ ] Écrire d'abord le test `ctrl_c_outside_a_terminal_still_kills_by_sigint` (voir plus bas) et le lancer **sur la boucle actuelle**. Vert attendu : `ExitStatusExt::signal() == Some(2)`, ce qui est la mesure du code retour hors terminal sur le code de `master`. Reporter la sortie.
- [ ] `cargo tree -i libc -e normal`, avant et après l'ajout de `libc` à `crates/armadai/Cargo.toml`. Puis `git diff Cargo.lock` : seule la liste `dependencies` du paquet `armadai` doit gagner `"libc"`, sans `[[package]]` nouveau. Si un paquet apparaît, s'arrêter et le rendre à `lead` (P2).

**Interface de `cli/term.rs`**
```rust
/// The terminal size: `COLUMNS`/`LINES`, else the size `ioctl(TIOCGWINSZ)` gives for stdout, else 80x24, axis by axis.
pub fn size() -> Size
/// Pure: one axis from the env value, then the ioctl value; "0", empty or not a number is skipped.
fn pick(env: Option<&str>, ioctl: Option<u16>, default: usize) -> usize
/// Hides the cursor while alive. On unix it also catches SIGINT to show the cursor before dying of the signal.
pub struct HiddenCursor;
impl HiddenCursor { pub fn new() -> std::io::Result<Self> }   // ESC[?25l, then on unix signal(SIGINT, on_sigint)
impl Drop for HiddenCursor { /* ESC[?25h; on unix signal(SIGINT, SIG_DFL) */ }
#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int)  // write(STDOUT_FILENO, ESC[?25h), signal(SIGINT, SIG_DFL), raise(SIGINT); nothing else
```
- `ioctl` : `libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws)`, **sans aucun `as` sur la requête**. Le crate `libc` donne à chaque plateforme le type de requête que son `ioctl` attend (`c_ulong` sous macOS et glibc). Un `as` déclencherait `clippy::unnecessary_cast` sur l'une des deux, et la CI ne compile que Linux. La tâche est vérifiée par la barrière sous macOS (machine locale) et sous Linux (CI). Un retour `!= 0` ou une valeur 0 font passer au défaut, jamais à une erreur.
- Pose du gestionnaire : `libc::signal(libc::SIGINT, on_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t)`. Vérifier au compilateur et à clippy sous les deux plateformes.
- Hors `cfg(unix)` : `size()` lit `COLUMNS`/`LINES` puis prend 80×24 ; `HiddenCursor` n'écrit rien et ne pose rien.

**Changements dans `cli/flow.rs`**
- Après `if !watch { … }` : si `std::io::stdout().is_terminal()`, appeler `view.animate()`. Sinon, garder la boucle actuelle **sans** la ligne `anstream::print!("\x1b[2J\x1b[H")`, avec un pas de 1 s et `print_painted(&view.text())`. La sortie hors terminal reste ainsi la liste d'aujourd'hui (objectif 3), sans aucune séquence de positionnement.
- `View::animate(&self) -> anyhow::Result<()>` suit la boucle du §3 : cache `(session, longueur, snapshot)`, `tick` augmenté à chaque image dessinée tant qu'une étape tourne, `PACE` = 250 ms partout, au repos compris (un `stat`, ni relecture ni redessin : Q2), et image vide `NO_ACTIVITY` comme `text()`.
- Écriture d'une image : construire tout le tampon, puis `std::io::stdout().lock()`, `write_all` et `flush` d'un seul coup, avec `?`. Une erreur d'écriture sort de la boucle, et `Drop` restaure le curseur. **Piège** : `anstream` en mode `auto` sans couleur (`NO_COLOR`) retire toutes les séquences, positionnement compris. Les séquences de positionnement s'écrivent donc sur `std::io::stdout()` brut, et seul le contenu passe par la décision de couleur : `anstream::AutoStream::choice(&std::io::stdout())`, puis `anstream::adapter::strip_str` si la couleur est coupée. Vérifier ces deux noms dans la source d'`anstream` 1.0, qui est au registre local.
- Peinture : `print_painted` est découpée en `paint_line(&str) -> String`, sans changement de sortie pour `flow status`. On y ajoute `paint_boxes(&str) -> String`, appelée seulement par `animate`, qui peint le signe placé juste après un `[` avec le style de `painted(&[(Piece::Mark(m), sign)])`. Le `Mark` se retrouve à partir du signe en parcourant les sept variantes : aucune table nouvelle.
- Supprimer le `#[allow(dead_code)]` de la tâche 3.
- `--help` de `--watch` : texte du §9.

**Tests**
- `term::tests::size_is_env_then_ioctl_then_default` : on vérifie `pick(Some("132"), Some(100), 80) == 132`, `pick(Some("0"), Some(100), 80) == 100`, `pick(Some("x"), None, 80) == 80`, `pick(None, Some(0), 24) == 24`. Mutations : ioctl avant l'environnement → rouge ; accepter 0 → rouge.
- `flow::tests::the_mark_after_a_box_bracket_is_painted` : on compare la sortie de `paint_boxes("  [✓ a (x)  ]   [● b (y) |]")` à la chaîne attendue, construite avec `paint(style::ok(), "✓")` et `paint(style::running(), "●")`. Mutations : styles de `Done` et `Running` permutés → rouge ; peindre un signe qui ne suit pas un `[` → rouge (ajouter `[○ c ✓  ]`, le `✓` du libellé doit rester brut). Peindre le repère `-` ne change rien : le style de `NotRun` est vide, la mutation ne peut pas rougir.
- `tests/flow_status_cli.rs`, `watch_outside_a_terminal_is_the_list_once_without_escape` (critère 3) : lancer `flow status --watch` sur le journal `looping` avec `std::process::Command` (stdout en tube, `HOME` et `ARMADAI_CONFIG_DIR` isolés, `NO_COLOR=1`). Un fil lit stdout jusqu'à avoir `LOOPING.len()` octets, avec une borne de 20 s comptée sur le démarrage lent sous charge, pas sur le temps nominal. Attendre ensuite 2,5 s (plus de deux pas de 1 s), tuer le processus et lire le reste. La sortie entière doit valoir `LOOPING`. Mutations : réintroduire `print!("\x1b[2J\x1b[H")` → rouge ; redessiner à chaque pas → rouge (deux images).
- Le même test avec `--color always` : après retrait des séquences SGR (`\x1b[` … `m`), on attend exactement `LOOPING`, aucun autre `\x1b`, et au moins une séquence SGR dans la sortie brute (sans quoi le test serait vide). Mutation : la ligne `\x1b[2J\x1b[H` d'aujourd'hui, qui sort sur `master` en `always` → rouge.
- `#[cfg(unix)] ctrl_c_outside_a_terminal_still_kills_by_sigint` (critère 4, partie mesurable sans pty) : lancer, attendre la première image comme ci-dessus, envoyer `/bin/kill -INT <pid>` par `std::process::Command` (pas d'`unsafe` dans les tests), puis vérifier `status.signal() == Some(2)`. Ce test est vert sur `master` (étape 1). **Mesure du gestionnaire sans pty**, temporaire et non commitée : poser `HiddenCursor` aussi hors terminal. Le test doit rester vert, ce qui montre que le gestionnaire rend bien la mort par le signal. Remplacer ensuite `raise(SIGINT)` par `libc::_exit(0)` : rouge attendu. Restaurer les deux et reporter les deux sorties.
- Répéter 5 fois les trois tests d'intégration (règle des bornes de temps).

**Commit** : `feat(flow): animate flow status --watch in a terminal`

## 8. Tâche 5 : documentation (`cli-specialist`)

**Fichiers** : `docs/wiki/flow-status.md`, `CHANGELOG.md`.

- [ ] Remplacer la ligne `--watch` du bloc de « Reading the output » et ajouter le paragraphe du §9 sous ce bloc, avec une image copiée **telle quelle** du golden `a_running_run_is_drawn_with_its_marks` (tâche 3), sans la réécrire à la main.
- [ ] Ajouter aux « Limits » la ligne Windows du §9.
- [ ] Ajouter la ligne du `CHANGELOG` du §9 sous `## Unreleased` / `### Added`.
- [ ] Vérifier qu'aucun chemin absolu de machine n'apparaît.

**Tests** : aucun (documentation). Le relecteur compare chaque texte au §9 validé.

**Commit** : `docs(flow): document the animated watch`

## 8 bis. Tâche 6 : correctifs après le rejet de `review` (`cli-specialist`)

Ce que la tâche part de corriger : la tête `94c0554` et les deux lignes de rejet de `reviewer`.
- **R1** : les autres exécutions sont ajoutées sous le dessin sans borne de hauteur (`watch.rs:69-72`), et l'image est réécrite toutes les 250 ms tant qu'une étape tourne (`animate`, `changed || running`). Mesures de `reviewer` : 58 lignes dans un pty de 30 lignes avec 5 exécutions, 87 exécutions et 685 lignes sur la session de ce dépôt, 9 images identiques en 2,1 s en écran étroit.
- **R2** : `HiddenCursor::new` pose `on_sigint` sans regarder la disposition en place. Si le parent ignore SIGINT, `master` survit à un Ctrl-C alors que la branche meurt du signal 2 (critère 4). Le commentaire `shortcut:` est faux.

Les décisions de Dimitri sont citées dans la section « Décisions de Dimitri ».

**Fichiers** : `crates/armadai/src/flow_status/watch.rs`, `crates/armadai/src/cli/term.rs`, `crates/armadai/src/cli/flow.rs`, `crates/armadai/tests/flow_status_cli.rs`, `docs/wiki/flow-status.md`, `CHANGELOG.md`.

### 6.1 L'image tient dans l'écran (R1, `watch.rs`)

`frame` garde sa signature. L'image se construit par paliers, et on prend le premier qui tient (`fits`, inchangée : chaque ligne ≤ `cols` caractères, au plus `rows − 1` lignes) :

1. **Dessin.** L'en-tête de session, le titre, le dessin, une ligne vide, le bloc de l'exécution ouverte, puis la ligne de compte des autres exécutions, si elles existent. `fits` s'applique **à ce tout, ligne de compte comprise**.
2. **Liste de l'exécution suivie.** Ce palier sert quand le dessin ne tient pas, quand l'exécution n'a pas de flow connu (`a | b`) ou quand aucune exécution n'est ouverte. On affiche l'en-tête de session, puis le bloc `render` (titre et corps) de l'exécution que nomme `--line` (`open_run(runs).or(runs.last())`, la règle déjà partagée), puis la même ligne de compte pour les autres. S'il n'y a aucune exécution, on affiche `render(runs, session)` tel quel (deux lignes).
3. **Coupe** (Q4 : « Couper et le dire (Recommandé) »). Si le palier 2 ne tient pas lui non plus, ce qui arrive avec une exécution plus longue que l'écran ou un terminal très petit :
   - on garde `rows − 2` lignes ;
   - la ligne suivante, la dernière, est `+ <n> more lines: armadai flow status` (`+ 1 more line: armadai flow status` au singulier), où `<n>` compte toutes les lignes laissées de côté, ligne de compte des autres exécutions comprise ;
   - une ligne plus large que `cols` est coupée à `cols − 1` caractères, suivis de `…`, et cela s'applique aussi à la ligne de coupe ;
   - si `rows ≤ 2`, il n'y a pas de place pour la ligne de coupe : on garde `rows − 1` lignes, c'est-à-dire aucune quand `rows = 1`, et l'image est vide.

```rust
/// `+ <n> other runs: armadai flow status`, `run` when n == 1; `None` when n == 0.
fn others_line(n: usize) -> Option<String>
/// Cut `text` to `size`: at most `rows − 1` lines, the last one saying what was cut,
/// `+ <n> more lines: armadai flow status`, and every line at most `cols` chars, a cut one ending in `…`.
fn cut(text: &str, size: Size) -> String
```
Hors terminal, rien ne change : la boucle de liste écrit `view.text()` en entier, comme sur `master` (critères 3 et 8).

### 6.2 Ne jamais redessiner une image identique, suivre un redimensionnement (R1, `flow.rs`)

- `animate` garde `last: Option<String>`, l'image plate (avant peinture) écrite en dernier. Elle n'écrit que si `image != last`. Le repère tourne et change donc l'image tant que le dessin est affiché. En palier 2 ou 3, une étape qui tourne ne change rien à l'image, et rien n'est écrit.
- Suggestion de `reviewer`, **retenue** : on calcule une image aussi quand `term::size()` a changé depuis la précédente (condition `changed || running || size != last_size`). Sans cela, un écran réduit au repos garde une image trop grande et défile. Le coût est un `ioctl` par pas.
- Suggestion de `reviewer`, **retenue** : `latest_session` (un `read_dir` et un `stat` par fichier de journal) n'est relu qu'au plus une fois par seconde, comme sur `master`. Entre deux relectures, on garde la session trouvée et on ne fait que `stat` son journal à chaque pas. Avec `--session`, rien ne change. Un événement de la session suivie reste visible en 250 ms (critère 6). Une session **nouvelle** est vue en 1 s au plus, comme sur `master`.
- Suggestion de `reviewer`, **retenue** : `paint_image_line` peint les marques de toute ligne qui contient `[` suivi d'un signe de `Mark::sign()` et d'une espace, et non plus seulement d'une ligne qui commence par `[`. Une boîte peut se trouver après un trait d'une autre colonne (`│   [● c (z) |]`). Les autres lignes restent peintes par `paint_line`.

### 6.3 SIGINT ignoré reste ignoré (R2, `term.rs`)

- `HiddenCursor { caught: bool }`. `new()` lit la disposition en place **sans la changer** : `libc::sigaction(SIGINT, null, &mut old)`, avec `old` mis à zéro par `std::mem::zeroed()` dans le même bloc `unsafe`. Le gestionnaire `on_sigint` n'est posé que si `old.sa_sigaction == SIG_DFL`. Dans tous les autres cas (`SIG_IGN`, ou un échec de `sigaction`), on ne touche à rien et `caught = false` ; le curseur est quand même masqué.
- Lire avant de poser, plutôt que poser puis remettre `SIG_IGN` : cela ferme la fenêtre où un Ctrl-C tuerait un processus censé l'ignorer.
- `Drop` montre le curseur, puis remet `SIG_DFL` **seulement si `caught`**. On revient ainsi à la disposition précédente, qui était `SIG_DFL` dans ce cas.
- `on_sigint` ne change pas : il n'est posé qu'au-dessus de `SIG_DFL`, donc remettre `SIG_DFL` puis relancer le signal reproduit exactement `master`.
- Supprimer le commentaire `shortcut:` faux. Le champ `sa_sigaction` porte le même nom sous Linux et sous macOS dans `libc`, mais c'est à vérifier au compilateur sous les deux, sans `as` superflu.

### 6.4 Documentation (décisions « Déplacer » et Q5 : « Oui, textes proposés (Recommandé) »)

- `docs/wiki/flow-status.md` : déplacer le paragraphe du watch et son image après l'exemple de liste et sa table de marques. « as listed below » devient « as listed above ». Le paragraphe et la ligne des limites prennent les textes validés de la tâche 6 au §9.
- `CHANGELOG.md` : sa ligne prend le texte validé de la tâche 6 au §9.
- L'image du wiki reste copiée telle quelle du golden `a_running_run_is_drawn_with_its_marks`. Elle n'a qu'une exécution, donc pas de ligne de compte.

### Tests (chaque test avec la mutation qui doit le faire rougir)

Dans `watch.rs` (purs) :
- `the_other_runs_are_one_count_line` : `[finished hotfix, interrupted bugfix, open deliver]`, grande taille. On compare l'image entière à un golden qui se termine par `+ 2 other runs: armadai flow status`. Mutations : remettre la boucle des autres blocs → rouge ; compter l'exécution dessinée (`runs.len()`) → rouge.
- `one_other_run_is_singular_and_none_is_no_line` : avec une autre exécution, la dernière ligne vaut exactement `+ 1 other run: armadai flow status` ; sans autre exécution, la dernière ligne est celle du bloc. Mutations : pluriel toujours → rouge ; `Some` pour `n == 0` → rouge.
- `fits_counts_the_count_line` : `rows` vaut le nombre de lignes du palier 1 **avec** sa ligne de compte, plus 1 → dessin ; avec un de moins → palier 2. Mutation : tester `fits` avant d'ajouter la ligne de compte → rouge.
- `a_drawing_too_big_lists_the_followed_run_and_counts_the_others` : petite largeur, `[open deliver, finished hotfix]` dans cet ordre. On compare l'image entière (palier 2) : en-tête, bloc de `deliver`, puis `+ 1 other run: armadai flow status`. Mutations : revenir à `render(runs, session)` → rouge ; suivre `runs.last()` au lieu de `open_run(runs).or(runs.last())` → rouge (`runs.last()` est `hotfix`).
- `every_image_fits_its_screen` (critère « l'image tient dans `rows − 1` ») : 87 exécutions fabriquées, avec des marques distinctes et des transitions. Pour chacune des tailles (200×100), (60×30), (30×10), (20×3), (10×1), on vérifie : nombre de lignes ≤ `rows − 1` et chaque ligne ≤ `cols` caractères. **Pour que la boucle couvre chaque palier**, une assertion par taille nomme aussi le palier atteint, par la première ligne du dessin, la ligne de compte ou la ligne de coupe, comparées en entier : 1, 2, 3, 3, 3. Mutations : supprimer `cut` → rouge ; `<=` au lieu de `<` dans `fits` → rouge ; `rows == 1` sans garde (soustraction qui déborde) → panique, donc rouge.
- `a_run_longer_than_the_screen_is_cut_with_its_line` (palier 3) : golden entier pour (40×8), avec une ligne d'étape plus large que 40 colonnes. On attend 7 lignes, dont la dernière est `+ <n> more lines: armadai flow status` et la ligne large finit par `…` en 40e colonne. Mutations : coupe sans ligne de coupe → rouge ; une ligne de trop → rouge ; `<n>` qui oublie la ligne de compte → rouge ; ligne coupée sans `…` → rouge.
- `one_line_left_out_is_singular` : on choisit une taille qui laisse exactement une ligne de côté, et la dernière ligne vaut `+ 1 more line: armadai flow status`. Mutation : pluriel toujours → rouge.

Dans `cli/flow.rs` :
- `a_box_after_an_edge_is_painted` : on compare `paint_image_line("  │   [● c (z) |]")` à la chaîne attendue, avec `●` peint en `style::running()`. Mutation : revenir à `starts_with('[')` → rouge. Un second cas fixe le comportement sur une ligne d'étape (`"  ● guard     running"`) : elle reste peinte par `paint_line`. Mutation : passer toute ligne par `paint_boxes` → rouge.
- `an_identical_image_is_not_written` : la décision d'écrire passe par une fonction pure `fn to_write<'a>(last: Option<&str>, image: &'a str) -> Option<&'a str>`. On vérifie `None` pour deux images égales et `Some` sinon (première image comprise). Mutation : toujours `Some` → rouge.

Dans `tests/flow_status_cli.rs` :
- `#[cfg(unix)] ctrl_c_ignored_by_the_parent_stays_ignored` (critère 4) : lancer `/bin/sh -c 'trap "" INT; exec "$0" flow status --watch' <binaire>` ; un signal ignoré le reste à travers `exec`. Attendre la première image, envoyer `/bin/kill -INT <pid>`, attendre 1 s (un délai qui laisse largement mourir un processus qui ne survivrait pas), vérifier que le processus vit encore (`try_wait() == None`), puis le tuer. Hors terminal, ce test est vert sur `master` comme sur la branche.
- **Mesure du correctif sans pty**, temporaire et non commitée, comme à la tâche 4 : poser `HiddenCursor` aussi hors terminal. Sur le code de `94c0554`, le test passe au rouge (mort par le signal 2) ; avec le correctif, il passe au vert. `ctrl_c_outside_a_terminal_still_kills_by_sigint` doit rester vert avec la même mutation. Reporter les trois sorties.
- Répéter 5 fois les tests d'intégration du watch.

**Mesure de reprise par `validator`** : rejouer les mesures de `reviewer` dans son pty (30 lignes avec 5 exécutions, la session de ce dépôt, l'écran étroit). Attendu : aucune image de plus de 29 lignes, et aucune image identique écrite deux fois.

**Commit** : `fix(flow): keep the watch image on screen and an ignored SIGINT ignored`

## 9. Textes et valeurs (validés par Dimitri le 2026-11-09, Q3 : « Validés tels quels (Recommandé) »)

| Valeur | Valeur retenue | Statut | Raison |
|---|---|---|---|
| Pas en terminal (`PACE`) | 250 ms partout, au repos compris : un `stat` du journal à chaque pas, une image seulement si le journal change ou si une étape tourne | Validé (Q2, Q3) | Un tour complet du repère par seconde (4 images). Un événement s'affiche en 250 ms au plus, sous la borne de 1 s du critère 6. |
| Pas hors terminal | 1 s, inchangé | Validé (Q3) | « la sortie est celle d'aujourd'hui » (objectif 3) |
| Taille par défaut | 80 colonnes × 24 lignes | Validé (Q3) | Taille d'ouverture de Terminal.app, et largeur de la liste de `graph --view team` |
| Seuil de repli | Largeur max (en caractères) ≤ colonnes, et lignes de la partie dessinée, en-tête de session compris, ≤ lignes − 1 | Validé (Q3) | La dernière ligne garde le curseur : rien ne défile, donc `ESC[H` revient bien au début de l'image. |
| Repère | `\|` `/` `-` `\`, suffixe `" " + repère` sur l'étape `Running`, deux espaces ailleurs | Validé (spec du 2026-11-09, Q3) | Cellule réservée à toutes les boîtes |
| Étape sans marque | Préfixe de deux espaces | Validé (Q3) | Aucun état inventé, et largeur égale à `"✓ "` |
| Sous le dessin | Une ligne vide, puis le bloc complet de `render` : lignes d'étapes, liens, transitions | Validé (Q1, puis « Ajouter une ligne vide », 2026-11-09) | Aucune information ne disparaît ; la liste `links not drawn:` ne se confond pas avec les lignes d'étapes |
| Ligne de compte (tâche 6) | `+ <n> other runs: armadai flow status` ; au singulier `+ 1 other run: armadai flow status` ; aucune ligne si `n = 0` | Validé (« Ce texte (Recommandé) ») | Les autres exécutions ne font plus défiler l'écran |
| Hauteur de l'image en terminal (tâche 6) | Toujours ≤ lignes − 1 : dessin, sinon liste de l'exécution suivie, sinon coupe | Validé (« Une ligne de compte (Recommandé) », Q4 « Couper et le dire (Recommandé) ») | R1 de `reviewer` |
| Ligne de coupe (tâche 6) | `+ <n> more lines: armadai flow status` ; au singulier `+ 1 more line: armadai flow status` ; une ligne trop large est coupée à `colonnes − 1` caractères suivis de `…` | Validé (Q4) | On sait ce qui manque et où le lire |
| Image identique (tâche 6) | Jamais réécrite | Validé (« Une ligne de compte (Recommandé) ») | R1 de `reviewer` |
| Code retour de Ctrl-C | **Non mesuré à ce jour** (le plan est écrit sans shell). Attendu : mort par le signal 2, soit `$?` = 130 sous zsh. Valeur à reporter ici après la tâche 4, étape 1 (hors terminal), et après l'essai réel avec le binaire de `master` (en terminal). | Méthode validée (Q3), valeur à mesurer | Critère 4 |

**Aide de `--watch`** (validée, Q3 ; remplace « Redraw on every new event (checked each second) until Ctrl-C ») :
> Follow the session until Ctrl-C: in a terminal, the running flow is drawn in boxes with its marks and a spinner on the running step; elsewhere, the text is printed again on each new event

**Wiki, ligne du bloc « Reading the output »** (validée, Q3) :
> `armadai flow status --watch            # follow the session until Ctrl-C (in a terminal: the running flow drawn in boxes)`

**Wiki, paragraphe sous ce bloc** (validé, Q3) :
> In a terminal, `--watch` draws the run in progress in boxes (the drawing of `armadai graph --format ascii`), each step with its mark, and a spinner (`| / - \`) after the running step. The last run still open is drawn (the one `--line` names), and the other runs are listed under it as above. The view is redrawn within 250 ms of a new event, without clearing the screen, and stays still while no step runs. When the drawing does not fit the terminal (`COLUMNS` and `LINES`, else the terminal's size, else 80×24), that run is listed instead of drawn. A flow shown as `a | b` is listed too. The cursor is hidden while watching and shown again on exit, Ctrl-C included. Outside a terminal (a pipe, a file), `--watch` prints the list again on each new event, with no cursor or erase sequence, even with `--color always`.

**Wiki, ligne des « Limits »** (validée, Q3) :
> - `--watch` on Windows is not tested: it draws without hiding the cursor. A terminal that shows `●` or `✓` two columns wide shifts the boxes. A list taller than the terminal scrolls, and the view is then redrawn from the top of the screen.

**CHANGELOG**, sous `## Unreleased` / `### Added` (validée, Q3) :
> - `armadai flow status --watch` in a terminal draws the run in progress in boxes, each step with its mark (`✓ ● ○ ? ✗ ~ -`) and a spinner on the running step, redrawn within 250 ms of a new event without clearing the screen; the other runs stay listed under it, and a run that does not fit the terminal is listed instead. The cursor is hidden while watching and shown again on exit, Ctrl-C included. Outside a terminal the output is the list, as before, and no longer carries a clear-screen sequence, even with `--color always`. See [Flow status](docs/wiki/flow-status.md).

Les textes du wiki et du CHANGELOG ci-dessus sont ceux de la tâche 5. La tâche 6 les remplace par les suivants (Q5 : « Oui, textes proposés (Recommandé) ») et déplace le paragraphe (« Déplacer (Recommandé) ») :

**Wiki, paragraphe du watch, après l'exemple de liste et sa table de marques** (validé, Q5) :
> In a terminal, `--watch` draws the run in progress in boxes (the drawing of `armadai graph --format ascii`), each step with its mark, and a spinner (`| / - \`) after the running step. The last run still open is drawn (the one `--line` names), and the other runs are counted on one line under it (`+ 2 other runs: armadai flow status`). The view is redrawn within 250 ms of a new event, without clearing the screen, and stays still while no step runs and the terminal keeps its size. When the drawing does not fit the terminal (`COLUMNS` and `LINES`, else the terminal's size, else 80×24), that run is listed instead of drawn, with the same count line. A flow shown as `a | b` is listed too. The cursor is hidden while watching and shown again on exit, Ctrl-C included. Outside a terminal (a pipe, a file), `--watch` prints the list again on each new event, with no cursor or erase sequence, even with `--color always`.

**Wiki, phrase d'amorce de l'image** (validée, « Déplacer ») : « as listed below » devient « as listed above », et le reste de la phrase ne change pas.

**Wiki, ligne des « Limits »** (validée, Q5) :
> - `--watch` on Windows is not tested: it draws without hiding the cursor. A terminal that shows `●` or `✓` two columns wide shifts the boxes. A run longer than the terminal is cut, its last line saying how many lines are left out.

**CHANGELOG**, sous `## Unreleased` / `### Added` (validée, Q5) :
> - `armadai flow status --watch` in a terminal draws the run in progress in boxes, each step with its mark (`✓ ● ○ ? ✗ ~ -`) and a spinner on the running step, redrawn within 250 ms of a new event without clearing the screen; the other runs are counted on one line under it, a run that does not fit the terminal is listed instead, and the image never scrolls the terminal. The cursor is hidden while watching and shown again on exit, Ctrl-C included. Outside a terminal the output is the list, as before, and no longer carries a clear-screen sequence, even with `--color always`. See [Flow status](docs/wiki/flow-status.md).

Lignes nouvelles de la tâche 6 : la ligne de compte et la ligne de coupe, toutes deux validées. Il n'y a pas d'autre ligne de sortie nouvelle : pas de diagnostic, pas de message d'erreur. Les autres lignes de l'image sont celles de `render` et de `flow_to_ascii`.

## 10. Essai réel (Claude Code, macOS, avec Dimitri)

Préparation, par `validator` :
- `cargo build` dans le worktree. Le binaire à essayer est `<worktree>/target/debug/armadai`, et le binaire de référence est l'`armadai` de `master` (rc.10 installé, dont le code de `--watch` est celui de `e263db5`).
- Projet : ce dépôt (`.armadai/`, flows `lot` et `task`), déjà lié pour Claude Code. Les hooks d'enregistrement écrivent avec le binaire installé, dont le format de journal ne change pas.

Déroulé :
1. Terminal 1 : une session Claude Code qui lance une étape de flow à sous-agent (une revue `guardian` en lecture seule suffit).
2. Terminal 2 : `<worktree>/target/debug/armadai flow status --watch`. Vérifier que le dessin suit l'exécution et que le repère tourne seulement pendant qu'une étape est `running`. Juger le clignotement dans Terminal.app ou iTerm, puis dans tmux si Dimitri l'utilise.
3. Réduire la fenêtre sous la largeur du dessin : on doit voir la liste. L'agrandir : le dessin revient à l'image suivante. Essayer aussi `COLUMNS=40 … --watch` : on doit voir la liste.
4. Ctrl-C : le curseur doit être visible, puis `echo $?`. Refaire le même geste avec le binaire de `master` : les deux valeurs doivent être égales (critère 4).
5. `… --watch | cat`, puis `… --watch --color always | cat -v` : aucune séquence autre que SGR (critère 3).
6. Au repos (aucune étape `running`), pendant 10 s : `/bin/ps -o time= -p <pid>` pour les deux binaires, et aucune image redessinée (critère 5).
7. Ajouter une ligne au journal pendant le watch : elle doit apparaître en moins d'une seconde, sur 5 essais (critère 6). Le plus simple est qu'une étape démarre dans le terminal 1.

## 11. Ordre et tâches à refaire

Ordre : 1 → 2 → 3 → 4 → 5, chaque tâche dépendant de la précédente. Il n'y a pas de tâche à refaire (premier plan du lot, aucune tâche commencée avant les réponses ci-dessous).

Reprise après le rejet de `review` à la tête `94c0554` : les tâches 1 à 5 sont faites et ne se refont pas. La **tâche 6** corrige ce que les tâches 3 (`watch.rs`), 4 (`term.rs`, `flow.rs`) et 5 (wiki, CHANGELOG) ont livré, en un seul passage par le flow `task`. Q4 et Q5 sont tranchées et rien ne la bloque. Ensuite, `review` relit de nouveau la branche entière.

## Décisions de Dimitri (2026-11-09, plan)

Mot pour mot, relayées par `lead` :
- Q1 : « Le bloc complet (Recommandé) ». Sous le dessin vient le bloc complet de `render` : lignes d'étapes, liens, transitions (tâche 3).
- Q2 : « 250 ms, sans redessin (Recommandé) ». Un seul pas de 250 ms, au repos compris, où la boucle ne fait qu'un `stat` du journal, sans relecture ni redessin (tâche 4). La spec §4.2 est alignée, et le critère 6 reste inchangé.
- Q3 : « Validés tels quels (Recommandé) ». Toutes les valeurs et tous les textes du §9 sont validés.

Après le rejet de `review` (tête `94c0554`), le 2026-11-09, mot pour mot, relayées par `lead` (tâche 6) :
- « Une ligne de compte (Recommandé) » : sous le dessin, une seule ligne de compte remplace les autres exécutions, et une image identique n'est jamais redessinée.
- « Ce texte (Recommandé) » : `+ <n> other runs: armadai flow status` ; au singulier `+ 1 other run: armadai flow status`.
- « Déplacer (Recommandé) » : dans `docs/wiki/flow-status.md`, le paragraphe du watch et son image passent après l'exemple de liste et sa table de marques. « as listed below » devient « as listed above ». Le reste du texte validé ne change pas.
- Q4 : « Couper et le dire (Recommandé) ». Au palier 3, l'image est coupée à `lignes − 1`. Sa dernière ligne vaut `+ <n> more lines: armadai flow status`, et une ligne trop large finit par `…` (tâche 6.1).
- Q5 : « Oui, textes proposés (Recommandé) ». Le paragraphe du wiki, la ligne des limites et la ligne du CHANGELOG prennent les textes de l'option (a) (§9, tâche 6.4).

## Questions posées (historique, réponses ci-dessus)

**Q4. Quand même la liste de l'exécution suivie ne tient pas à l'écran** (palier 3, tâche 6.1). Cela arrive avec une exécution plus longue que l'écran ou un terminal très petit. Sans coupe, l'écran défile de nouveau (R1).
- (a) Couper à `lignes − 1` : la dernière ligne gardée est remplacée par `+ <n> more lines: armadai flow status` (`line` au singulier), et une ligne plus large que l'écran est coupée à `colonnes − 1` caractères suivis de `…`. Même forme que la ligne de compte, et on sait ce qui manque et où le lire. **Recommandé.**
- (b) Couper sans le dire : l'écran ne défile plus, mais rien n'indique la coupe.
- (c) Remplacer l'image par une seule ligne `terminal too small: armadai flow status`.

**Q5. Phrases du wiki et du CHANGELOG que la ligne de compte rend fausses** (tâche 6.4). « Le reste du texte validé ne change pas », mais ces trois phrases décrivent l'ancien comportement :
- Wiki, paragraphe : « and the other runs are listed under it as above » devient « and the other runs are counted on one line under it (`+ 2 other runs: armadai flow status`) ». « that run is listed instead of drawn » devient « that run is listed instead of drawn, with the same count line ». « stays still while no step runs » devient « stays still while no step runs and the terminal keeps its size ».
- Wiki, limites : on retire « A list taller than the terminal scrolls, and the view is then redrawn from the top of the screen. » et on ajoute, selon Q4 (a), « A run longer than the terminal is cut, its last line saying how many lines are left out. »
- CHANGELOG : « the other runs stay listed under it, and a run that does not fit the terminal is listed instead » devient « the other runs are counted on one line under it, a run that does not fit the terminal is listed instead, and the image never scrolls the terminal ».
- (a) Ces textes (**Recommandé** : ils disent ce que fait le code).
- (b) Garder les phrases actuelles : la doc contredit alors le code.
- (c) D'autres textes.

**Q1. Ce qui reste sous le dessin de l'exécution en cours** (tâche 3). La spec dit « les lignes de `render` de cette exécution (liens pris plusieurs fois, transitions UTC) pour qu'aucune information ne disparaisse ». Or l'état et la note d'une étape (`approved`, `(no STATE line)`) ne tiennent pas dans les boîtes sans les faire bouger d'une image à l'autre.
- (a) Le bloc complet de `render` sous le dessin : lignes d'étapes, liens, transitions. Rien ne disparaît et aucun texte n'est nouveau, mais la vue est plus haute (environ 4 lignes de plus pour `deliver`) et passe plus tôt en liste sur un petit terminal. **Recommandé** : c'est la lecture qui tient la promesse « aucune information ne disparaît ».
- (b) Seulement les liens pris plusieurs fois et les transitions. La vue est plus courte, mais l'état et la note de chaque étape ne se lisent plus que par `flow status`.

**Q2. Le pas au repos** (tâche 4). La spec §4.2 dit « sans étape Running, la boucle dort par pas de 1 s », alors que le critère 6 demande qu'un nouvel événement apparaisse en moins d'une seconde. Avec un pas de 1 s au repos, un événement qui arrive au repos peut mettre jusqu'à 1 s.
- (a) Un seul pas de 250 ms : au repos, un `stat` du journal par pas, sans relecture ni redessin. Le critère 6 tient partout. Le coût CPU d'un `stat` toutes les 250 ms est à mesurer à l'essai réel (critère 5 : « pas plus de CPU que master »). **Recommandé.**
- (b) Garder 1 s au repos, et réécrire le critère 6 en « moins d'une seconde quand une étape tourne, une seconde au plus sinon ».
- (c) 500 ms au repos (compromis).

**Q3. Les valeurs et textes du §9** (aide de `--watch`, ligne et paragraphe du wiki, ligne des limites, ligne du CHANGELOG, défaut 80×24, seuil de repli à `lignes − 1`, préfixe de deux espaces) : les valider en bloc, ou dire lesquels changer.
