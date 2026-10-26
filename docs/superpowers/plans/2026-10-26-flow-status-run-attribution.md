# flow status : attribuer une exécution au flow lancé — plan d'implémentation

> **Pour les agents :** sous-skill requise : superpowers:subagent-driven-development (recommandé) ou superpowers:executing-plans, tâche par tâche. Les étapes se cochent (`- [ ]`).

**Objectif :** enregistrer le nom de la commande que l'utilisateur tape dans Claude Code (`UserPromptExpansion`) et, quand ce nom est l'identifiant d'un flow, attribuer l'exécution à ce flow : plus de mauvais flow deviné, plus d'étape perdue quand des sous-agents se chevauchent, l'exécution précédente affichée `interrupted`. Sans ligne `command` dans le journal, la sortie ne change pas d'un caractère.

**Spec :** `docs/superpowers/specs/2026-10-26-flow-status-run-attribution-design.md` (avec son « Cadre » et les décisions de Dimitri du 2026-10-26). Issue publique : #527.

**Branche de lot :** `fix/flow-status-run-attribution`, depuis `master` (`6da1328` au moment du plan). La spec et ce plan, non suivis sur `master`, sont commités par la tâche 1 (étape 0).

**Architecture :** rien de neuf. `record.rs` apprend un 4e événement et une 3e sorte de ligne ; `claude_settings.rs` installe un 4e hook par le même chemin que les trois autres (fusion, manifeste, `unlink` inchangés : ils sont génériques) ; `runs.rs` traite la ligne `command` comme une frontière et place les sous-agents dans l'exécution attribuée ; `render.rs` affiche `interrupted`. `armadai-model` n'est pas touché (pas de `schema-specialist`, pas de vérification de `frozen/armadai-audit`).

## Ce que dit la doc des hooks (https://code.claude.com/docs/en/hooks.md, lue le 2026-10-26)

Cité :
- « Runs when a user-typed command expands into a prompt before reaching Claude. [...] typing `/skillname` directly bypasses `PreToolUse`. `UserPromptExpansion` fires on that direct path. »
- Matcher : « Matches on `command_name`. Leave the matcher empty to fire on every prompt-type command. »
- Entrée : « In addition to the common input fields, UserPromptExpansion hooks receive `expansion_type`, `command_name`, `command_args`, `command_source`, and the original `prompt` string. The `expansion_type` field is `slash_command` for skill and custom commands, or `mcp_prompt` for MCP server prompts. » Exemple : `{"session_id":"abc123","transcript_path":"/Users/.../00893aaf.jsonl","cwd":"/Users/...","permission_mode":"default","hook_event_name":"UserPromptExpansion","expansion_type":"slash_command","command_name":"example-skill","command_args":"arg1 arg2","command_source":"plugin","prompt":"/example-skill arg1 arg2"}`.
- Sortie : « The exceptions are `UserPromptSubmit`, `UserPromptExpansion`, `SessionStart`, and `PostModelSwitch`, where Claude Code adds plain-text stdout as context that Claude can see and act on. »
- Hooks asynchrones : « When an async hook fires, Claude Code starts the hook process and immediately continues without waiting for it to finish. The hook receives the same JSON input via stdin as a synchronous hook. » ; « After the background process exits, Claude Code delivers the `additionalContext` and `systemMessage` fields from the hook's JSON response to Claude on the next conversation turn ». La page ne dit pas ce que devient le texte brut de stdout d'un hook asynchrone sur `UserPromptExpansion` : on garde zéro octet (spec, critère 1).

Non trouvé dans la page : la version de Claude Code qui introduit `UserPromptExpansion` (non vérifié) ; la forme de `command_name` pour une commande de plugin (l'exemple a `command_source: "plugin"` et `command_name: "example-skill"`, sans espace de noms ; non vérifié). Règle de Dimitri : égalité stricte, toute autre forme retombe sur l'inférence.

## Séquence

```mermaid
sequenceDiagram
    actor D as Dimitri
    participant CC as Claude Code
    participant H as armadai __record-flow-event
    participant J as .armadai/state/claude/s1.jsonl
    participant FS as armadai flow status
    D->>CC: tape /bugfix secret arg
    CC->>H: UserPromptExpansion (async, stdin JSON)
    H->>J: {"ts":N,"event":"command","command":"bugfix"}
    Note over H: stdout et stderr vides, code 0
    CC->>H: SubagentStart / SubagentStop / PreToolUse(SubagentHandback)
    H->>J: lignes start / stop (inchangées)
    FS->>J: read_log (une ligne illisible est sautée)
    FS->>FS: runs(): command = frontière si flows.contains_key(nom)
    FS->>D: render(): bugfix finished / interrupted / in progress
```

Le hook n'écrit que le nom ; `command_args` et `prompt` ne quittent jamais le processus. La lecture reste pure (`runs(project, events)`) : la frontière est une ligne du journal comme une autre.

```mermaid
stateDiagram-v2
    [*] --> ouverte: command <flow> (attribuée) ou inférence (non attribuée)
    ouverte --> finished: plus rien en cours ni lien à prendre
    ouverte --> interrupted: command <flow> suivante
    interrupted --> finished: un stop tardif d'un agent qui y était placé la termine
    ouverte --> masquee: attribuée et aucune étape démarrée à la fin du journal
    masquee --> [*]: ni affichée ni comptée par --list
```

Une exécution attribuée sans aucune étape démarrée n'est pas rendue (décision de Dimitri du 2026-10-26, en fin de plan).

## Contraintes globales

- **Barrière complète**, code retour de chaque commande lu, jamais de tube qui le masque : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`. `frozen/armadai-audit` non concerné (aucun crate dont il dépend ne change) ; si une tâche touche `armadai-core` ou `armadai-model`, elle s'arrête et le dit.
- **Un worktree et un target cargo privés par tâche** : `git worktree add <wt> fix/flow-status-run-attribution` (ou une branche de tâche rebasée sur celle du lot), `CARGO_TARGET_DIR=<wt>/target`. Jamais `git stash`. Aucune commande en arrière-plan.
- **Tests** : `crates/armadai` est un binaire seul (`cargo test --bin armadai <nom>` ou `--test <fichier>`, jamais `--lib`), toujours `--no-fail-fast`. Égalité exacte (texte entier de `flow status`, ligne entière du journal, valeur JSON entière de `settings.json`), pas de `contains` qu'une autre sortie satisferait, sauf l'absence (`!contains("secret")`) en complément d'une égalité. Fixtures à valeurs distinctes deux à deux (horodatages, ids d'agents).
- **Une mutation vue rouge par test ajouté ou changé** : casser le code protégé, voir le rouge (sortie citée dans le rapport), restaurer avec une mtime neuve (`touch`, jamais `cp -p`). Les mutations prévues sont listées par tâche ; elles sont un minimum.
- **Essais du binaire** avec `HOME` et `ARMADAI_CONFIG_DIR` dans un dossier temporaire ; les tests d'intégration le font déjà (`Env`). Les tests du hook retirent `RUST_LOG` de l'environnement (`env_remove("RUST_LOG")`).
- **Journal** : une ligne JSON par événement, une seule écriture en ajout ; la ligne de commande est exactement `{"ts":N,"event":"command","command":"<command_name>"}`, rien d'autre. Les anciennes lignes se lisent telles quelles.
- **Règle du MVP** : `command_name == flow id`, sensible à la casse (`project.flows.contains_key(name)`). Une commande qui ne nomme aucun flow ne change rien.
- **Commits** Conventional, un type par sujet, en anglais, trailer `Co-Authored-By` fourni par la session. PR ouverte à la fin du lot, corps en français, pas de fusion sans l'accord exprès de Dimitri.

## Choix de conception

| Choix | Options et coût | Retenu |
|---|---|---|
| Forme de `FlowEvent` pour la ligne `command` | **A.** Garder la struct ; `EventKind::Command` ; `#[serde(default)]` sur `agent_id`/`agent_type` ; un champ `command: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`) ; une fonction `log_line(&FlowEvent) -> String` qui écrit la ligne `command` par une petite struct privée `{ts, event, command}` (ordre des clés garanti) et les autres par `serde_json::to_string` comme aujourd'hui. Coût : ~25 lignes, les littéraux `FlowEvent` des tests gagnent `command: None`. Les lignes `start`/`stop` restent identiques à l'octet. **B.** Une énumération étiquetée `#[serde(tag = "event")]` (`Start{..}`, `Stop{..}`, `Command{ts, command}`). Coût : réécrire les accès `e.agent_id`/`e.agent_type`/`e.state` de `runs.rs` (une vingtaine de sites) et `merge_stops`, pour un type plus juste. | **A** : le plus petit diff ; le `match` exhaustif sur `EventKind` force `runs()` et `merge_stops()` à traiter `Command`. Limite connue : une ligne `start` forgée sans `agent_id` se lit désormais avec `agent_id == ""` au lieu d'être sautée (jamais écrite par le hook). |
| Ordre des hooks | Le 4e en dernier dans `flow_status_entries` ; ou trié. | Dernier (Cadre, « Observé, non décidé » : habitude du code). |
| Lien d'une étape démarrée avant la fin de sa source (exécution attribuée) | **A.** L'étape est placée `running` sans transition et notée « en attente » ; quand une étape `s` finit, chaque étape en attente que `s` atteint (`reach`) reçoit la transition `s → t`, horodatée à la fin de `s`, avec l'état de `s`. Pour que ce lien ne reste pas « à prendre » (sinon l'exécution ne finit jamais), `linked_from(to)` compare au plus récent de `started[to]` et d'un nouveau `entered[to]` (le tick de cette fin). **B.** Réécrire le tick de départ de `t` : casse le cas où `t` a fini avant sa source (scénario 2 de l'issue : `fix-ui` finit à 30, `test` à 40 ; `fix-ui` ne mènerait plus à `review`). | **A** (B mesuré faux à la main sur le scénario 2, voir tâche 3). |
| Ordre des blocs | Aujourd'hui `finished` d'abord, puis l'ordre de départ. Le critère 5 veut le bloc `interrupted` (parti le premier) avant `bugfix  finished`, le critère 6 `finished` avant `in progress`. | Clé de tri : « terminée ou interrompue » avant « en cours », ordre de départ conservé (tri stable). Sans ligne `command`, aucune exécution n'est interrompue : ordre inchangé. |
| Exécution attribuée sans étape démarrée | Masquée partout ; ou affichée (`bugfix  in progress`, tout `pending`) mais non comptée par `--list`. | Masquée partout (le Cadre exige que `--list` ne la compte pas ; une seule règle). Décidé par Dimitri le 2026-10-26. |

---

### Tâche 1 [adapter-specialist] : enregistrer la commande tapée

**Fichiers :**
- Commit d'abord : `docs/superpowers/specs/2026-10-26-flow-status-run-attribution-design.md`, `docs/superpowers/plans/2026-10-26-flow-status-run-attribution.md`.
- Modifier : `crates/armadai/src/flow_status/record.rs` (types, `event_from_hook`, `log_line`, `record`, en-tête du module), les littéraux `FlowEvent` qui ne compilent plus (`crates/armadai/src/flow_status/runs.rs`, tests : helpers `start`/`stop` ; ajouter `command: None` sans rien changer d'autre).
- Test d'intégration : le fichier qui lance déjà `__record-flow-event` s'il existe (le trouver : `grep -rln "__record-flow-event" crates/armadai/tests`), sinon `crates/armadai/tests/record_flow_event_cli.rs`.

**Interfaces produites (pour les tâches 2 à 4) :**
```rust
pub enum EventKind { Start, Stop, Command }            // sérialisé start / stop / command
pub struct FlowEvent {
    pub ts: u64,
    pub event: EventKind,
    #[serde(default)] pub agent_id: String,
    #[serde(default)] pub agent_type: String,
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub command: Option<String>,
}
/// La ligne du journal, `\n` final compris.
pub fn log_line(e: &FlowEvent) -> String;
```
`event_from_hook` : `"UserPromptExpansion"` avec `expansion_type == "slash_command"` et un `command_name` non vide → `FlowEvent { event: Command, agent_id: "", agent_type: "", state: None, command: Some(command_name) }`, avec `session_id` et `cwd` exigés comme aujourd'hui. Les branches existantes ne changent pas.

- [ ] **Étape 0 :** créer la branche `fix/flow-status-run-attribution` depuis `master`, commiter la spec et ce plan : `docs(flow): spec and plan for run attribution from the typed command`.
- [ ] **Étape 1 — tests (rouges d'abord) :**
  - `record.rs`, const `EXPANSION` = l'exemple de la doc avec `"command_name":"bugfix"`, `"command_args":"secret arg"`, `"command_source":"project"`, `"prompt":"/bugfix secret arg"`, `session_id` `abc123`, `cwd` `/p`. Le helper `line()` passe à `log_line` (au lieu de `serde_json::to_string`), pour que les trois tests existants `subagent_start_records_a_start`, `subagent_stop_records_the_state_of_the_last_message`, `a_handback_records_the_state_of_its_message` protègent aussi `log_line`, avec leurs valeurs actuelles inchangées.
  - `a_typed_command_records_its_name_only` : `line(EXPANSION) == ("abc123", "/p", r#"{"ts":100,"event":"command","command":"bugfix"}"#)` (sans le `\n` si le helper le retire ; le dire).
  - `other_events_record_nothing` étendu de quatre entrées : `EXPANSION` avec `"expansion_type":"mcp_prompt"`, sans `expansion_type`, avec `"command_name":""`, sans `command_name` → `None` chacune (la boucle existante couvre chaque entrée ; vérifier qu'aucune n'est oubliée).
  - `old_and_command_lines_read_back` : `serde_json::from_str::<FlowEvent>` de `{"ts":1,"event":"start","agent_id":"i1","agent_type":"an","state":null}` donne l'événement avec `command: None` ; de `{"ts":2,"event":"command","command":"bugfix"}` donne `FlowEvent { ts: 2, event: Command, agent_id: "", agent_type: "", state: None, command: Some("bugfix") }`.
  - Intégration `a_typed_command_is_recorded_silently` (projet temporaire avec `.armadai/project.yaml`, `.git/`, `HOME`/`ARMADAI_CONFIG_DIR` isolés, `RUST_LOG` retiré), quatre lancements de `armadai __record-flow-event`, chacun vérifié par `(code, stdout en octets, stderr en octets) == (Some(0), b"", b"")` :
    1. la charge du critère 1 de la spec (`session_id` `s1`, `cwd` le projet) : le journal `.armadai/state/claude/s1.jsonl` vaut exactement `format!("{{\"ts\":{ts},\"event\":\"command\",\"command\":\"bugfix\"}}\n")`, `ts` relu dans la ligne et compris entre l'heure avant et après le lancement ; le fichier ne contient ni `secret` ni `/bugfix` ;
    2. la même avec `"expansion_type":"mcp_prompt"` : le journal n'a pas changé (une ligne) ;
    3. `{not json` : idem ;
    4. la charge valide avec un `cwd` hors de tout projet : aucun fichier créé sous ce `cwd`.
- [ ] **Étape 2 :** implémenter (le `record` écrit `log_line(&event)` ; la commande cachée et `main.rs` ne changent pas). Mettre à jour l'en-tête du module (`UserPromptExpansion`, « only the command name »).
- [ ] **Étape 3 — mutations à montrer rouges :** (a) écrire `command_args` dans la ligne → `a_typed_command_records_its_name_only` rouge ; (b) écrire la ligne `command` par `serde_json::to_string(&event)` → idem (les champs `agent_id`, `agent_type`, `state` apparaissent) ; (c) retirer la garde `expansion_type == slash_command` → `other_events_record_nothing` et l'intégration (cas 2) rouges ; (d) retirer `#[serde(default)]` de `agent_id` → `old_and_command_lines_read_back` rouge ; (e) un `println!` ou `eprintln!` dans `record_from_stdin` → l'intégration rouge sur stdout, puis sur stderr (deux mutations distinctes).
- [ ] **Étape 4 :** barrière ; commit `fix(flow): record the name of the command a user types`.

Critère couvert : 1.

### Tâche 2 [adapter-specialist] : `link` installe le 4e hook

**Fichiers :**
- Modifier : `crates/armadai/src/compile/claude_settings.rs` (`flow_status_entries` : `("UserPromptExpansion", None)` en dernier ; commentaire de la fonction avec la citation de la doc sur le matcher vide) ; `crates/armadai/tests/link_new_cli.rs` ; les golden outputs qui contiennent les hooks d'enregistrement ; `docs/wiki/link.md` (l. 57 : « four async hooks ... `UserPromptExpansion` (a typed command, its name only) »).
- Liste exacte des golden : **non dressée par l'architecte** (aucun outil de recherche dans cette étape). Le spécialiste la dresse en premier (`grep -rln "__record-flow-event" crates/armadai/tests crates/armadai/src`), la cite dans son rapport et met à jour chaque fichier listé (régénération par le mécanisme du dépôt, `UPDATE_GOLDEN=1` selon le plan précédent, à vérifier dans le harnais, puis relecture du diff) ou dit pourquoi il ne change pas. La spec nomme `crates/armadai/tests/golden/init_pack/` et `link_new_cli.rs`.
- Ne pas toucher : `compile/patch.rs`, `linker/manifest.rs`, `cli/unlink` : la fusion, la propriété par valeur et le retrait sont génériques sur `HookEntry` (lu dans `merge_settings`/`remove_entries`). Si un test montre le contraire, s'arrêter et le rapporter.

**Interfaces :** consomme le nom de commande `armadai __record-flow-event` seulement (indépendante de la tâche 1). Produit `flow_status_entries()` à 4 entrées.

- [ ] **Étape 1 — tests :**
  - `a_project_with_a_flow_records_subagents_after_the_gate` : la liste attendue devient `[gate_entry(), recording("SubagentStart", None), recording("SubagentStop", None), recording("PreToolUse", Some("SubagentHandback")), recording("UserPromptExpansion", None)]`.
  - Nouveau `a_project_linked_with_three_recording_hooks_gets_the_fourth_on_relink` (`claude_settings.rs`) : `old` = les trois premières entrées ; `(first, owned1) = merge_settings(Some(USER), &old, &none(), &Owned::default())` ; `(second, owned2) = merge_settings(Some(&first), &flow_status_entries(), &none(), &owned1)`. Attendus : `value(&second)["hooks"]["UserPromptExpansion"] == json!([{"hooks": [{"type": "command", "command": "armadai __record-flow-event", "async": true}]}])` ; `owned2.entries == flow_status_entries()` ; `value(&remove_entries(&second, &owned2).unwrap()) == value(USER)`.
  - `link_new_cli.rs::link_then_unlink_gives_the_user_tree_back_and_relinking_is_idempotent` : l'objet `hooks` attendu gagne `"UserPromptExpansion": [{"hooks": rec}]` ; le reste du test (relink sans changement, `unlink` qui rend l'arbre de l'utilisateur) inchangé : il couvre le retrait avec manifeste. `without_a_manifest_unlink_removes_only_unchanged_generated_files` (compare `settings.json` entier) couvre le retrait sans manifeste ; le lancer et citer sa sortie.
  - Golden : mis à jour et relus. La ligne de rapport reste une seule ligne, mais elle nomme désormais les quatre hooks, `SubagentStart, SubagentStop, PreToolUse (SubagentHandback), UserPromptExpansion` (décision de Dimitri du 2026-10-26, « Nommer les 4 (Recommandé) ») : chacun des 11 golden qui contiennent le rapport change de cette seule ligne, et d'aucune autre ; le vérifier dans le diff.
- [ ] **Étape 2 :** implémenter (une ligne dans `flow_status_entries`), régénérer les golden.
- [ ] **Étape 3 — mutations :** (a) retirer l'entrée `UserPromptExpansion` → les trois tests ci-dessus et les golden rouges (citer chacun) ; (b) lui donner un matcher (`Some("bugfix")`) → `a_project_with_a_flow...` et `link_then_unlink...` rouges ; (c) dans `merge_settings`, ne pousser dans `owned.entries` qu'une entrée déjà dans `previous.entries` → `..._gets_the_fourth_on_relink` rouge.
- [ ] **Étape 4 :** barrière ; commit `fix(link): install the UserPromptExpansion recording hook`.

Critère couvert : 8 (et la partie `link.md` du 9).

### Tâche 3 [cli-specialist] : la commande comme frontière dans `runs()`, et `interrupted`

Dépend de la tâche 1 (`EventKind::Command`, `FlowEvent.command`).

**Fichiers :**
- Modifier : `crates/armadai/src/flow_status/runs.rs`, `crates/armadai/src/flow_status/render.rs`, `crates/armadai/src/cli/flow.rs` (seulement le `long_about` : une phrase sur l'exécution attribuée à la commande tapée et la marque `interrupted` ; vérifier qu'aucun golden d'aide ne la fige, sinon le mettre à jour).

**Interfaces :**
- `Run` gagne `pub interrupted: bool` : fermée par une commande de flow suivante avant d'avoir fini ; jamais vrai quand `finished`. Les littéraux `Run` des tests existants gagnent `interrupted: false` et rien d'autre (tout autre changement d'un test existant est un signal d'alarme pour la revue).
- `Builder` gagne `attributed: Option<usize>` (l'exécution de la dernière commande de flow). `Building` gagne `attributed: bool`, `interrupted: bool`, `awaiting: BTreeSet<String>` (étapes placées sans lien), `entered: HashMap<String, usize>` (tick de la fin de la source qui a noté un lien tardif).
- Fonction factorisée (pas de copie) : l'enregistrement des sauts d'un lien (`from`, étapes du coordinateur traversées, `to`, ts, état) aujourd'hui dans `start`, appelée aussi par `stop`.

**Règles (spec §4.2) :**
1. `runs()` : `EventKind::Command` → si `project.flows.contains_key(name)`, `boundary(name)`, sinon rien ; `merge_stops` laisse passer la ligne sans toucher `last_stop`.
2. `boundary(flow)` : chaque exécution non `dropped`, non `finished` devient `interrupted` (candidates groupées comprises) ; `boundary` ne pose pas `closed` (écart de la tâche 3, approuvé par `guardian` : une fois `attributed` posé, il reste `Some` jusqu'à la fin de la session, l'écriture de `closed` serait morte) ; une `Building` neuve du flow s'ouvre dans son propre groupe, `attributed = true` ; `Builder.attributed = Some(index)`.
3. `start(e)` quand `Builder.attributed == Some(i)` : seules les étapes du flow de `i` tenues par `e.agent_type` comptent. Aucune → l'événement est ignoré (pas d'exécution fantôme). Deux ou plus → la règle actuelle (`?` « ambiguous step »). Une seule `sid` → `linked_from(sid)` s'il existe (transition comme aujourd'hui), sinon placée `running` sans transition et ajoutée à `awaiting`, sauf si `sid` est une étape où une exécution peut s'ouvrir, c'est-à-dire une étape d'entrée ou une étape juste après un relais : celle-ci n'entre jamais dans `awaiting` (écart de la tâche 3, approuvé par `guardian` : sinon `review-loop` prendrait un faux lien). Les branches d'inférence (entrée qui ferme, ouverture à mi-flow, groupes `a | b`) ne s'appliquent pas. Une étape en amont d'une étape placée qui n'a pas tourné dans l'exécution attribuée prend la marque `- … (not run in this session)` ; une étape en aval qui n'a pas encore tourné reste `○ pending` (décision de Dimitri du 2026-10-26, « not run in this session (Recommandé) »). Pas de condition « rien en cours » pour ce marquage : une étape pas encore lancée s'affiche `not run` dès qu'une étape après elle a fini, et jusqu'à son propre démarrage, où elle passe `running` ; cet affichage transitoire est accepté et le wiki l'explique (tâche 4).
4. `stop` d'une étape `s` d'une exécution attribuée, après la mise à jour de sa vue et avant le calcul de `finished` : pour chaque `t` de `awaiting` que `reach(s)` atteint, la transition `s → t` est notée (ts de ce stop, état `leads_with(s)`, sauts du coordinateur compris), `taken` compté, `entered[t] = tick`, `linked_at[t] = ts`, `t` retiré d'`awaiting`. Deux gardes, conservées et testées : aucun lien tardif depuis une étape `?` ou `✗`, et aucun lien d'une étape vers elle-même (`s == t`).
5. `linked_from(to)` compare `max(started[to], entered[to])` au lieu de `started[to]` ; sans `entered` (toute exécution non attribuée), comportement identique.
6. Fin de `runs()` : une exécution `attributed` sans aucune étape démarrée n'est pas rendue ; `merge_candidates` calcule `interrupted` (interrompue si toutes ses candidates le sont, et jamais quand elle a fini) ; `finish()` n'a pas de ligne `interrupted && !finished` (écart de la tâche 3, approuvé par `guardian` : `merge_candidates` la calcule déjà).
7. `render` : statut `finished`, sinon `interrupted`, sinon `in progress` ; tri stable par « en cours » en dernier (`sort_by_key(|r| !(r.finished || r.interrupted))`). `print_painted` reconnaît déjà l'en-tête (`<flow>  ...`).

Vérification à la main faite par l'architecte (règle 4 et 5) sur les scénarios 1 et 2 de l'issue, en ticks : scénario 2 avec `bugfix`, `fix-ui` démarre (tick 3) avant `test` (tick 4) et finit (5) avant lui (6) ; au stop de `test`, `entered[fix-ui] = 6` ferme `test → fix-ui`, et `fix-ui` (départ 3 < arrêt 5) mène encore à `review`. À rejouer par les tests, pas à croire.

**Tests** (`runs.rs`, module de tests ; texte comparé par un helper `text(p, events) = render::render(&runs(p, events), "s1")`). Fixtures : `issue()` = projet de l'issue #527 (`schema_version: 1`, agents `an u t r v`, flows `evolution` et `bugfix` copiés de l'issue) ; `issue_with_docs()` = le même plus l'agent `x` et un flow `docs` (`steps:\n  write: { agent: x }\n`) ; `bugfix_only()` = agents `an t u r`, flow `bugfix` à quatre étapes (`analyse -> test -> fix-ui -> review`). `T = 1_792_660_500` (09:15:00 UTC). Helpers `start(ts, id, agent)`, `stop(ts, id, agent, None)` existants, et `cmd(ts, name)` = `FlowEvent { ts, event: Command, agent_id: "", agent_type: "", state: None, command: Some(name) }`. Notation ci-dessous : `an i1 0/10` = `start(T+0, "i1", "an")`, `stop(T+10, "i1", "an", None)`.

Séquences :
- `S1` (scénario 1 de l'issue) : `an i1 0/10`, `start(T+20,i3,t)`, `start(T+30,i2,u)`, `stop(T+40,i3,t)`, `stop(T+50,i2,u)`, `r i4 60/70`, `v i5 80/90`.
- `S2` (scénario 2) : `an i1 0/10`, `start(T+20,i2,u)`, `start(T+21,i3,t)`, `stop(T+30,i2,u)`, `stop(T+40,i3,t)`, `r i4 60/70`, `v i5 80/90`.
- `S4` (un seul flow) : `an i1 0/10`, `start(T+20,i2,t)`, `start(T+30,i3,u)`, `stop(T+40,i2,t)`, `stop(T+50,i3,u)`, `r i4 60/70`.
- `RUN(o)` (exécution complète décalée de `o`) : `an 10/20`, `t 30/40`, `u 50/60`, `r 70/80`, `v 90/100` (ids `i2`..`i6`), tous à `T+o+…`.

Textes attendus (en-tête compris, égalité exacte) :

`BUGFIX_S1` (`cmd(T,"bugfix")` + `S1`) :
```
session s1 (UTC)

bugfix  finished
  ✓ analyse   done
  ✓ test      done
  ✓ fix-ui    done
  ✓ review    done
  ✓ validate  done
  09:15:20  analyse → test  (done)
  09:15:40  test → fix-ui  (done)
  09:16:00  fix-ui → review  (done)
  09:16:20  review → validate  (done)
```
`BUGFIX_S2` (`cmd(T,"bugfix")` + `S2`) : comme `BUGFIX_S1`, la première transition à `09:15:21  analyse → test  (done)`.

`EVOLUTION_S2` (`cmd(T,"evolution")` + `S2`) :
```
session s1 (UTC)

evolution  finished
  ✓ analyse   done
  ✓ ui        done
  ✓ test      done
  ✓ review    done
  ✓ validate  done
  09:15:20  analyse → ui  (done)
  09:15:30  ui → test  (done)
  09:16:00  test → review  (done)
  09:16:20  review → validate  (done)
```
`BUGFIX_S4` (`bugfix_only()`, `cmd(T,"bugfix")` + `S4`) :
```
session s1 (UTC)

bugfix  finished
  ✓ analyse  done
  ✓ test     done
  ✓ fix-ui   done
  ✓ review   done
  09:15:20  analyse → test  (done)
  09:15:40  test → fix-ui  (done)
  09:16:00  fix-ui → review  (done)
```
`INTERRUPTED` (`t i1 0/10`, `cmd(T+3600,"bugfix")`, `RUN(3600)`) :
```
session s1 (UTC)

bugfix | evolution  interrupted
  - analyse   (not run in this session)
  ✓ test      done
  ○ review    pending
  ○ validate  pending

bugfix  finished
  ✓ analyse   done
  ✓ test      done
  ✓ fix-ui    done
  ✓ review    done
  ✓ validate  done
  10:15:30  analyse → test  (done)
  10:15:50  test → fix-ui  (done)
  10:16:10  fix-ui → review  (done)
  10:16:30  review → validate  (done)
```
`TWO_COMMANDS` (`cmd(T,"bugfix")`, `RUN(0)`, `cmd(T+200,"evolution")`, `start(T+210,"i7","an")`) :
```
session s1 (UTC)

bugfix  finished
  ✓ analyse   done
  ✓ test      done
  ✓ fix-ui    done
  ✓ review    done
  ✓ validate  done
  09:15:30  analyse → test  (done)
  09:15:50  test → fix-ui  (done)
  09:16:10  fix-ui → review  (done)
  09:16:30  review → validate  (done)

evolution  in progress
  ● analyse   running
  ○ ui        pending
  ○ test      pending
  ○ review    pending
  ○ validate  pending
```
Sorties actuelles de l'issue (sans ligne `command`), à **mesurer d'abord sur `master`** avant toute modification : si une sortie diffère du bloc ci-dessous, le test fige celle de `master` et le rapport cite l'écart.
- `ISSUE_S1` (`S1`) : `session s1 (UTC)\n\nbugfix  in progress\n  ✓ analyse   done\n  ✓ test      done\n  ○ fix-ui    pending\n  ○ review    pending\n  ○ validate  pending\n  09:15:20  analyse → test  (done)\n`
- `ISSUE_S2` (`S2`) : `session s1 (UTC)\n\nevolution  in progress\n  ✓ analyse   done\n  ✓ ui        done\n  ○ test      pending\n  ○ review    pending\n  ○ validate  pending\n  09:15:20  analyse → ui  (done)\n`
- `ISSUE_LONE` (`t i1 0/10`) : `session s1 (UTC)\n\nbugfix | evolution  in progress\n  - analyse   (not run in this session)\n  ✓ test      done\n  ○ review    pending\n  ○ validate  pending\n`
- `ISSUE_S4` (`bugfix_only()`, `S4`) : `session s1 (UTC)\n\nbugfix  in progress\n  ✓ analyse  done\n  ✓ test     done\n  ○ fix-ui   pending\n  ○ review   pending\n  09:15:20  analyse → test  (done)\n`

Tests, un par critère, sans boucle sur les scénarios (une assertion par séquence) :

| Test | Assertion | Mutation qui doit le faire rougir |
|---|---|---|
| `a_command_attributes_the_run_and_an_overlap_loses_no_step` | `text(issue, cmd+S1) == BUGFIX_S1` | ignorer `awaiting` au stop (règle 4) ; ne pas lire `entered` dans `linked_from` (règle 5 : l'exécution reste `in progress`) |
| `a_step_stopped_before_its_source_still_leads_on` | `text(issue, cmd+S2) == BUGFIX_S2` | réécrire `started[t]` au lieu d'`entered[t]` (option B) |
| `the_same_events_follow_the_flow_the_command_names` | `text(issue, cmd evolution + S2) == EVOLUTION_S2` | `boundary` ouvre le premier flow de `project.flows` au lieu du flow nommé |
| `one_flow_with_an_overlap_finishes` | `text(bugfix_only, cmd+S4) == BUGFIX_S4` | règle 4 retirée |
| `a_command_interrupts_the_open_run` | `text(issue, INTERRUPTED events) == INTERRUPTED` | `boundary` ne marque pas `interrupted` (`in progress` revient) ; tri de `render` inchangé (`finished` d'abord : ordre des blocs inversé) |
| `an_agent_no_step_of_the_attributed_flow_holds_is_ignored` | avec `issue_with_docs()`, les événements `INTERRUPTED` plus `start(T+3625,"x1","x")`, `stop(T+3626,"x1","x",None)` donnent `INTERRUPTED`, et la même chose sans `x1` aussi | en mode attribué, retomber sur l'inférence pour un agent hors du flow (un bloc `docs` apparaît) |
| `a_second_command_closes_the_first_run` | `text(issue, TWO_COMMANDS events) == TWO_COMMANDS` | `boundary` ne remplace pas `Builder.attributed` (l'`an` de 210 rouvre `bugfix`) |
| `a_command_naming_no_flow_changes_nothing` | `text(issue, cmd(T,"deploy")+S1) == ISSUE_S1` et `text(issue, cmd(T,"Bugfix")+S1) == ISSUE_S1` | comparaison insensible à la casse ; toute commande traitée comme frontière |
| `a_command_alone_opens_no_run` | `runs(&issue(), &[cmd(T,"bugfix")]) == vec![]` et `runs(&issue(), &[cmd(T,"bugfix"), cmd(T+1,"evolution")]) == vec![]` | retirer le filtre de la règle 6 |
| `without_a_command_the_issue_scenarios_are_unchanged` | quatre assertions : `ISSUE_S1`, `ISSUE_S2`, `ISSUE_LONE`, `ISSUE_S4` | appliquer le placement attribué à toute exécution (sans commande) |
| `render.rs::an_interrupted_run_says_so_and_comes_before_the_running_one` | `render(&[running, interrupted, finished], "s")` (valeurs distinctes, l'interrompue partie la deuxième) : en-tête, puis le bloc `<flow>  interrupted` puis `finished` dans l'ordre de départ, puis `in progress`, texte exact | statut `in progress` pour une interrompue ; tri d'origine |

S'y ajoutent, pour les écarts de la tâche 3 : un test par garde des liens tardifs (aucun lien depuis `?`/`✗` ; aucun lien d'une étape vers elle-même), et un test sur `review-loop` où une étape d'entrée ou juste après un relais n'entre pas dans `awaiting` (mutation : la laisser y entrer → faux lien rendu) ; chacun avec sa mutation vue rouge.

Les tests existants de `runs.rs` et `render.rs` restent verts sans autre changement que `interrupted: false` ; la suite `flow_status_cli` aussi.

- [ ] **Étape 1 :** mesurer les quatre sorties `ISSUE_*` sur `master` (binaire du worktree, projet temporaire, `HOME`/`ARMADAI_CONFIG_DIR` isolés), les citer.
- [ ] **Étape 2 :** écrire les tests, les voir rouges ; implémenter les règles 1 à 7 ; vert.
- [ ] **Étape 3 :** mutations du tableau, chacune vue rouge puis restaurée (`touch`), sortie citée ; 5 passes de `cargo test --bin armadai flow_status` (rien ne lit l'environnement ici, mais la règle s'applique au module).
- [ ] **Étape 4 :** barrière ; commit `fix(flow): attribute a run to the flow its command names`.

Critères couverts : 2, 3, 4, 5, 6, 7.

### Tâche 4 [cli-specialist] : bout en bout, documentation, CHANGELOG

Dépend des tâches 1 à 3.

**Fichiers :**
- Modifier : `crates/armadai/tests/flow_status_cli.rs` (le `Env` y écrit déjà des journaux ; ajouter un écrivain de ligne `command` et un lancement de `__record-flow-event` avec stdin) ; `docs/wiki/flow-status.md` ; `CHANGELOG.md` (`## Unreleased`, `### Fixed`).

- [ ] **Étape 1 — tests :**
  - `a_typed_command_through_the_hook_attributes_the_run` : la charge `UserPromptExpansion` du critère 1 (`session_id` `s1`, `cwd` le projet de l'issue écrit dans l'`Env`) passée par stdin à `armadai __record-flow-event` ; puis les lignes de `S1` ajoutées au journal (le `ts` réel de la ligne `command` est plus grand que ceux de `S1` : sans effet, l'ordre du fichier fait foi, à confirmer par le test) ; `flow status` rend `(Some(0), BUGFIX_S1, "")`. Mutation : ne pas traiter `EventKind::Command` dans `runs()` → `ISSUE_S1` à la place.
  - `list_does_not_count_a_session_with_only_a_command` : session `c` avec la seule ligne `{"ts":1792660500,"event":"command","command":"bugfix"}` (mtime `T0`), session `s1` = boucle existante (mtime `T0 + 3600`) : `--list` rend exactement `SESSION  LAST EVENT (UTC)     RUNS\ns1       2026-10-22 10:15:00  1\nc        2026-10-22 09:15:00  0\n`. Mutation : retirer le filtre de la règle 6 → `1` pour `c`.
- [ ] **Étape 2 — `docs/wiki/flow-status.md` :** l. 13 « four asynchronous hooks » ; tableau : ligne `UserPromptExpansion` | « a command typed in Claude Code (`/name`, skill or custom command): its name only » ; l. 22-25 : la ligne `command` (nom seul, ni arguments ni prompt) ; nouvelle section « The command you type » : `command_name` égal à un id de flow (casse comprise) ouvre une exécution de ce flow seul, ferme la précédente (`interrupted`), place chaque sous-agent dans l'étape unique qu'il tient, même avant la fin de l'étape source (transition notée à sa fin), ignore un agent hors du flow ; une commande qui ne nomme pas de flow, ou un flow lancé en langage naturel, garde l'inférence ; un projet lié avant doit relancer `armadai link --target claude` ; table des marques/statuts : `interrupted` ; l. 101 (`a | b` seulement sans commande) ; limites l. 132 (arrière-plan : couvert quand la commande est enregistrée) et l. 142 ; ajouter les limites de la spec §4.3 (`/clear`, appel isolé d'un agent du flow attribué, chevauchement sans commande, pas de message quand le hook manque) et « version de Claude Code introduisant `UserPromptExpansion` : non vérifiée ». Ajouter aussi : la marque d'une étape amont sautée dans une exécution attribuée, `- … (not run in this session)`, une étape aval pas encore lancée restant `○ pending` ; l'affichage transitoire (une étape peut s'afficher `not run` dès qu'une étape après elle a fini, puis `running` quand elle démarre) ; la limite connue `agent_id` vide (décision de Dimitri du 2026-10-26, « Ne rien faire (Recommandé) ») ; le risque accepté par la spec au §6 : une transition tardive est horodatée à la fin de sa source, donc possiblement après la fin de l'étape cible. Vérifier que la page ne dit plus « three hooks » nulle part.
- [ ] **Étape 3 — `CHANGELOG.md` :** sous `## Unreleased`, `### Fixed` : « `flow status` attributes a run to the flow whose id is the command you typed (new `UserPromptExpansion` recording hook: relink with `armadai link --target claude`); overlapping background sub-agents no longer leave steps pending; a new flow command marks the previous run `interrupted` (#527). »
- [ ] **Étape 4 :** mutations de l'étape 1 vues rouges ; barrière ; commits `test(flow): pin run attribution end to end` puis `docs(flow): describe the command boundary of flow status`.
- [ ] **Étape 5 (lead puis validator) :** préparer l'essai réel du critère 10 : un projet temporaire aux deux flows de l'essai du 2026-10-26, `armadai link --target claude`, la commande du second flow tapée par Dimitri avec des sous-agents en arrière-plan, `flow status --watch` ; relever dans le journal la ligne `command` (sa présence prouve que `UserPromptExpansion` se déclenche sur cette version de Claude Code).

Critères couverts : 9, 10 (préparation), et 1/2/7 de bout en bout.

## Ordre

1 → 2 → 3 → 4. La tâche 2 ne dépend pas de la tâche 1 et pourrait tourner en parallèle de la 3 dans un autre worktree ; recommandé : en série (même branche, revues `guardian` plus simples).

## Couverture

| Critère de la spec | Tâche |
|---|---|
| 1 hook silencieux, nom seul | 1 (et 4, bout en bout) |
| 2 `cmd bugfix` + scénario 1 | 3 (`a_command_attributes_the_run...`), 4 (bout en bout) |
| 3 scénario 2, `bugfix` et `evolution` | 3 |
| 4 un seul flow, chevauchement | 3 |
| 5 appel isolé puis commande ; agent `x` ignoré | 3 |
| 6 deux commandes | 3 |
| 7 compatibilité sans commande | 3 (`without_a_command...`, `a_command_naming_no_flow...`), suites existantes inchangées |
| 8 `link`/`unlink`, golden | 2 |
| 9 pages wiki | 2 (`link.md`), 4 (`flow-status.md`) |
| 10 essai réel, barrière, mutations | 4 (étape 5), chaque tâche (barrière, mutations), validator |

## Review Focus

1. **Silence du hook** : zéro octet sur stdout et sur stderr, code 0, sur entrée valide, `mcp_prompt`, JSON invalide, hors projet (tâche 1). Limite connue, hors lot : avec `RUST_LOG=debug` exporté, `tracing::debug!` dans `record_from_stdin` écrit sur stderr pour une entrée rejetée, comme pour les trois hooks existants ; la doc lue dit que stdout (pas stderr) entre dans le contexte.
2. **Aucun texte de prompt** : ni `command_args` ni `prompt` dans le journal (égalité exacte de la ligne).
3. **Compatibilité** : les tests existants de `runs.rs`, `render.rs`, `flow_status_cli.rs`, `record.rs` ne changent que par `interrupted: false` / `command: None` ; un vieux binaire saute une ligne `command` (variant inconnu, `read_log` ignore la ligne), un nouveau lit un vieux journal sans frontière.
4. **Lien tardif** : il ne laisse aucun lien « à prendre » (l'exécution finit) et n'empêche pas l'étape tardive de mener plus loin (scénario 2).
5. **Ordre et statut des blocs** : `interrupted` avant `finished` selon l'ordre de départ, `in progress` en dernier ; une exécution attribuée vide n'apparaît ni dans `flow status` ni dans `--list`.
6. **`settings.json`** : le 4e hook passe par la même fusion ; `unlink` avec manifeste le retire seul, sans manifeste ne touche pas le fichier ; un projet lié avant le reçoit au relink.

## Décisions de Dimitri

- **Exécution attribuée avant le premier sous-agent** (2026-10-26) : « Rien jusqu'au 1er agent (Recommandé) ». L'exécution n'apparaît qu'au premier sous-agent placé ; une session qui n'a que des commandes compte `0` dans `--list` (règle 6 de la tâche 3, état `masquee` du second diagramme).
- **Ligne de rapport de `link`** (2026-10-26) : « Nommer les 4 (Recommandé) ». La ligne unique nomme `SubagentStart, SubagentStop, PreToolUse (SubagentHandback), UserPromptExpansion` ; les 11 golden qui contiennent le rapport changent chacun d'une ligne (tâche 2).
- **`agent_id` vide** (2026-10-26) : « Ne rien faire (Recommandé) ». Limite connue, écrite dans `docs/wiki/flow-status.md` (tâche 4, étape 2).
- **Étape amont sautée dans une exécution attribuée** (2026-10-26) : « not run in this session (Recommandé) ». Marque `- … (not run in this session)` ; une étape aval pas encore lancée reste `○ pending` (règle 3 de la tâche 3).

## Questions pour Dimitri

Aucune.
