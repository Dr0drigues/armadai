# Retour d'expérience orchestration : marquage des runs, délégations, statistiques, barre d'état, étapes facultatives, tests de pack

Légende. **V** = vérifié au code du dépôt ou dans un fichier lu. **D** = lu dans la documentation officielle de Claude Code (avec la date). **C** = à confirmer par une mesure ou une lecture avant d'écrire du code : tout C devient une tâche de vérification du plan ou un critère d'essai.

## 1. Intention

Une session réelle de plusieurs jours (environ 5,7 jours, une trentaine de MR, 107 runs reconstruits, un journal de 2 742 lignes) menée avec une équipe décrite par ArmadAI et un pack, sous `policy.delegation: strict`, a fait ressortir les limites de l'outil. `flow status` attribue mal les verdicts quand plusieurs lots s'enchaînent (71 verdicts de revue comptés dans des runs d'un autre flow, 74 runs sur 88 « interrupted »), parce qu'un même agent tient plusieurs étapes et qu'un run ne se ferme qu'à la commande suivante. Le journal public de ce dépôt (équipe de développement d'ArmadAI, session de 15 jours, 92 runs reconstruits par `flow status --json`, mesuré le 2026-11-10) confirme le défaut sans pack privé : 80 runs `lot` pour 79 démarrages d'`analyst`, tous `in-progress`, dont 60 réduits à la seule étape `spec` ; 191 verdicts du relecteur de tâche dans le journal contre 11 rattachés à l'étape `guard` du flow `task` (plus 13 transitions de rejet), aucun run `task` après le 2026-10-26 ; 75 runs montrent `spec done` alors que l'agent n'a rendu `done` que 8 fois (l'état est déduit de l'arrêt). Dans ce journal les flows sont lancés par délégation, jamais par une commande tapée (0 ligne `UserPromptExpansion`) : l'attribution par commande ne sert à rien, seul un marqueur dans l'appel de délégation tient. Une autorisation donnée à l'oral (« valide les demandes jusqu'à l'ouverture de la MR ») ne vit que dans le contexte et se perd à la compaction. Aucune durée, aucun taux de rejet n'est agrégé. Avec `ui.status_line: true`, la barre du projet masque celle de l'utilisateur. Une étape qui n'a pas lieu reste « pending » pour toujours.

Dimitri a demandé (2026-11-10) un seul lot qui regroupe les recommandations, dans l'ordre « Marquage, grant, stats », avec cette idée d'origine : « dans les flows, il faudrait qu'on définisse ce qui peut bypass les règles asks, exemple, dans mes flows d'évolutions, j'aimerais les mettre en auto jusqu'à l'ouverture de la MR ». Pour la barre d'état : « Améliorer ArmadAI », plus « La raison du ?, L'agent de l'étape ». Pour la liste de tâches native de Claude Code : « essaie de voir si on peut utiliser cet ui dans nos packs […] ou en recréer une similaire ».

Source : le retour d'expérience de Dimitri (privé, hors dépôt). Les mesures citées ici viennent de ce retour (journal de flow d'une session réelle, méthode : durée = de `start` au premier `stop` d'un même `agent_id`) ; les faits sur le code ont été revérifiés le 2026-11-10 sur master `7c14af5`.

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs

1. **Marquage des runs.** Chaque délégation à un agent d'étape porte un marqueur `[flow/étape#lot]` dans la `description` de l'appel `Agent` ; le gate l'impose sous `strict`, `flow status` l'utilise pour placer l'étape et grouper par lot. Une ligne `REF:` facultative dans le handback porte une référence courte (branche, numéro de MR).
2. **Délégations nommées (`grant` / `revoke`).** Des niveaux déclarés par le projet ou le pack, activés pour une durée bornée par une étape et/ou une heure, qui passent des règles `ask` en `allow` et ne lèvent jamais un `deny`.
3. **`flow stats`.** Durées, appels, taux de rejet, boucles, par agent, par étape, par lot.
4. **Barre d'état enchaînée.** La barre écrite par `link` appelle celle de l'utilisateur puis ajoute la ligne du flow, sans la masquer. `--line` gagne l'agent de l'étape et la raison du `?`.
5. **Étapes facultatives.** `optional: true` sur une étape : `flow status` montre « sautée » au lieu de « pending ».
6. **Vue liste de tâches d'un flow** dans Claude Code, dessinée par le mod `armadai-ui` à partir du journal.
7. **`pack test`.** Lance les tests déclarés d'un pack dans une copie isolée.
8. **Équipe du dépôt (phase A).** Intégrer les propositions du retex de `lead` qui touchent `.armadai/` et la skill `armadai-rules` : lecture pour `analyst` et `architect`, relecture du plan, sortie « défaut local à une tâche » du flow `lot`, briefs par référence, citations de doc stockées une fois, hygiène outillée.

### Non-objectifs

- `wait:` (attente d'une condition externe), `resume:`, tier et effort par étape, multi-dépôt, mode `policy.delegation: ordered`, `params:` de flow, `flow board`, lecture de la CI. L'état réservé `pending` reste hors lot (décision 8), comme la consigne `CLAUDE.md` demandant au coordinateur de créer des tâches natives (`TaskCreate`), que la spec ne recommandait pas : guidance non mesurée, non retenue.
- Écrire dans la liste de tâches native de Claude Code : la doc présente `TaskCreate` comme un outil du modèle, les hooks `TaskCreated` / `TaskCompleted` ne font que bloquer, et l'API des mods n'offre aucun appel de tâche (D, 2026-11-10, `tools-reference`, `hooks`, `mods/reference` ; « non trouvé » dans la doc, pas « impossible »).
- Compiler le marquage, `grant`, la barre enchaînée ou la vue vers OpenCode, Codex et Copilot : lignes `not-compiled (Claude Code only)`.
- Un jeton ou un coût dans `flow stats` : le payload de `SubagentStop` n'en porte pas (D, 2026-11-10, `hooks#subagentstop`) ; l'usage n'existe que dans le `tool_response` du `PostToolUse` d'un appel `Agent` au premier plan. Hors lot.
- Des tests de permissions « statiques » qui réimplémenteraient les règles de Claude Code.
- Un modèle d'exécution : ArmadAI n'exécute rien, il compile et observe.

### Anti-objectifs

- Qu'un marqueur manquant fasse refuser une délégation sans dire la forme attendue, ou qu'un échec de compréhension du gate refuse (règle du gate : l'incertitude rend « pas d'avis »).
- Qu'un niveau de `grant` lève un `deny`, de quelque façon que ce soit.
- Que `link` ou le gate écrivent le texte de la description d'un appel ou celui d'une réponse dans le journal (seuls le marqueur et la `REF:` bornée).
- Que la barre enchaînée ralentisse Claude Code ou perde la barre de l'utilisateur quand la commande de celle-ci échoue.
- Qu'`armadai` exécute des scripts d'un pack ailleurs que dans la commande explicite `pack test`.
- Que le dépôt public reçoive un contenu privé (journal brut, nom de pack, chemin de machine).

## 3. Contraintes

- **Rien de privé** dans le dépôt public : spec, plan, wiki, fixtures. Les journaux de rejeu sont anonymisés : agents, flows et lots à noms génériques, aucun chemin, aucun texte libre. Un test échoue si une fixture contient `/Users/` ou un nom absent de l'allowlist générique.
- **Aucun paquet nouveau au lock** sans décision de Dimitri. Les durées, médianes, la copie isolée et l'exécution de scripts se font avec la bibliothèque standard et les crates déjà présents ; une dépendance déjà au lock mais non déclarée par `crates/armadai` peut être déclarée sans paquet nouveau.
- **`link` hors réseau** (comme aujourd'hui) ; `pack test` ne télécharge rien : il lit le cache déjà synchronisé ou un dossier local.
- **Formats natifs vérifiés** dans la documentation officielle avant d'être écrits, avec URL et date dans le code et le wiki : `statusLine` (type, command, refreshInterval, entrée sur stdin), `permissions` et leur précédence, `settings.local.json`, hooks (`PreToolUse`, `SubagentStart`, `SubagentStop`), mods (`AbovePrompt`, `Pane`, `Text`). Déjà lus : `statusLine` et `permissions` deny d'abord (V dans `compile/claude_settings.rs` et `docs/wiki/link.md`, 2026-10-28 et 2026-11-05) ; mods (`docs/wiki/flow-status.md`, 2026-11-09). Lus le 2026-11-10 (D) : `settings.local.json` (`.claude/settings.local.json`, plus prioritaire que `settings.json`, listes `permissions` fusionnées entre portées, relu à chaud) ; le champ `description` du `tool_input` de l'outil `Agent` ; mods : `Pane` et `AbovePrompt` sont des sites de rendu (`AbovePrompt` : Terminal et Desktop), `Text` est un élément d'un arbre rendu, `$.fs.read`, `$.process.run` (liste d'arguments, sans shell, 30 s) et `$.clock.every` existent, la page de référence décrit la version 2.1.290. Pas de fusion décrite entre la `statusLine` du projet et celle de l'utilisateur : la règle générale « la valeur du niveau le plus haut » s'applique (inférence, pas citation). **C** : le champ `if` d'un hook, l'effet d'un `allow` de hook sur une règle `ask`, les propriétés exactes d'`AbovePrompt`.
- **Opt-in** : une clé de compilation absente de `project.yaml` laisse `link` sans écriture sur ce point ; en conflit avec une valeur de l'utilisateur, `link` refuse et rapporte `not-compiled` avec la raison. Les niveaux de `grant` n'écrivent rien tant qu'on n'a pas lancé `grant`.
- **Versions** : le manifeste `link-manifest.yaml` reste en `version: 1` (champs optionnels seulement) ; `flow status --json` reste `armadai.flow-status/1` (champs ajoutés seulement, la doc dit « read the keys you need ») ; `flow stats --json` naît `armadai.flow-stats/1` ; les journaux `.jsonl` gagnent des champs et un genre de ligne, jamais de renommage ; un test lit un document écrit par la version précédente.
- Codes de diagnostic en kebab-case, ajoutés à la table de `docs/wiki/project-format.md` ; texte de toute ligne nouvelle de sortie proposé par le plan et validé par Dimitri ; golden relus ligne à ligne.
- `armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars) ; `cargo build -p armadai-core` et `-p armadai-model` passent ; schémas JSON régénérés ; wiki mis à jour (`flow-status.md`, `policy-gate.md`, `link.md`, `project-format.md`, `starter-packs.md`).
- Mutations vues rouges pour chaque test ajouté ; essais du binaire avec `HOME` et `ARMADAI_CONFIG_DIR` isolés.

## 4. Conception

### État du code (V, master `7c14af5`)

- Le gate (`crates/armadai/src/claude_adapter/policy_gate.rs`, `decide`) lit `tool_name`, `tool_input.subagent_type`, `agent_type`, `cwd` ; il ne lit pas `tool_input.description`, que le payload porte pourtant (fixture de test du gate : `tool_input: {description, prompt, subagent_type}`).
- Le journal (`flow_status/record.rs`) : `FlowEvent { ts, event: start|stop|command, agent_id, agent_type, state, command }` ; les événements `SubagentStart` n'ont ni description ni marqueur ; une étape se place par `agent_type` (`runs.rs`), d'où l'ambiguïté d'un agent qui tient plusieurs étapes.
- Le modèle (`armadai-model/src/types.rs`) : `Step { agent, states, on_failure }` ; `Policy { delegation, free_agents }` ; `Ui { status_line, subagent_status_line, panel, documents }` (booléens). Aucun `optional`, aucun `grants`.
- Permissions : `native.claude.permissions` est fusionné liste par liste dans `settings.json` (`Owned.permissions`, appartenance à l'entrée près, `deny_over_allow` signalé à l'identique seulement). Aucun fichier `settings.local.json` n'est écrit ni lu par le code (aucune occurrence dans `crates/`).
- `statusLine` : `ui.status_line: true` écrit `{"type":"command","command":"armadai __claude-statusline","refreshInterval":5}` (`status_line_value`) ; une barre existante qui n'est pas la nôtre est gardée, `not-compiled` (`keep_users_status_line`). La barre de niveau utilisateur n'est pas lue : la barre du projet la masque (précédence managed > CLI > `settings.local.json` > `settings.json` > `~/.claude/settings.json`, D, 2026-11-05).
- `--line` (`flow_status/line.rs`) : flow, état (`finished`/`interrupted`), `●` étape courante, `d/n`, `(loops k)` ; ni agent de l'étape ni note du `?`.
- `pack.yaml` est lu par `StarterPack` (`armadai-core/src/starter.rs`, sans `deny_unknown_fields`, champs `name`, `description`, `format`, `agents`, `prompts`, `skills`) ; `PackCommand` n'a ni `Test` ni champ `tests`.
- Mesure du dépôt sur la prose : le wiki du gate rapporte « 9 délégations sur 520 » reçues par le coordinateur déclaré, sans gate. **Aucune mesure du dépôt ne porte sur le respect d'une consigne de description d'appel** : c'est l'hypothèse qui justifie d'imposer le marqueur par le gate plutôt que par une consigne seule, et un essai réel la vérifie (critère 1).

### Phase 1. Marquage (`armadai`, `armadai-model`, `compile/`)

**Marqueur.** Début de `tool_input.description` : `[<flow>/<étape>#<lot>]` ; `#<lot>` facultatif (décision 1). `flow`, `étape` et `lot` suivent `is_valid_id` (lettres, chiffres, `.`, `_`, `-`), le lot étant limité à 40 caractères. Exemple : `[evolution/review#menu-refonte] relire la branche`.

**Gate (sous `strict`).** Pour un appel dont la cible tient au moins une étape d'un flow, hors agents libres :

- le marqueur doit nommer un flow du projet, une étape de ce flow et que l'agent de cette étape est la cible ; sinon refus, avec le message qui donne la forme et les étapes que tient la cible, par exemple `the description of a delegation to reviewer must start with [flow/step#lot], with <flow>/<step> among: evolution/review, mr/revue-mr` ;
- la topologie (`allowed_targets`) est jugée comme aujourd'hui ; le marqueur ne la remplace pas ;
- un agent libre (`free_agents`) ou un agent sans étape n'a pas besoin de marqueur ;
- l'incertitude reste « pas d'avis » : charge illisible, projet invalide ou `off` ne changent pas (le refus sur marqueur manquant est décidé : décision 2).

**Enregistrement.** Le recorder (`__record-flow-event`) gagne un hook asynchrone `PreToolUse` sur `Agent|Task` : il écrit une ligne `{"ts","event":"mark","agent_type","flow","step","lot"}` avec le seul marqueur lu (jamais le texte de la description). Il fonctionne sans `strict` ; les consignes du bloc coordinateur de `CLAUDE.md` (la forme du marqueur, la ligne `REF:`) sont émises dès que le projet a un flow, sur Claude Code ; sur les autres cibles : `not-compiled`. **C** : le lien entre une ligne `mark` et le `SubagentStart` qui suit (par `tool_use_id` si `SubagentStart` le porte, sinon la première `start` non marquée du même `agent_type` après la `mark`, dans l'ordre du journal) ; à mesurer avec des appels parallèles du même agent (risque 2).

**Attribution (`runs.rs`).** Un `start` marqué est placé à l'étape et au flow du marqueur, dans le run du couple (flow, lot) ; le run de ce couple se ferme à sa fin naturelle et non à la commande suivante ; un `start` non marqué suit les règles actuelles. Sans aucun marqueur, la sortie est octet pour octet celle d'aujourd'hui (golden inchangés). `--json` ajoute `runs[].lot` (chaîne ou `null`) ; le texte et le Markdown groupent par lot (`deliver #menu-refonte  in progress`).

**`REF:`.** Une ligne `REF: <mot>` en fin de handback, à côté de `STATE:` : lue avec la même règle (dernière ligne exacte), bornée à 64 caractères de `[A-Za-z0-9._/#-]`, sinon ignorée ; enregistrée comme champ `ref` de la ligne `stop`, montrée dans `flow status` et `--json` (`steps[].ref`). Cela élargit d'un cran la règle « rien d'autre n'est enregistré » du wiki ; Dimitri l'a accepté (décision 3).

**Reprise d'un agent.** Une reprise par `SendMessage` garde l'`agent_id` et n'a pas de nouvel appel `Agent` : elle ne porte pas de marqueur (181 des 299 appels du journal public mesuré sont des reprises). Un `start` d'un `agent_id` déjà marqué hérite de son dernier marqueur. **C** : un hook sur `SendMessage` pour marquer une reprise vers une autre étape ; sans lui, une reprise reste dans l'étape de l'appel d'origine.

**Forme réelle du journal (V, mesure du 2026-11-10).** Une invocation écrit 1 `start` et 2 `stop` dans la même seconde, l'état n'étant que sur le premier `stop` ; les `stop` du fil principal n'ont ni type ni `start` (3 253 sur 4 707). L'attribution, les statistiques et les rejeux comptent des appels par `agent_id` démarré, jamais des `stop`.

**Rejeu de journaux.** Un dossier `tests/fixtures/flow-replay/` de journaux `.jsonl` anonymisés, chacun avec le `flow status --json` attendu ; un test d'intégration rejoue le journal et compare le JSON exact. Cas : deux lots entrelacés du même flow ; un agent qui tient deux étapes de deux flows ; un handback sans `STATE:` ; des extraits courts (quelques dizaines de lignes) du journal public de ce dépôt (aucun texte libre : identifiants, types d'agent, états ; identifiants de session et d'agent remplacés) où des verdicts du relecteur de tâche tombent dans le run du mauvais flow, et où chaque reprise ouvre un run, avec le résultat attendu après marquage ajouté à la main. Un extrait n'est retenu qu'après relecture : rien de privé.

### Phase 2. `grant` / `revoke` (`armadai-model`, `compile/`, `cli/`)

**Niveaux.** Déclarés dans `project.yaml` (donc aussi dans un pack, fusionnés par la règle du pack) sous `policy.grants` (décision 4) :

```yaml
policy:
  grants:
    auto-until-mr:
      description: "Pousser les branches de travail et ouvrir la MR en brouillon sans demande"
      allow: ["Bash(git push origin feature/*)", "Bash(glab mr create --draft *)"]
      until: publish        # facultatif : étape par défaut, forme <étape> ou <flow>/<étape>
```

Les clés sont des identifiants ; `allow` est une liste de règles de permission au format natif (opaques pour ArmadAI, comme `native.claude.permissions`).

**Commandes.** `armadai grant <niveau> [--until <étape>] [--for 2h | --until-time 18:00]` et `armadai revoke [<niveau>|--all]` ; `armadai grant` sans argument liste les niveaux et ceux qui sont actifs, avec leur borne. Une borne est obligatoire (celle de la commande, sinon celle du niveau) : pas de niveau sans fin.

**Fait établi qui oriente le mécanisme (D, 2026-11-10, `code.claude.com/docs/en/permissions` et `settings`).** Claude Code évalue les règles dans l'ordre `deny`, `ask`, `allow`, « the first match in that order determines the outcome », quelle que soit la portée : une règle `allow` ajoutée dans `.claude/settings.local.json` **ne peut pas lever** une règle `ask` posée par `settings.json` (le cas des règles de MR du pack). Écrire des `allow` dans `settings.local.json` ne sert donc que pour les appels que rien ne met en `ask`. Les settings sont relus à chaud (D : « reloads them when they change », y compris `permissions` et `hooks`).

**Mécanisme retenu (hypothèse C, mesurée en première tâche de la phase).** Un hook `PreToolUse` `armadai __claude-grant`, installé par `link` seulement si le projet déclare des niveaux, avec une entrée par règle de niveau et le champ `if` du hook, au format des règles de permission, pour que Claude Code fasse la correspondance et qu'ArmadAI ne réimplémente rien. Le hook rend `permissionDecision: "allow"` quand un grant actif couvre la règle et que sa borne n'est pas échue, sinon aucun avis. Aucun fichier `settings.local.json` n'est écrit. L'état (niveau, bornes, heure d'activation, étape de départ) est dans `.armadai/state/grants.yaml` (déjà ignoré par `.armadai/.gitignore`) ; `revoke` le vide, `unlink` retire les hooks et l'état. **À mesurer avant d'écrire du code** : (a) le champ `if` d'un hook existe et s'applique à `PreToolUse` ; (b) un `allow` rendu par un hook passe une règle `ask` ; (c) il ne passe jamais une règle `deny`. Si (b) est faux, le mécanisme n'existe pas sous cette forme : le plan s'arrête et Dimitri choisit (par exemple écrire les règles `ask` du niveau dans `settings.local.json` plutôt que `settings.json`, ce que `grant` réécrirait).

**Un `deny` n'est jamais levé.** À la validation, un niveau dont une règle est égale, caractère pour caractère, à un `deny` de `native.claude.permissions` (projet ou pack) est une erreur (`grant-over-deny`) ; `grant` refuse de même contre les `deny` de `settings.json` et `settings.local.json`. Claude Code applique `deny` en premier (D) : le refus protège de l'illusion. Un recouvrement par motif (`Bash(git *)` contre `Bash(git push:*)`) n'est pas vu, et la doc le dit (risque 4).

**Expiration, vérifiée à chaque appel.** Le hook lit l'état à chaque appel visé (un `stat` et une lecture quand un grant est actif, rien sinon) : l'heure (`--for`, `--until-time`) est comparée à l'horloge, et la borne `--until <étape>` est atteinte quand le journal contient, depuis l'activation, un appel marqué (phase 1) vers cette étape. Le grant tombe donc **avant** que l'étape ne tourne, sans balayage ni fenêtre de relecture. Cette borne suppose le marquage : sans marqueur enregistré, `--until <étape>` est refusé par `grant` avec la raison. Un état illisible rend « aucun avis », jamais « allow ».

### Phase 3. `flow stats` (`cli/flow.rs`, `flow_status/stats.rs`)

`armadai flow stats [<flow>] [--session <id> | --all] [--by agent|step|lot] [--json]`. Par groupe : appels, durée médiane, maximale, totale (de `start` au premier `stop` d'un même `agent_id` ; un agent repris par `SendMessage` garde son identifiant et son second `stop` n'est pas compté : minorant, écrit dans la sortie), répartition des états rendus, taux de rejet (états `rejected` sur états rendus de l'étape qui en déclare un), boucles (liens pris plus d'une fois). `--by agent` ne dépend pas du marquage ; `--by step` et `--by lot` n'ont de sens que sur les `start` marqués : les autres sont comptés à part (« n appels non marqués ») et, sans aucun marqueur, la commande l'écrit en une ligne, code 0. La sortie `--json` porte `schema: "armadai.flow-stats/1"`. Sans journal : sortie vide valide, code 0. Aucun texte de réponse n'est lu hors du mot `STATE`.

### Phase 4. Barre d'état, étapes facultatives, vue

**Barre enchaînée.** `ui.status_line: true` écrit toujours la `statusLine` du projet ; `__claude-statusline` lit à l'exécution la `statusLine` de niveau utilisateur (`~/.claude/settings.json`, hors `HOME` isolé : celui de l'essai), exécute sa commande avec le même JSON sur stdin (délai borné à 1 s, sa sortie lue en entier), imprime sa sortie puis la ligne du flow sur une ligne suivante (une ligne par `echo`, D 2026-11-05). Échec, délai ou barre absente : la ligne du flow seule, code 0. La lecture se fait à l'exécution et non au `link` : `link` ne copie donc pas la commande de l'utilisateur dans un fichier versionné, et la barre suit les changements de l'utilisateur. Ce changement de comportement (aujourd'hui la barre du projet remplace l'autre) est décidé (décision 5). Le rapport de `link` dit `compiled (chained after your user status line when there is one)`. Le cas « une `statusLine` du projet qui n'est pas la nôtre » reste `not-compiled`.

**`--line`.** Ajoute entre parenthèses l'agent de l'étape et, pour un `?` ou un `✗`, la note : `deliver ● guard (qa) 2/4`, `deliver ? guard (qa: no STATE line) 2/4`. `StepView` gagne `agent`, exposé par `--json` (`steps[].agent`). Texte exact validé par Dimitri au plan.

**`optional: true`.** `Step.optional` (booléen, défaut `false`, absent de l'écriture canonique par défaut ; schéma régénéré). Une étape facultative jamais démarrée est `skipped` (marque `⤼`, JSON `mark: "skipped"`, `sign: "⤼"`, note « skipped ») quand une étape qu'elle atteint par les liens a démarré ou que le run est fini ; tant que rien n'a démarré après elle, elle reste `pending`. Une étape facultative en attente ne maintient pas le run ouvert. Texte, Markdown, `--watch` (dessin ASCII) et `--line` (qui ne compte pas les `skipped`) suivent. Une étape facultative ne peut pas être la première d'un flow (code `optional-start-step`, nom à valider). L'état `skipped` est une valeur de plus du champ `mark` : le mod du panneau (lecteur de `armadai.flow-status/1`) est mis à jour dans le même lot et le wiki le dit (risque 5). Le bloc coordinateur de `CLAUDE.md` marque l'étape « facultative ».

**Vue liste de tâches.** Une bande dans le mod `armadai-ui` existant, sous une clé opt-in (nom au plan, par exemple `ui.flow_band`), lue depuis `panel.json` ; elle est tirée de `flow status --json` toutes les 2 s comme le panneau : une ligne `✓ design  ✓ build  ■ guard  □ validate  ⤼ test` pour le run ouvert, placée au-dessus du prompt (`AbovePrompt`)  (décision 6, bande plutôt que liste verticale ; pas de consigne `TaskCreate`). `AbovePrompt` existe (D, 2026-11-10) ; **C** : ses propriétés, sa place avec le panneau (un `Pane` hors terminal large se replie en bande au-dessus du prompt), la version minimale (le panneau annonce 2.1.318) ; relues dans `code.claude.com/docs/en/plugins/mods/reference` et `interface` par l'adapter-specialist avant d'écrire le JavaScript, puis testées avec `claude plugin test` ; ligne de rapport `approximated`, comme le panneau. Une ligne sous le prompt existe aussi (`$.ui.status`) : variante à comparer dans l'essai.

### Phase 5. `pack test` (`armadai-core`, `cli/pack.rs`)

`pack.yaml` gagne `tests: [tests/permissions.sh, …]` (liste facultative de chemins relatifs au pack, sans `..` ni chemin absolu ; sinon erreur, code à nommer au plan). `armadai pack test [<dossier de pack>]` : sans argument, le pack du projet (snapshot verrouillé du cache, sans réseau) ; avec un dossier, ce dossier. Le pack est copié dans un dossier temporaire ; chaque test est lancé avec `bash <script>` dans cette copie, avec `HOME`, `CLAUDE_CONFIG_DIR` et `ARMADAI_CONFIG_DIR` pointant vers des dossiers temporaires, un délai borné par test (défaut à fixer au plan) ; une ligne `ok` ou `FAIL` par test, le code retour et, en échec, la sortie ; code de sortie non nul si un test échoue ; aucun test déclaré : message, code 0. Aucune écriture hors des dossiers temporaires. Aucune exécution à `link`, `init` ou `pack sync`. Pas de test de permissions déclaratif en statique.

### Phase A. L'équipe de ce dépôt (`.armadai/`, skill `armadai-rules`)

Décision de Dimitri (2026-11-10) : « intègre les propositions du retex dans le lot #12 ». Le retex de lead (équipe du dépôt, session du 2026-11-04 au 2026-11-10) donne six propositions qui ne touchent pas l'outil mais l'équipe ; sa septième recoupe le marquage et `flow stats` (déjà au lot). Cette phase n'écrit aucun code Rust : elle édite des fichiers de `.armadai/` (domaine du `framer` et du `schema-specialist`) et la skill `armadai-rules` (tenue par le `framer` avec Dimitri ; après chaque édition, `armadai link --target claude`). Chacune change une façon de travailler ; Dimitri a tranché (décisions 10 à 14).

- **A1. Lecture pour `analyst` et `architect`.** Aujourd'hui `capabilities: { shell: deny }` (V) : ni recherche de texte, ni `git`. Mesuré par lead : une hypothèse corrigée par lead, une phrase de plan fausse, « 0 `/Users/` » déclaré sans pouvoir compter, des « non vérifié » reportés. Décidé : les laisser appeler `Explore` (lecture seule par construction ; `capabilities.delegate` ouvert pour ces deux agents, `Explore` étant déjà agent libre du projet, à vérifier au plan) ; pas de shell, les permissions de Claude Code étant par projet et non par agent.
- **A2. Relire le plan.** « Rien de privé » et « le plan est relu » dans la checklist du `guardian` (mesuré : 6 chemins privés trouvés dans un plan par le `reviewer`, après trois revues de tâche qui ne lisaient pas le plan). Décidé : la checklist, sans étape de plus dans `lot`.
- **A3. Défaut local à une tâche.** Aujourd'hui un rejet en `validate` ou `review` repasse par `plan` (mesuré : deux révisions de plan pour une correction de test et une de wiki). Les liens d'un flow ne visent que des étapes du même flow (V : `unknown-step`, `crates/armadai-model/src/check.rs`, boucle des liens), et dans `lot` les tâches ne sont qu'une phrase de description, aucune étape : un lien `validate → build` du flow `task` est impossible. Décidé : une étape `tasks` tenue par le coordinateur dans `lot` (une étape de coordinateur est montrée `~` par `flow status`, sans état), `plan → tasks`, `tasks → review`, `review → tasks when rejected-task`, `validate → tasks when rejected-task`, en gardant `rejected → plan` pour un défaut de conception ; `validate` et `review` déclarent l'état `rejected-task` en plus.- **A4. Briefs par référence.** Un registre du lot (fichier d'état tenu par `lead`, lu par chaque agent) au lieu de recopier décisions, état de la branche et règles dans chaque brief. Décidé : `.armadai/state/ledger-<lot>.md`, local et ignoré par git (dossier `state/`), donc sans risque de contenu privé dans le dépôt public.
- **A5. Une citation de doc officielle, une fois.** Règle d'`armadai-rules` : le premier lecteur écrit dans le plan l'extrait exact (URL, phrase, date) ; les suivants vérifient l'extrait, non la page (mesuré : la page relue par quatre agents).
- **A6. Hygiène outillée.** Script de la skill `armadai-rules` : copie jetable sous un préfixe unique, suppression par chemin littéral ; le `guardian` vérifie `git status --ignored` en fin de tâche (mesuré : un dossier de cible oublié dans le worktree, des journaux `/tmp` non supprimés par motif). **C** : que `link` recopie les fichiers annexes (scripts) d'une skill vers `.claude/skills/` ; à vérifier au plan.

La phase A passe en premier : elle outille le reste du lot (un `analyst` et un `architect` qui peuvent compter, un plan relu) et n'attend aucun morceau de l'outil.

### Phasage

Un seul lot, un plan, six phases : **A équipe du dépôt** (en tête), puis dans l'ordre de Dimitri **P1 marquage**, **P2 grant**, **P3 stats**, puis les restes **P4 barre + facultatif + vue**, **P5 pack test**. P2 dépend de P1 pour `--until <étape>` seulement ; P3 ne dépend de P1 que pour `--by step|lot` ; P4 et P5 sont indépendantes. Chaque phase finit barrière verte et utilisable seule ; l'essai réel de Dimitri se joue à la fin de P1 (la mesure qui valide l'hypothèse du gate) et à la fin de P4 (ce qu'il voit). Deux PR (décision 7) : A et P1 à P3 d'abord, puis P4 et P5.

## 5. Critères d'acceptation

1. Sous `policy.delegation: strict`, un appel `Agent` vers un agent qui tient une étape, dont la description ne commence pas par un marqueur valide, est refusé avec un message qui donne la forme `[flow/étape#lot]` et les étapes de cet agent ; le même appel avec `[evolution/review#menu]` passe ; un agent libre passe sans marqueur. Essai réel : sur une session jouée par Dimitri, le coordinateur marque ses appels (ou le gate le lui fait corriger) et le journal le montre.
2. Le journal d'une session marquée contient des lignes `mark` qui ne portent que flow, étape et lot (aucun texte de description) ; sur un journal rejoué où un même agent tient une étape dans deux flows, `flow status --json` place chaque appel dans l'étape de son marqueur ; sur les journaux existants sans marqueur, la sortie texte et JSON est identique à celle d'avant le lot ; sur l'extrait rejoué du journal public où des verdicts de relecteur tombaient dans le run d'un autre flow, une fois marqué, chaque verdict est dans l'étape `guard` du run `task`, et une reprise d'agent n'ouvre pas de run.
3. Deux lots du même flow entrelacés dans une session donnent deux runs distincts, chacun avec `lot` dans `--json` et dans l'entête du texte, et aucun n'est marqué `interrupted` par le démarrage de l'autre ; un handback qui finit par `STATE: approved` puis `REF: feature/menu` montre `feature/menu` pour l'étape dans `flow status` et `steps[].ref`, et une `REF:` de plus de 64 caractères ou avec un caractère hors liste est ignorée.
4. Dans une session réelle où une règle du pack met `git push` d'une branche de travail en `ask`, après `armadai grant auto-until-mr --until publish` cet appel passe sans demande ; après `armadai revoke`, il redemande ; `armadai unlink` retire les hooks du niveau ; un niveau dont une règle est égale à un `deny` du projet est refusé à `validate` (`grant-over-deny`) et par `grant`, et un appel couvert par un `deny` reste refusé sous un grant actif.
5. Un grant `--for 1h` dont l'heure est dépassée (horloge injectée en test) ne donne plus d'avis au prochain appel ; un grant `--until publish` ne donne plus d'avis dès qu'un appel marqué `[…/publish#…]` a été enregistré, avant que cette étape ne tourne ; un état de grant illisible ne donne jamais `allow`.
6. `armadai flow stats --by agent` sur le journal de rejeu donne, pour chaque agent, le nombre d'appels, la médiane, le maximum, le total, les états rendus et le taux de rejet attendus (valeurs exactes dans le test) ; `--json` porte `schema: "armadai.flow-stats/1"` ; `--by lot` sur un journal sans marqueur dit en une ligne qu'aucun appel n'est marqué, code 0.
7. Avec `ui.status_line: true`, `armadai link --target claude` écrit la `statusLine` du projet, et la barre affichée par Claude Code montre d'abord la sortie de la barre de niveau utilisateur puis la ligne du flow ; si la commande de l'utilisateur échoue ou dépasse 1 s, seule la ligne du flow s'affiche, code 0 ; `armadai flow status --line` sur un run dont l'étape courante est tenue par `qa` affiche `qa`, sur une étape `?` sans `STATE:` il affiche la raison, et le JSON porte `steps[].agent`. Essai réel de Dimitri avec sa propre barre.
8. Une étape `optional: true` jamais démarrée est montrée `⤼ skipped` (texte, `--json`, Markdown, `--watch`) dès qu'une étape qu'elle atteint a démarré, ne retient pas le run ouvert, et une étape facultative en première position est refusée à `validate`.
9. Avec la clé de la vue, `armadai link --target claude` écrit la donnée du mod (rien sans la clé, diff nul) et Claude Code affiche au-dessus du prompt une bande `✓/■/□/⤼` qui suit le run ouvert, essai réel de Dimitri (ligne de rapport `approximated`).
10. `armadai pack test` sur un pack qui déclare deux tests dont un échoue affiche `ok` et `FAIL` avec le nom et la sortie du second, sort avec un code non nul, n'a écrit hors de ses dossiers temporaires ni `~/.claude` ni `~/.config/armadai` ; un chemin de test hors du pack est refusé ; `link` n'exécute aucun test.
11. (Phase A, flow `lot` de ce dépôt.) Après la modification du flow `lot`, `armadai validate` ne rend aucune erreur, `armadai graph lot --format mermaid` montre un lien étiqueté `rejected-task` de `validate` vers l'étape des tâches et un lien `rejected` de `validate` vers `plan`, et `armadai flow status` sur un journal rejoué où `validate` rend `STATE: rejected-task` montre le run revenir à l'étape des tâches, pas à `plan`.
12. (Phase A, équipe et règles de ce dépôt.) Un `analyst` lance en session réelle une recherche de texte et un `git log` en passant par `Explore`, sans que `lead` intervienne (gate non refusé) ; la checklist du `guardian` contient « rien de privé » et la relecture du plan ; la skill `armadai-rules` contient la règle des extraits de doc cités une fois et le script d'hygiène (copie jetable sous préfixe unique, suppression par chemin littéral), et `armadai link --target claude` laisse `.claude/skills/` à jour (diff vide à la relance).

## 6. Risques

1. **Le modèle ne marque pas, ou mal.** L'hypothèse (le gate suffit là où la prose échoue) n'est pas mesurée dans le dépôt. Signe : refus répétés pour marqueur manquant dans une session. Parade : message de refus qui donne la forme, essai réel du critère 1 avant de généraliser ; sinon repli sur « avertir sans refuser » (alternative écartée par la décision 2, à rouvrir avec Dimitri si la mesure est négative).
2. **Corrélation `mark` ↔ `SubagentStart`.** Deux appels parallèles du même agent peuvent être inversés (C). Signe : étapes permutées dans le rejeu. Parade : mesurer si `SubagentStart` porte `tool_use_id` ; sinon, ordre du journal et `?` « ambiguous » plutôt qu'un choix silencieux.
3. **Le mécanisme de `grant` peut ne pas exister.** Un `allow` de `settings.local.json` ne lève pas un `ask` (D) ; le hook avec `if` et son `allow` sur un `ask` sont des hypothèses (C). Signe : le critère 4 échoue en essai réel. Parade : mesure en première tâche de la phase, arrêt du plan et question à Dimitri si elle est négative, sans livrer un `grant` qui ne contourne rien. Hors cette hypothèse, un hook qui plante ne doit jamais donner `allow` (critère 5), et une expiration est vérifiée à chaque appel, pas seulement au démarrage.
4. **Un niveau trop large.** Une règle qui recouvre un `deny` par motif n'est pas vue. Parade : `deny` d'abord (D), niveaux courts et exacts dans le pack, mesure au banc de permissions du pack (hors dépôt).
5. **Schéma `flow-status/1` et lecteurs.** Une valeur de `mark` nouvelle (`skipped`) et de nouvelles clés peuvent surprendre un lecteur strict. Parade : mod du panneau mis à jour, wiki, test de lecture d'un document de l'ancienne version.
6. **Barre enchaînée.** Exécuter la commande de l'utilisateur à chaque rafraîchissement double le coût ; une barre lente ralentit Claude Code. Parade : délai de 1 s, `refreshInterval` inchangé, mesure de la durée ajoutée.
7. **Mod instable.** L'API des mods bouge ; `AbovePrompt` est peu documenté (C). Parade : opt-in, ligne `approximated`, lecture de la doc avant le JavaScript, `claude plugin test`.
8. **Lot trop gros.** Six phases (la phase A comprise), quatre crates, des golden à relire, des fichiers `.armadai/` de l'équipe. Parade : phases indépendantes qui finissent vertes, revue de branche relue par phase, deux PR décidées (A et P1 à P3, puis P4 et P5).
11. **La phase A change la façon de travailler de l'équipe en cours de lot.** Modifier `lot`, les permissions ou les briefs pendant que ce lot tourne peut casser le gate de l'équipe elle-même (un agent refusé). Parade : la phase A passe en premier, chaque édition est suivie de `armadai link --target claude` et d'un `armadai validate`, et une édition d'`armadai-rules` passe par `framer` et Dimitri.
9. **Journaux de rejeu privés.** Un journal réel contient des noms de projet. Parade : anonymisation par construction (noms génériques) et test qui refuse `/Users/` et les noms hors allowlist.
10. **Exécuter le code d'un pack.** `pack test` lance des scripts d'un pack éventuellement distant avec les droits de l'utilisateur. Parade : commande explicite seulement, copie isolée, `HOME` temporaire, dit dans le wiki ; pas d'isolation réseau (non promise).

## Décisions de Dimitri (2026-11-10)

Réponse, mot pour mot : « Toutes les recos (Recommandé) ». Les 14 questions posées sont tranchées selon la recommandation :

1. Marqueur `[flow/étape#lot]`, le lot est facultatif.
2. Un marqueur manquant entraîne un refus, avec un message qui donne la forme attendue.
3. `REF:` enregistré (64 caractères au plus, alphabet restreint).
4. Niveaux de `grant` dans `policy.grants` de `project.yaml`, fusionnés par le pack.
5. `ui.status_line: true` s'enchaîne par défaut après la barre de l'utilisateur.
6. Une bande `✓ ■ □` au-dessus du prompt ; pas de consigne `TaskCreate` (hors lot).
7. Deux PR : la phase A et P1 à P3, puis P4 et P5.
8. `pending` reste hors lot.
9. La phase A va avec le reste du lot, en premier.
10. `analyst` et `architect` peuvent appeler `Explore`.
11. La checklist du `guardian` ajoute « rien de privé » et « plan relu ».
12. Une étape `tasks` dans `lot`, avec l'état `rejected-task`.
13. Registre du lot dans `.armadai/state/ledger-<lot>.md`.
14. A5 et A6 (citations de doc stockées une fois, hygiène outillée) vont dans `armadai-rules`.

## Questions ouvertes

Aucun choix produit n'est en suspens. Restent les points **C** (mesures et lectures de doc à faire en première tâche de leur phase) : corrélation `mark` / `SubagentStart`, champ `if` d'un hook et `allow` de hook contre une règle `ask` (si ce dernier échoue, le plan s'arrête et Dimitri choisit), propriétés d'`AbovePrompt`, hook `SendMessage` pour marquer une reprise, recopie des scripts d'une skill par `link`, `Explore` agent libre du projet.
