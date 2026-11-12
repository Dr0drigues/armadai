# Lot « sessions » : pack figé par lot, lien entre sessions, journal partagé

Légende. **V** = vérifié au code ou au wiki du dépôt (branche `feat/lot-sessions`, depuis master `727dd74`, v1.0.0-rc.11), par lecture, rien exécuté. **D** = lu dans la documentation officielle de Claude Code le 2026-11-12. **C** = à mesurer au plan avant d'écrire du code.

## 1. Intention

Un pack réel, utilisé sous `policy.delegation: strict` pendant plusieurs jours avec des lots menés en parallèle dans des worktrees, a montré trois manques de l'outil. Une publication du pack en plein lot a changé les règles d'un lot déjà commencé (le `.armadai/` et le `.claude/` d'un worktree de lot étaient des liens vers ceux du chantier) : une règle de nommage nouvelle a fait rejeter en revue un lot écrit et approuvé sous l'ancienne, ce qui a coûté un cycle complet. Les sessions des worktrees ne partagent aucun état : le coordinateur d'un lot a repris une prémisse non vérifiée et l'utilisateur a servi de relais par copier-coller. Un journal d'état tenu à la main (`.armadai/state/ledger-<lot>.md`) n'est qu'une consigne de prompt, rien ne l'écrit ni ne le lit.

Dimitri a décidé le 2026-11-12, à la question « Quelles pistes prendre dans un lot ArmadAI (spec → plan → tâches), et avant le lot C ? » : « Pack figé par lot, Lien entre sessions, Journal partagé ». Hors périmètre : les hand-backs livrés en double, défaut du harness Claude Code et non d'ArmadAI.

Décisions de Dimitri sur la conception (2026-11-12), mot pour mot : verrou de worktree « Verrou de worktree (Recommandé) » ; cycle de vie « pack pin explicite + update refusé (Recommandé) » ; liens vers le chantier sous verrou « Refuse avec la raison (Recommandé) » ; emplacement du journal « Un fichier par lot » (`state/journal-<lot>.jsonl`, contre la recommandation de l'analyst : un fichier unique) ; format « JSONL via journal add (Recommandé) » ; lecture « Consigne + journal: read (Recommandé) » ; lien « Par flow (Recommandé) » ; découpage « 2 PR (Recommandé) ».

Sources : le retour d'expérience du pack (privé, hors dépôt), section « Après la 1.7.0 » et pistes du bilan 1.4 à 1.6 ; la spec et le plan du lot précédent (`2026-11-10-retex-orchestration`).

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs

1. **Pack figé par worktree.** Un worktree de lot garde la version du pack avec laquelle il a démarré, pour `link`, `validate`, `flow`, le gate et les hooks, tant qu'on ne la change pas explicitement, même si le chantier avance son `pack.lock`.
2. **Journal partagé.** Un journal natif d'état et de décisions, lisible et écrit par la session de chaque lot et par celle du chantier, partagé entre worktrees, via des commandes (`armadai journal add|show`), sans fichier tenu à la main.
3. **Lien entre sessions.** Un flow (ou le projet) déclare un canal vers la session du chantier ; `link` compile pour Claude Code la consigne qui s'en sert (la messagerie entre sessions du harness) au lieu du copier-coller, avec le journal en repli.

### Non-objectifs

- Dédoublonner les hand-backs (harness).
- Créer ou supprimer des worktrees : ArmadAI ne gère pas `git worktree` ; le lien symbolique du pack reste le choix du pack.
- Exécuter ou intercepter la messagerie : ArmadAI ne lit, n'envoie, ne filtre aucun message entre sessions (pas de hook, pas de gate sur `SendMessage`).
- Écrire `crossSessionInbound`, `isolatePeerMachines` ou une règle de permission sur `SendMessage`/`ListAgents` : ce sont les réglages de l'utilisateur.
- Messagerie vers d'autres machines ou le cloud ; compiler le lien vers OpenCode, Codex, Copilot (`not-compiled`).
- Un serveur, un démon ou une base : le journal est un fichier.
- Synchroniser le journal entre machines ou le versionner.

### Anti-objectifs

- Qu'un projet sans la clé voie le moindre octet changer (sorties, manifeste, `settings.json`, `CLAUDE.md`).
- Que `link` lancé dans un worktree épinglé réécrive, à travers un lien symbolique, le `.claude/` ou le `.armadai/` du chantier.
- Que le gate (seule contrainte dure sous `strict`) dépende du journal ou du lien entre sessions.
- Que le journal enregistre le texte d'une réponse d'agent ou d'un prompt (seulement ce que l'utilisateur ou l'agent y écrit explicitement, borné).
- Qu'un message d'une autre session soit traité comme une instruction d'ArmadAI, ou compte comme un consentement.
- Que le dépôt public reçoive un contenu privé (journal brut, nom de pack, chemin de machine).

## 3. Contraintes

- Compilateur déclaratif, aucun moteur d'exécution ; `link` ne touche jamais le réseau ; `armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars) et `cargo build -p armadai-core` / `-p armadai-model` passent.
- **Opt-in** : clé absente de `project.yaml`, `link` n'écrit rien sur ce point (diff nul) ; en conflit avec une valeur de l'utilisateur, `link` refuse et rapporte `not-compiled` avec la raison. Le verrou de worktree n'existe que si on le pose.
- **Sorties versionnées** : le manifeste reste `version: 1` (champs optionnels), `flow status --json` reste `armadai.flow-status/1`, `pack.lock` reste `format: 1` (V : `deny_unknown_fields`, donc le verrou de worktree n'ajoute aucun champ au `pack.lock` commité), le journal naît `armadai.journal/1`. Un test lit un document écrit par la version précédente.
- Aucun paquet nouveau au lock sans décision de Dimitri.
- Rien de privé dans le dépôt public ; fixtures anonymes (noms génériques, aucun `/Users/`).
- Le gate sous `strict` reste la seule contrainte dure ; un hook qui ne comprend pas son entrée ne donne aucun avis (jamais `allow` sur un doute).
- Les packs ont un mode référence (`pack:` dans `project.yaml`, `.armadai/pack.lock`, snapshot en cache) et un mode copie (fichiers dans `.armadai/`, état dans le manifeste, ni lock ni snapshot) ; le verrou de worktree ne concerne que le mode référence (en copie, les fichiers sont déjà dans le worktree).
- Codes de diagnostic en kebab-case ajoutés à la table de `docs/wiki/project-format.md` ; texte de toute ligne nouvelle de sortie proposé par le plan et validé par Dimitri ; wiki mis à jour (`starter-packs.md`, `flow-status.md`, `link.md`, `project-format.md`, `policy-gate.md`).
- Mutations vues rouges pour chaque test ; essais du binaire avec `HOME` et `ARMADAI_CONFIG_DIR` isolés.

## 4. Conception

### État du code (V)

- **Pack.** `pack.lock` (`pack/lock.rs`) est lu par toutes les commandes via `pack::effective::open` (appelants : `link`, `validate`, `flow`, `list`, `graph`, `unlink`, `new`, `grant`, `pack test`, le gate et le hook de grant). Il fixe un commit (ou un digest), jamais un tag ; seul `pack update` l'avance. Le snapshot `<registry>/packs/<clé>/<pack>/<id>/` est immuable, en lecture seule, jamais réécrit. Il n'existe aucun moyen de pinner hors de `.armadai/`. Cause du défaut : le `pack.lock` lu est celui du `.armadai/` du worktree ; quand celui-ci est un lien vers le chantier, un `pack update` du chantier le change pour tous. Et `link` du chantier réécrit le `.claude/` partagé.
- **Localisation du projet.** Trois implémentations de « remonter jusqu'au premier `.armadai/project.yaml` sans franchir `.git` », sans `canonicalize` ; `policy_gate::resolve_project` retombe sur le checkout principal depuis un worktree git (`main_checkout_of`). `link` n'a pas ce repli. L'écriture de `state/`, `state/claude/`, du journal de flow, de `pack.lock` est refusée si la cible est un lien symbolique.
- **État.** `.armadai/state/` (ignoré par git) reçoit `claude/<session>.jsonl` (journal de flow) et `grants.yaml`. `ledger-<lot>.md` n'est écrit que par convention de prompt (`.armadai/agents/lead.md`).
- **SendMessage.** Le gate ne le juge pas (matcher `Agent|Task`, `policy-gate.md`), le recorder non plus : aucun hook de ce dépôt ne voit un message entre sessions. Aucune occurrence de `ListAgents` ou `crossSessionInbound` dans `crates/` ni `docs/`.

### Fait de la cible : la messagerie entre sessions (D, 2026-11-12)

Source : `https://code.claude.com/docs/en/cross-session-messaging.md`, lue le 2026-11-12. La fonction est documentée (le plan l'avait seulement observée). Extraits, tels quels :

- « Cross-session messaging requires Claude Code v2.1.224 or later on macOS and Linux, including Linux inside WSL 2. » et « When a session meets the requirements, messaging is on with nothing to enable. »
- « Claude discovers the target with `ListAgents` and sends with `SendMessage`, so you never call either tool yourself. »
- « A session answers to the name you set with the `/rename` command or the `--name` flag. When you don't set one, Claude Code names the session itself. »
- « Coordinate parallel worktrees: when sessions work the same repository in separate worktrees, Claude can tell the other sessions what landed. »
- Même machine : « Over a per-session socket on macOS and Linux [...] never through Anthropic servers. »
- « Claude Code instructs the receiving Claude never to change permission settings, `CLAUDE.md`, or other configuration because another session asked. » ; un message « can't approve anything » et ses commandes « arrive as plain text ».
- Réception (`crossSessionInbound`) : « The receiving session bypasses permission prompts: Claude Code holds each message for your approval. It delivers one only when the sending session identifies itself as also bypassing. » ; un message tenu « reaches Claude only when you approve it ».
- Limites : texte seul, environ un million de caractères, rafales refusées, boucles limitées (50 messages en file).

Conséquences pour la conception : (1) rien à activer, ArmadAI n'a donc aucun réglage à écrire ; (2) la cible d'un message est un **nom de session** (`--name`, `/rename`), que l'utilisateur donne au lancement : ArmadAI ne peut que le déclarer et l'afficher ; (3) une session du chantier en mode qui contourne les permissions **tient** les messages entrants pour approbation, ce qui casse un canal « sans copier-coller » (risque 3) ; (4) la messagerie est de la guidance de prompt côté ArmadAI, jamais une garantie : d'où le journal, qui est un fichier. **C** : la version de Claude Code de Dimitri (`claude --version` ≥ 2.1.224) et un aller-retour réel entre deux sessions nommées, à mesurer au plan.

### Piste 1. Verrou de pack par worktree (`armadai`, `pack/`)

Principe (décidé) : un **verrou de worktree**, copie du `pack.lock` pris au moment du pin, rangé hors de `.armadai/` et hors de tout lien vers le chantier, par exemple dans le répertoire git privé du worktree (non versionné, non partagé par construction). **C** : le chemin exact (`git rev-parse --git-dir`, ou lecture du fichier `.git` d'un worktree) et le comportement quand ce n'est pas un dépôt git (le verrou n'est alors pas disponible, message clair).

- `armadai pack pin` : écrit le verrou de worktree à partir du `pack.lock` effectif ; `armadai pack unpin` le retire ; `pack status` indique « pinned (worktree) <version> » et, s'il diffère, la version du lock du chantier.
- `pack::effective::open` lit le verrou de worktree **avant** `.armadai/pack.lock` quand il existe ; tout appelant en hérite (V : un seul point d'entrée), y compris le gate et les hooks. Aucun champ de plus dans `pack.lock`.
- Snapshot absent du cache pour la version épinglée : erreur « run `armadai pack sync` » comme aujourd'hui, sans réseau.
- `link` sous un verrou actif écrit à partir de la vue épinglée, et **refuse** (code non nul, raison donnée) d'écrire dans un `.claude/` ou `.armadai/` qui est un lien symbolique sortant du worktree, pour ne pas réécrire le chantier (décidé). Sans verrou : une ligne de note, pas de refus (comportement actuel conservé).
- `pack update` dans un worktree épinglé : refus nommant le verrou (décidé) ; le pin ne se pose jamais seul (ni au premier `link`, ni au démarrage d'un flow).
- `unlink` retire ce que `link` a écrit, comme aujourd'hui ; il ne retire pas le verrou (c'est `pack unpin`).

### Piste 2. Journal partagé (`armadai`, nouveau `journal/`)

Principe (décidé) : **un fichier JSONL append-only par lot**, `armadai.journal/1`, `state/journal-<lot>.jsonl`, dans le `state/` du **checkout principal** (retrouvé par le repli de `resolve_project`), donc le même pour le chantier et tous ses worktrees, qu'ils soient des checkouts réels ou des liens.

```json
{"schema":"armadai.journal/1","ts":1763000000,"kind":"decision","lot":"menu","by":"<agent_type|session>","text":"…"}
```

- `kind` : `decision`, `state`, `note` (liste fermée ; un genre inconnu à la lecture est montré tel quel). `text` borné (2000 caractères, une ligne ; refusé au-delà avec le message de la limite). `by` pris du hook (agent) ou du nom de session quand on le connaît, sinon absent.
- **Le nom de lot sert de nom de fichier.**
  - Une seule définition de « lot valide » : celle du marqueur du gate (`is_valid_lot` dans `flow_status/marker.rs`, branche de correctif `fix/marker-lot-case` : 1 à 40 lettres ASCII majuscules et minuscules, chiffres, `.` et `-` ; `_` et `+` sont refusés, Dimitri : « Non, `-` suffit » pour `+` ; le journal appelle cette même fonction, jamais une copie, pour suivre le correctif, qui n'est pas encore sur master `727dd74`). Le journal y ajoute sa règle de nom de fichier : le lot doit commencer par une lettre ou un chiffre. `.`, `..` et tout lot commençant par `.` sont refusés par `journal add` avec la raison ; ils ne peuvent pas sortir de `state/` (le nom est `journal-` + lot + `.jsonl`, sans séparateur de chemin), mais un fichier caché ou ambigu n'est pas voulu. Conséquence assumée : un lot que le gate accepte peut ne pas avoir de journal (message clair, pas de refus du gate).
  - Sur un système de fichiers qui ignore la casse (macOS), `Menu` et `menu` seraient un même fichier : le nom de fichier est le lot en minuscules, le champ `lot` de chaque ligne garde la casse exacte, et `show --lot` filtre sur le champ. Deux lots qui ne diffèrent que par la casse partagent un fichier mais ne se mélangent pas à la lecture. Révocable si Dimitri préfère les refuser.
  - Les fichiers sont reconnus par leur nom exact `journal-<lot>.jsonl` (et `journal.jsonl` ci-dessous) ; tout autre fichier de `state/` est ignoré par `show`.
- **Entrées sans lot** (décidé : « state/journal.jsonl, lu avec chaque lot ») : fichier `journal.jsonl`, sans suffixe, qui ne peut pas entrer en collision avec un `journal-<lot>.jsonl`. Il porte les décisions du chantier qui valent pour tous les lots (par exemple une version de pack). `journal show --lot X` montre les entrées de `X` **et** celles de ce fichier, marquées comme telles.
- `armadai journal add [--lot <lot>] [--kind decision|state|note] <texte>` ; `armadai journal show [--lot <lot>] [--kind …] [--last <n>] [--json]`, sortie stable, code 0 sur journal vide. Sans `--lot`, `show` lit **tous** les fichiers (`journal.jsonl` et `journal-*.jsonl`), fusionne par `ts` (égalité : nom du lot, puis ordre dans le fichier, donc un ordre stable), puis applique `--last` ; chaque ligne montre son lot. `--lot X` ne lit que le fichier de `X` (et `journal.jsonl`) : c'est ainsi qu'un agent vérifie une prémisse sur un autre lot.
- Écriture : une seule écriture en mode ajout par ligne (atomique sous la taille d'un tuyau), refus si le fichier ou `state/` est un lien symbolique (règle existante du dépôt). Aucune édition ni suppression de ligne.
- Lecture par chaque session : avec la clé opt-in, `link` ajoute au bloc coordinateur de `CLAUDE.md` la consigne « avant de décider, `armadai journal show --lot <lot> --last 20` ; après une décision, `armadai journal add --kind decision …` » ; **C** : un hook `SessionStart` qui affiche les dernières entrées (la sortie d'un hook de démarrage entre-t-elle dans le contexte ? à lire dans la doc des hooks et mesurer avant de l'inclure).
- **Lecture par les agents, par leur définition (ajout du 2026-11-12).** Retour du pack : les sous-agents de revue ne lisaient pas le journal des décisions, le coordinateur devait le leur faire lire dans un brief. Une consigne dans le bloc coordinateur ne les atteint pas (ils ne lisent pas `CLAUDE.md` comme le coordinateur). Un agent déclare donc `journal: read` (ou `write`, qui inclut la lecture) dans sa définition `.armadai/agents/<id>.md` ; `link` ajoute alors au corps de l'agent compilé pour Claude la consigne de lire `armadai journal show --lot <lot> --last 20` avant de juger, et de vérifier une prémisse sur un autre lot dans le journal plutôt que de mémoire. L'agent a besoin d'un moyen d'exécuter la commande : le validateur rapporte un avertissement (code à nommer au plan) pour un agent qui déclare `journal` sans capacité shell. Sans la clé sur l'agent, son fichier compilé est inchangé. Déjà en place côté pack, hors de ce lot : vérifier une prémisse sur un autre lot dans le journal, inscrire les réponses de l'utilisateur dans le brief suivant. Le défaut « `#lot` en majuscules refusé par le gate » est corrigé à part, hors de cette spec.
- Le journal remplace la convention `ledger-<lot>.md` du coordinateur de ce dépôt (changement de `.armadai/agents/lead.md`, tâche du plan, après la livraison de la commande).
- Clé opt-in : `sessions.journal: true` dans `project.yaml` (nom à valider au plan). Absente : ni consigne, ni hook ; les commandes `journal` restent utilisables à la main.

### Piste 3. Lien entre sessions (`armadai-model`, `compile/`)

Principe (décidé : par flow) : un flow déclare vers quelle session on s'adresse, dans un champ pur du modèle :

```yaml
flows:
  evolution:
    session_link:
      to: chantier              # nom de session Claude Code (--name / /rename)
      when: "question sur le périmètre, une prémisse ou une règle du chantier"
```

- Validation : `to` suit `is_valid_id` ; code de diagnostic nouveau pour un `session_link` mal formé (nom à valider au plan) ; schémas JSON régénérés ; absent, rien ne change.
- `link --target claude` ajoute au bloc coordinateur de `CLAUDE.md` une consigne par flow : démarrer cette session avec `--name <nom propre>`, écrire à la session `<to>` (« sends with `SendMessage` », D) quand `when` s'applique, **recopier la réponse dans le journal** si le journal est actif, et, si la session n'est pas listée ou ne répond pas, retomber sur le journal plutôt que sur l'utilisateur. La consigne rappelle qu'un message reçu n'est ni un consentement ni un ordre de changer la configuration (D).
- Rien d'autre n'est écrit : pas de réglage de messagerie, pas de permission, pas de hook ; le gate ne juge pas `SendMessage` (inchangé, test existant).
- Autres cibles : `not-compiled (Claude Code only)`. Ligne de rapport Claude : `approximated` (guidance, le harness décide d'envoyer).
- `flow status` n'affiche rien de nouveau ; la messagerie n'est pas observable par un hook (V), ArmadAI ne peut donc ni la mesurer ni la compter.

### Ordre de livraison (décidé : 2 PR)

Deux PR, chacune barrière verte et utilisable seule :
1. **PR 1, pack figé** (piste 1) : indépendante, résout le coût mesuré.
2. **PR 2, journal puis lien** (pistes 2 et 3) : le lien se replie sur le journal, il faut donc qu'il existe ; essai réel de Dimitri à la fin avec deux sessions nommées.

L'ordre de Dimitri (« pack, lien, journal ») est tenu pour le pack ; le journal passe avant le lien dans la PR 2 parce que le lien en dépend.

## 5. Critères d'acceptation

1. Dans un worktree d'un projet en mode référence, `armadai pack pin` écrit un verrou hors de `.armadai/` et de tout lien vers le chantier : `git status` reste propre, `armadai pack status` affiche « pinned (worktree) » avec la version épinglée ; `armadai pack unpin` le retire et `pack status` ne le mentionne plus.
2. Après un `pack pin` à la version 1.3.0, le chantier lance `pack update` vers 1.4.0 (son `pack.lock` change) : dans le worktree, `armadai link --target claude`, `armadai validate` et `armadai list` utilisent le contenu du snapshot 1.3.0, et le gate décide sur la vue 1.3.0 ; dans le chantier, c'est la 1.4.0.
3. Sous un verrou actif, `armadai link` dans un worktree dont `.claude/` (ou `.armadai/`) est un lien symbolique vers un dossier hors du worktree refuse, code non nul, avec la raison, et le dossier du chantier est intact (sommes de contrôle identiques) ; `armadai pack update` y est refusé en nommant le verrou ; le verrou visant un snapshot absent du cache donne l'erreur « run `armadai pack sync` », sans accès réseau.
4. Sans `pack pin`, `link`, `validate`, `flow`, le gate et `pack status` donnent une sortie identique octet pour octet à celle de master `727dd74` (golden inchangés) ; un `pack.lock` écrit par la version précédente se lit sans changement.
5. `armadai journal add --lot menu --kind decision "..."` suivi de `armadai journal show --lot menu` depuis un autre worktree du même dépôt affiche l'entrée, écrite dans `state/journal-menu.jsonl` du checkout principal ; `--json` porte `schema: "armadai.journal/1"` ; une ligne de plus de 2000 caractères ou un `--kind` hors liste est refusé avec la limite dans le message ; un journal vide donne une sortie vide, code 0.
6. Deux processus qui ajoutent chacun 50 entrées au même lot en même temps laissent 100 lignes intactes (JSON valide, aucune ligne coupée), sur 5 passes répétées ; un `state/` ou un fichier de journal qui est un lien symbolique est refusé à l'écriture.
7. Noms de lot : `journal add --lot .`, `--lot ..`, `--lot .x`, `--lot a_b`, `--lot a+b` et un lot de 41 caractères sont refusés avec la raison, sans rien écrire ni hors de `state/` ni dedans ; `--lot Menu-2.1` (majuscules, chiffre, `-`, `.`, que `is_valid_lot` accepte une fois le correctif livré) est écrit dans `journal-menu-2.1.jsonl` et `show --lot Menu-2.1` ne montre pas les entrées de `menu-2.1` (casse différente, même fichier) ; deux lots `a` et `b` écrits puis `armadai journal show` sans `--lot` les affichent ensemble, triés par `ts`, avec leur lot sur chaque ligne ; `show --lot a` n'affiche rien de `b`.
8. Avec `sessions.journal: true`, `armadai link --target claude` ajoute au bloc de `CLAUDE.md` la consigne de lecture et d'écriture du journal ; sans la clé, `CLAUDE.md`, `settings.json` et le manifeste sont identiques à avant (diff nul).
9. Un agent dont la définition porte `journal: read` (par exemple un agent de revue) a, dans son fichier compilé par `armadai link --target claude`, la consigne de lire le journal du lot avant de juger, sans qu'aucun brief ne la répète ; un agent sans la clé a un fichier compilé identique à avant ; `armadai validate` avertit (code stable) pour un agent qui déclare `journal` sans capacité shell ; la clé est refusée avec une raison sur une cible autre que Claude (`not-compiled`).
10. Un flow qui déclare `session_link.to: chantier` donne, après `armadai link --target claude`, une consigne dans le bloc coordinateur qui nomme `chantier` et le repli sur le journal ; un nom mal formé est refusé par `armadai validate` avec un code stable ; sur OpenCode, Codex et Copilot le rapport dit `not-compiled (Claude Code only)` ; `link` n'écrit ni `crossSessionInbound`, ni règle `SendMessage` ou `ListAgents` ; un appel `SendMessage` reste sans avis du gate sous `strict`.
11. Wiki et schémas : `armadai-model` compile seul, les schémas JSON sont régénérés sans diff résiduel, les nouveaux codes sont dans la table de `docs/wiki/project-format.md`, et `docs/wiki/starter-packs.md` documente le verrou de worktree et la limite en mode copie.
12. Essai réel de Dimitri : deux sessions nommées (le chantier et un lot dans un worktree épinglé). Le lot reste sur la version du pack de son démarrage après une publication du chantier ; sa session pose une question à la session du chantier par le canal déclaré, la réponse revient sans copier-coller, et le journal montre la décision depuis les deux sessions. Si la session du chantier tient les messages (mode qui contourne les permissions), l'essai le constate et le repli sur le journal fonctionne.

## 6. Risques

1. **Chemin du verrou de worktree.** Le répertoire git privé d'un worktree se retrouve de plusieurs façons (**C**) ; sans shell git, la lecture du fichier `.git` peut se tromper pour un sous-module ou un worktree déplacé. Signe : critère 1 rouge en essai. Parade : mesure au plan, erreur claire hors dépôt git, repli documenté (variable d'environnement) seulement si Dimitri le veut.
2. **`.claude/` partagé, résolu à moitié.** Le verrou fige ce que lit ArmadAI ; le `.claude/` d'un worktree lié au chantier est réécrit par le `link` du chantier, que ce verrou ne voit pas. Signe : règles qui changent dans le worktree sans que le verrou soit en cause. Parade : le refus du critère 3 ne protège que le `link` du worktree ; le wiki dit qu'un worktree épinglé doit avoir son propre `.claude/` (le pack le garantit, ArmadAI le détecte).
3. **Messages tenus.** Une session du chantier en mode qui contourne les permissions tient chaque message entrant pour approbation (D) : le canal se bloque en silence. Signe : question sans réponse. Parade : la consigne compilée prévoit le repli sur le journal ; le wiki documente la condition ; l'essai réel la mesure.
4. **Le modèle n'utilise pas le lien.** C'est de la guidance : la prose n'est pas mesurée dans ce dépôt (9 délégations sur 520 sans gate, `policy-gate.md`), et la messagerie n'est pas observable par un hook. Signe : l'utilisateur continue de relayer. Parade : le journal (fichier, commande) ne dépend pas du modèle de la même façon, et l'essai réel est le critère ; pas de promesse d'application.
5. **Journal qui grossit ou se pollue.** Append-only, texte libre, un fichier par lot qui s'accumule dans `state/`. Signe : `show` illisible, dossier encombré. Parade : bornes, filtre `--lot`/`--last`, par défaut les 20 dernières ; la rotation et le nettoyage d'un lot fini sont hors lot.
5b. **Nom de lot et système de fichiers.** Le nom du lot devient un nom de fichier : casse ignorée sur macOS, lots acceptés par le gate mais refusés par le journal (début par `.`), et un correctif du gate en cours (`fix/marker-lot-case`, pas encore sur master) qui fixe l'alphabet à lettres, chiffres, `.` et `-` (sans `_` ni `+`). Signe : critère 7 rouge après le correctif du gate. Parade : une seule fonction de validité partagée avec le marqueur, règle de fichier ajoutée par-dessus, test sur `.`, `..`, majuscules et 40 caractères.
6. **Contenu privé dans le journal.** Un agent y écrit un nom de client ; le fichier est dans `state/` (ignoré par git, jamais publié) mais `show` peut finir dans un rapport. Parade : wiki, aucun contenu dans les fixtures, test qui refuse `/Users/`.
7. **Deux sources de vérité.** Le journal et le `ledger-<lot>.md` du coordinateur coexistent pendant la transition. Parade : la tâche qui remplace la convention dans `lead.md` est dans la PR 2.
8. **API de la cible.** La messagerie est récente (v2.1.224) et peut changer de nom d'outil ou de réglage. Parade : opt-in, `approximated`, extraits de doc datés dans le wiki.

## Questions ouvertes

Aucune question de conception ouverte. Tranché par Dimitri le 2026-11-12 : entrées sans lot dans `state/journal.jsonl`, lu avec chaque lot ; verrou de worktree, `pack pin` explicite et `update` refusé, refus sous lien vers le chantier, un fichier de journal par lot, JSONL par `journal add`, consigne plus `journal: read`, lien par flow, 2 PR. Tranché par l'analyst, révocable : nom de fichier en minuscules avec la casse gardée dans le champ `lot`. Restent aussi les points **C** : chemin du répertoire git privé d'un worktree, hook `SessionStart` et contexte, version de Claude Code de Dimitri et aller-retour réel entre deux sessions nommées.
