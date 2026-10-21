# Étape 4, lot B1 : les commandes dans le modèle canonique

## 1. Intention

Le lot B livre un pack « du concept au code propre » dont l'utilisateur pilote une partie par des commandes explicites : `/mockup <sujet>`, `/iterate <retour>`, `/brief <idée>`. Aujourd'hui le modèle d'ArmadAI ne connaît que les agents, les skills, les flows et les automations : il n'a aucun moyen de déclarer une commande. B1 ajoute cet élément au modèle, une fois pour tous les packs, et le compile vers les deux cibles fidèles (Claude Code, OpenCode 2.x). B2 (le pack) s'appuie dessus.

Une commande est un **raccourci que l'utilisateur tape**. Elle tourne dans la conversation principale, pour garder l'interaction avec l'utilisateur (questions à choix, validation). La méthode longue vit dans une skill, que la commande charge ; la skill peut aussi être déclenchée par le modèle quand l'utilisateur formule la demande en langage naturel.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs :
1. `.armadai/commands/<id>.md` : frontmatter (`description` obligatoire, `argument_hint` facultatif, `native` facultatif comme pour un agent), corps = texte de la commande avec `$ARGUMENTS`.
2. Chargement strict avec diagnostics positionnés, validation sémantique à codes stables, écrivain canonique, schéma JSON régénéré.
3. `link` : Claude → `.claude/commands/<id>.md` ; OpenCode → `.opencode/commands/<id>.md` ; Codex, Copilot → ligne `not-compiled`. `unlink` retire ce que `link` a écrit (manifeste).
4. Une ligne de rapport de conformité par commande et par cible.
5. `armadai new command` (et entrée du menu guidé), `list`, `inspect` affichent les commandes.
6. Documentation (`docs/wiki/project-format.md`, `docs/wiki/link.md`), CHANGELOG.

Non-objectifs :
- Commande qui s'exécute dans un sous-agent (`context: fork` + `agent` chez Claude, `agent` sous-agent chez OpenCode) : elle perd l'interaction avec l'utilisateur, aucun besoin de B2. Reste possible par `native`.
- Arguments positionnels (`$1`, `$ARGUMENTS[N]`, arguments nommés).
- Commandes Codex et Copilot (adaptateurs pas encore fidèles).
- Le pack lui-même (B2).

Anti-objectifs (ce qui serait un échec même si ça marche) :
- Une commande qui, à texte identique, fait autre chose dans Claude et dans OpenCode.
- Une commande qui exécute du shell au moment où elle est tapée sans que l'auteur l'ait écrit dans `native`.
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
- `` !` `` ou une ligne ` ```! ` dans le corps → **erreur** de validation (exécution au rendu ; l'auteur qui le veut passe par `native`, à ses risques).
- `@chemin` : non vérifié dans la doc Claude ; laissé tel quel dans le texte, sans promesse.

## 4. Décidé avec Dimitri

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

`strict` : sans effet sur les commandes (elles tournent dans le fil principal, où le gate Claude et les permissions OpenCode s'appliquent déjà).

## 6. Laissé au constructeur

- Les noms exacts des codes de diagnostic, la forme du message de `new command`, l'ordre des lignes de rapport.
- La liste des noms intégrés par cible (relevée, sourcée dans le code).
- Le gabarit par défaut de `new command` (une description, un corps qui cite `$ARGUMENTS`).

## 7. Critères d'acceptation

1. Barrière complète verte ; chaque test ajouté vu rouge par mutation.
2. Étalons : un projet avec deux commandes (dont une avec `argument_hint` et une avec `native`) lié vers les 4 cibles ; `unlink` revient à l'état initial.
3. Validation : chaque règle du §5 a son test à message exact.
4. Essai réel par Dimitri : dans Claude Code et dans OpenCode 2.1.2, `/<commande> du texte` apparaît dans le menu, reçoit le texte, et peut poser une question à choix à l'utilisateur.

## 8. Risques

- La doc Claude dit que `.claude/commands/` « keep working » : format hérité, susceptible d'être retiré. Repli : écrire une skill `disable-model-invocation: true` — non retenu maintenant pour ne pas mélanger commandes et skills dans le même dossier.
- `argument-hint` dans un fichier `commands/` : la doc ne liste les clés que pour les skills (« work the same way » laisse supposer qu'elles valent aussi) ; à confirmer à l'essai réel.
