# Étape 4, lot B1 : commandes et prises (slots) dans le modèle canonique

## 1. Intention

Le lot B livre un pack « du concept au code propre » dont l'utilisateur pilote une partie par des commandes explicites : `/mockup <sujet>`, `/iterate <retour>`, `/brief <idée>`. Aujourd'hui le modèle d'ArmadAI ne connaît que les agents, les skills, les flows et les automations : il n'a aucun moyen de déclarer une commande. B1 ajoute cet élément au modèle, une fois pour tous les packs, et le compile vers les deux cibles fidèles (Claude Code, OpenCode 2.x). B2 (le pack) s'appuie dessus.

B1 ajoute aussi les **prises** (slots) : un pack doit pouvoir être complété par un autre (normes de code, docs d'infra, sécurité, guides UX) sans qu'on réécrive ses agents. Aujourd'hui un agent ne connaît que les skills nommées dans son `skills:` ; une skill de normes ajoutée plus tard lui est invisible, et sous OpenCode elle est même bloquée par `permission.skill` (`"*": deny`). Avec une prise, l'agent déclare ce qu'il consomme (`slots: [code-standards]`), la skill déclare ce qu'elle fournit, et `link` fait le raccord. Le lot C (packs de normes) livre des packs qui ne contiennent que des skills branchées sur ces prises.

Une commande est un **raccourci que l'utilisateur tape**. Elle tourne dans la conversation principale, pour garder l'interaction avec l'utilisateur (questions à choix, validation). La méthode longue vit dans une skill, que la commande charge ; la skill peut aussi être déclenchée par le modèle quand l'utilisateur formule la demande en langage naturel.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs :
1. `.armadai/commands/<id>.md` : frontmatter (`description` obligatoire, `argument_hint` facultatif, `native` facultatif comme pour un agent), corps = texte de la commande avec `$ARGUMENTS`.
2. Chargement strict avec diagnostics positionnés, validation sémantique à codes stables, écrivain canonique, schéma JSON régénéré.
3. `link` : Claude → `.claude/commands/<id>.md` ; OpenCode → `.opencode/commands/<id>.md` ; Codex, Copilot → ligne `not-compiled`. `unlink` retire ce que `link` a écrit (manifeste).
4. Une ligne de rapport de conformité par commande et par cible.
5. `armadai new command` (et entrée du menu guidé), `list`, `inspect` affichent les commandes.
6. Prises : champ `slots: [<nom>]` sur un agent ; une skill remplit une prise par `metadata: { armadai-slot: <nom> }` (plusieurs prises séparées par des virgules) ; `link` ajoute à l'agent toutes les skills du projet qui remplissent ses prises, exactement comme si elles étaient listées dans `skills:`.
7. Documentation (`docs/wiki/project-format.md`, `docs/wiki/link.md`), CHANGELOG.

Non-objectifs :
- Commande qui s'exécute dans un sous-agent (`context: fork` + `agent` chez Claude, `agent` sous-agent chez OpenCode) : elle perd l'interaction avec l'utilisateur, aucun besoin de B2. Reste possible par `native`.
- Arguments positionnels (`$1`, `$ARGUMENTS[N]`, arguments nommés).
- Commandes Codex et Copilot (adaptateurs pas encore fidèles).
- Le pack lui-même (B2), et sa commande de paramétrage (équivalent de `/init` pour les normes et la doc, demandé par Dimitri) qui écrira une skill de projet branchée sur les prises.
- Une prise qui apporte des agents ou modifie un flow : une prise ne contient que des skills.

Anti-objectifs (ce qui serait un échec même si ça marche) :
- Une commande qui, à texte identique, fait autre chose dans Claude et dans OpenCode.
- Une commande qui exécute du shell au moment où elle est tapée sans que l'auteur l'ait écrit dans un fichier natif (`.armadai/native/<target>/…`).
- Une commande qui masque silencieusement une skill du même nom (ou l'inverse).

## 3. Contraintes (formats natifs vérifiés)

**Claude Code** (doc officielle, `code.claude.com/docs/en/skills.md`, relevé du 2026-10-21) :
- « Custom commands have been merged into skills. A file at `.claude/commands/deploy.md` and a skill at `.claude/skills/deploy/SKILL.md` both create `/deploy` and work the same way. Your existing `.claude/commands/` files keep working. »
- Une skill et une commande de même nom : **la skill gagne**, sans erreur.
- Sous-dossier → nom à deux-points (`frontend/component.md` → `/frontend:component`).
- Clés : `description`, `argument-hint`, `disable-model-invocation`, `allowed-tools`, `model`, `context`, `agent`, etc. ; une clé inconnue est ignorée sans erreur.
- `$ARGUMENTS` = tout le texte ; `$N` / `$ARGUMENTS[N]` **indexés à partir de 0** ; sans placeholder, Claude ajoute `ARGUMENTS: <saisie>` à la fin.
- `` !`cmd` `` exécuté au rendu, contrôlé par les règles de permission.
- Un sous-agent n'a jamais `AskUserQuestion` (`sub-agents.md`, « Available tools »).

**OpenCode 2.x** (code `dev` au commit 681156c8, `packages/core/src/config/plugin/command.ts`, `packages/schema/src/config/command.ts`) :
- `.opencode/commands/<nom>.md` (ou `command/`), nom = chemin relatif sans `.md`.
- Frontmatter : `description`, `agent`, `model`, `subagent` (alias déprécié `subtask`) ; corps = `template`.
- `$ARGUMENTS` = tout le texte ; `$1`, `$2`… **indexés à partir de 1** ; sans placeholder, la saisie est ajoutée à la fin.
- `` !`cmd` `` exécuté au rendu.
- Une commande dont l'`agent` est un sous-agent part en sous-tâche (plus d'interaction) ; un `agent` primaire bascule l'agent de la session.

Noms intégrés relevés le 2026-10-21 : la page `code.claude.com/docs/en/commands.md` liste ~150 commandes et skills intégrées (dont `/init`, `/review`, `/design`, `/run`, `/verify`, `/plan`, `/loop`, `/debug`) ; aucune ne s'appelle `mockup`, `iterate` ou `brief`. OpenCode : `/init`, `/undo`, `/redo`, `/help`… (`docs/commands.mdx`, `docs/tui`). Une commande de projet qui porterait un nom intégré est masquée ou masque ; d'où l'avertissement du §5.

Conséquences retenues :
- Seul `$ARGUMENTS` est portable : `$<chiffre>` et `$ARGUMENTS[` dans le corps → **erreur** de validation (sens différent selon la cible).
- `` !` `` ou une ligne ` ```! ` dans le corps → **erreur** de validation (exécution au rendu ; l'auteur qui le veut écrit un fichier natif `.armadai/native/<target>/…` sous un autre id que ses commandes, à ses risques : la clé `native` d'une commande n'ajoute que des clés de frontmatter).
- `${CLAUDE_…}` dans le corps (`${CLAUDE_SESSION_ID}`, `${CLAUDE_EFFORT}`, `${CLAUDE_SKILL_DIR}`, `${CLAUDE_PROJECT_DIR}`, `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, doc Claude relevée le 2026-10-22) → **erreur** `command-target-variable` : substitué par Claude, littéral sous OpenCode.
- `@chemin` : non vérifié dans la doc Claude ; laissé tel quel dans le texte, sans promesse.

## 4. Décidé avec Dimitri

- Prises ajoutées à B1 (2026-10-21) : les packs doivent être « plug and play » avec des normes de code, des docs d'infra, etc.

- Noms en anglais : `/mockup`, `/iterate`, `/brief` (B2) ; `iterate` doit aussi pouvoir être induit par le prompt de l'utilisateur → la méthode vit dans une skill, la commande la charge.
- Ordre : B1 (commandes dans le modèle) puis B2 (pack).
- Les commandes tournent dans la conversation principale (interaction avec l'utilisateur).

## 5. Correspondance (proposition de conception)

| Modèle | Claude | OpenCode | Rapport |
|---|---|---|---|
| fichier `commands/<id>.md` | `.claude/commands/<id>.md` | `.opencode/commands/<id>.md` | `compiled` |
| `description` | `description` | `description` | — |
| `argument_hint` | `argument-hint` | (aucun équivalent) | `approximated` côté OpenCode |
| corps | corps | corps (`template`) | — |
| `native.claude` / `native.opencode` | clés ajoutées au frontmatter (collision avec une clé générée = erreur, comme pour un agent) | idem | `compiled` |
| tout | — | — | Codex, Copilot : `not-compiled` (« no command support yet ») |

Règles de validation (codes stables, à nommer par le constructeur dans la famille existante) :
- id de commande = même règle de nom qu'un agent ou une skill ;
- id de commande égal à un id de skill → **erreur** (Claude ferait gagner la skill en silence) ;
- id de commande égal à un nom intégré connu d'une cible (Claude : `help`, `clear`, `compact`, `init`, `review`, … ; OpenCode : `init`, `review`, … — liste à relever dans la doc et le code au moment d'écrire) → **avertissement** ;
- `description` vide → erreur ; corps vide → erreur ;
- placeholders non portables et shell au rendu → erreur (§3).

### Prises

| Modèle | Claude | OpenCode | Rapport |
|---|---|---|---|
| agent `slots: [s]`, skills `k1`, `k2` remplissent `s` | `k1`, `k2` ajoutées au champ `skills` du sous-agent (préchargées) | ajoutées à la consigne « charge ces skills » ; et à `permission.skill` (`allow`) seulement si l'agent déclare des `skills:` (une prise ne restreint jamais, ruling du 2026-10-22) | `agent <id> slot <s>: compiled (k1, k2)` |
| prise sans aucune skill | rien | rien | `agent <id> slot <s>: not-compiled (no skill fills it)` — **pas une erreur** : le pack marche sans norme branchée |
| coordinateur avec `slots:` | fil principal : pas de préchargement (comme `skills:` aujourd'hui) | consigne (+ `permission.skill` seulement avec des `skills:` déclarées) | `approximated` côté Claude |

Règles :
- nom de prise = même règle de nom qu'un id ;
- une skill listée à la fois dans `skills:` et par une prise n'apparaît qu'une fois ;
- `armadai-slot` de valeur vide ou mal formée → erreur ; une skill peut remplir plusieurs prises ;
- ordre déterministe (id de skill trié) pour des étalons stables ;
- `list` / `inspect` montrent pour chaque prise les skills qui la remplissent ; `graph` n'est pas modifié.

`strict` : sans effet sur les commandes (elles tournent dans le fil principal, où le gate Claude et les permissions OpenCode s'appliquent déjà).

## 6. Laissé au constructeur

- Les noms exacts des codes de diagnostic, la forme du message de `new command`, l'ordre des lignes de rapport.
- La liste des noms intégrés par cible (relevée, sourcée dans le code).
- Le gabarit par défaut de `new command` (une description, un corps qui cite `$ARGUMENTS`).

## 7. Critères d'acceptation

1. Barrière complète verte ; chaque test ajouté vu rouge par mutation.
2. Étalons : un projet avec une prise remplie par deux skills, une prise vide, et deux commandes (dont une avec `argument_hint` et une avec `native`) lié vers les 4 cibles ; `unlink` revient à l'état initial.
3. Validation : chaque règle du §5 a son test à message exact.
4. Essai réel par Dimitri : dans Claude Code et dans OpenCode 2.1.2, `/<commande> du texte` apparaît dans le menu, reçoit le texte, et peut poser une question à choix à l'utilisateur ; une skill ajoutée après coup avec `armadai-slot` est chargeable par l'agent qui déclare la prise après un nouveau `link`.

   Note du 2026-10-22 (ruling du coordinateur, revue finale de B1) : « et seulement par lui sous OpenCode » est retiré. Une prise ne restreint jamais : seul un agent qui déclare des `skills:` reçoit `permission.skill` (`"*": deny`, ses skills et celles de ses prises en `allow`) ; un agent sans `skills:` garde toutes les skills, comme sous Claude où la prise précharge sans restreindre. Sinon, remplir après coup la prise d'un coordinateur lui retirait toutes ses autres skills.

## 8. Risques

- La doc Claude dit que `.claude/commands/` « keep working » : format hérité, susceptible d'être retiré. Repli : écrire une skill `disable-model-invocation: true` — non retenu maintenant pour ne pas mélanger commandes et skills dans le même dossier.
- `argument-hint` dans un fichier `commands/` : la doc ne liste les clés que pour les skills (« work the same way » laisse supposer qu'elles valent aussi) ; à confirmer à l'essai réel.
