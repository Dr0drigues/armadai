# Policy gate d'orchestration — rendre la topologie déclarée contraignante — Design

**Date** : 2026-08-20
**Statut** : validé (design), à implémenter
**Cible** : socle + contrainte de topologie. Premier des quatre lots d'un moteur de politique (voir « Découpage »).

## Contexte : le constat mesuré

`.armadai/config.yaml` déclare déjà l'orchestration de ce projet, et elle est correcte :

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: dev-lead
  teams:
    - agents: [core-specialist, provider-specialist, cli-specialist, ui-specialist, qa-specialist]
```

Cette déclaration n'a **aucun effet** sur les sessions Claude Code : elle ne sert qu'à `armadai run --orchestrate`. Le comportement réel des sessions est censé être gouverné par de la prose dans `CLAUDE.md`. Il ne l'est pas.

**Mesures (2026-08-20, sur le corpus réel de ce dépôt via `armadai audit`)** :

- `dev-lead` reçoit **9 délégations sur 520** (2 %), alors que `CLAUDE.md` prescrit de passer par lui. `general-purpose` en reçoit 286.
- Bench déclaratif, 8 runs par variante, question « à qui confies-tu cette tâche ? » avec `CLAUDE.md` pour seule source : **0/8 mentions de `dev-lead`** avec la prose actuelle, **0/8** avec une prose réécrite prescrivant la délégation directe. Variance nulle.
- Test de lecture : interrogé sur ses instructions, le modèle **cite la phrase mot pour mot**, `dev-lead` inclus, et affirme la suivre — puis nomme `cli-specialist` face à une tâche réelle. L'écart n'est pas entre la prose et la lecture, il est entre la déclaration et l'action.
- Bench comportemental, tâche multi-modules, tous outils : 4 runs aboutis sur la variante A, **aucune délégation**. (Limite : `claude -p` a un biais anti-délégation propre au non-interactif ; l'historique interactif, lui, montre 520 délégations.)

**Cause identifiée** : trois sources concurrentes. `.armadai/config.yaml` et le `CLAUDE.md` racine s'accordent (dev-lead coordinateur) ; `.claude/CLAUDE.md` diverge en ouvrant par « **You are** the Dev Lead […] delegate them to the right specialist(s) » et fournit la table d'équipe. C'est cette troisième source qui gouverne — elle est plus spécifique et surtout **actionnable** : elle donne le routage, là où le racine impose une obligation sans mode d'emploi.

**Précédent** : le même mécanisme a déjà mordu ce projet le 2026-07-23 (mémoire `project_workroom_marker_emission`) — le modèle lisait le protocole de marqueurs dans `CLAUDE.md` mais suivait une consigne concurrente du même fichier. Diagnostic de l'époque : « tension de design de protocole ». La résolution retenue alors n'a pas été de mieux rédiger le Markdown, mais de déplacer la source de vérité vers un flux émis par du code (`RunEvent`). Ce spec applique le même remède au routage.

**Conclusion** : le Markdown est du contexte, pas une règle. Rien ne l'évalue, rien ne le vérifie, et deux fichiers peuvent se contredire indéfiniment sans que quoi que ce soit ne le signale. Ce n'est pas un problème de formulation ; c'est un problème de nature.

## Ce que le spike a établi (2026-08-20)

Vérifié **empiriquement** (worktree jetable, sonde journalisant le payload brut, 6 appels capturés) et **confirmé sur la documentation primaire** (`code.claude.com/docs/en/hooks.md`, `sub-agents.md`, `permissions.md`, `plugins-reference.md`, `headless.md`) :

- `PreToolUse` intercepte l'outil `Agent` (renommé depuis `Task` en v2.1.63 ; les deux noms restent valides). La doc le liste explicitement parmi les outils matchables.
- Le matcher porte sur le **nom d'outil** uniquement. `"Agent|Task"` est valide (alternative). Il ne peut pas filtrer sur `subagent_type` ; le champ `if` du handler le permet (`"if": "Agent(Explore)"`).
- Le payload stdin contient **la cible et l'appelant** :
  - `tool_input.subagent_type` — la cible ;
  - `agent_type` — l'appelant : **vide pour le fil principal**, renseigné (`dev-lead`) quand un sous-agent sous-délègue ;
  - plus `agent_id`, `tool_name`, `tool_use_id`, `cwd`, `session_id`, `transcript_path`, `permission_mode`.
- Refus : exit code 2, ou exit 0 avec `hookSpecificOutput.permissionDecision: "deny"` + `permissionDecisionReason`. **La raison est remontée au modèle** sur un `deny`.
- La contrainte s'applique **récursivement** : `dev-lead` a été intercepté en tentant de sous-déléguer.
- Les hooks fonctionnent en `claude -p` (le spike y a été mené). `--bare` les désactive.
- Comportement observé décisif : un seul refus, et le modèle a **immédiatement** emprunté le chemin prescrit, puis expliqué la règle. Là où la prose est citée et ignorée (0/8), le hook est obéi du premier coup.

**Impasses écartées** : `SubagentStart` ne peut pas bloquer ; les hooks déclarés dans le frontmatter d'un agent ne peuvent pas intercepter la délégation elle-même. `PreToolUse` sur `Agent` est le seul point de contrôle avant exécution.

**Capacité disponible mais non retenue** : `permissionDecision: "allow"` + `updatedInput` permet de **rediriger** une délégation (en réémettant tout le `tool_input`, pas seulement `subagent_type`). Écartée — voir « Décisions de cadrage ».

**Pourquoi un hook plutôt que les permissions statiques** : `permissions.deny: ["Agent(x)"]` et l'allowlist `tools: Agent(...)` du frontmatter existent et sont plus simples, mais ne connaissent pas **l'appelant**. Une topologie a besoin d'autoriser une cible depuis le coordinateur tout en la refusant depuis le fil principal. Seul le hook voit `agent_type`.

## Décisions de cadrage (Dimitri, 2026-08-20)

1. **Cible = moteur de politique complet**, construit par lots. Ce spec couvre le socle et la topologie.
2. **Comportement en cas d'écart** : `deny` avec raison actionnable, désactivable par `orchestration.policy: off|strict`. Pas de mode `warn`.
3. **Installation par les deux véhicules** : le linker (par projet) et le plugin (global), servis par la même sous-commande.
4. **Inversion allowlist (Dimitri)** : ce qui n'est **pas déclaré** ne passe pas. Pas de liste d'exemptions en dur — une seule règle, aucune exception. Conséquence recherchée : la config devient exhaustive et honnête.
5. **`free_agents`** : les agents d'assistance (`Explore`, `Plan`) se déclarent dans une liste distincte, appelable par tous, hors topologie.
6. **Pas de redirection** (`updatedInput`) : le `prompt` d'une délégation est rédigé pour sa cible ; réécrire la cible sans le prompt produirait une délégation incohérente. Le refus obtient un meilleur résultat — le modèle réécrit lui-même l'appel complet.

## Découpage (le moteur complet reste la cible)

Les cinq dimensions d'un moteur de politique n'ont pas les mêmes besoins techniques, et c'est ce qui dicte l'ordre :

| Dimension | Décidable avec quoi | Lot |
|---|---|---|
| Topologie | le seul payload, sans état | **ce spec** |
| Routes / tags | le payload, mais interprétation de `description`/`prompt` | lot 4 |
| Scope fichiers | **un second point d'interception** (`PreToolUse` sur `Write`/`Edit`) | lot 2 |
| Budget / coût | **un état persistant** entre appels | lot 3 |
| Profondeur max | idem (`agent_type` ne donne que le parent immédiat) | lot 3 |

Livrer l'ensemble d'un bloc imposerait de bâtir la couche d'état avant d'avoir vérifié qu'une policy bloquante est vivable au quotidien.

## Architecture

Une seule approche retenue. L'alternative — le linker **génère** un script ou un JSON de policy figé, évitant de lancer `armadai` à chaque délégation — est écartée : un artefact généré qui se désynchronise de sa source est exactement le problème que ce spec traite. Le coût est négligeable (les délégations se comptent en dizaines par session).

La config est donc **lue à chaud**, et le découpage suit la frontière que le projet applique déjà :

```
Claude Code émet un tool_use "Agent"
         │
         ▼  PreToolUse, matcher "Agent|Task"
armadai __claude-policy-gate            ← stdin : payload JSON
         │   (bin-side, voisin de __claude-register-session, cli/mod.rs:490)
         │   lit .armadai/config.yaml depuis le `cwd` du payload
         ▼
armadai_core::orchestration::policy::check_delegation(
    caller: Option<&str>,      // agent_type — None = fil principal
    target: &str,              // tool_input.subagent_type
    config: &OrchestrationConfig,
) -> Result<(), PolicyViolation>
         │
         ▼  stdout : hookSpecificOutput { permissionDecision, permissionDecisionReason }
Claude Code autorise ou refuse
```

- **La décision vit dans `armadai-core`** (`orchestration/policy.rs`), en fonction pure sans I/O, sur le modèle de `agent_selection.rs` (`select_agents`, même forme : roster + critères → `Result<_, SelectionError>`). Testable exhaustivement sans lancer Claude Code.
- **L'adaptation vit dans le bin** : parser le payload, résoudre et lire la config, sérialiser la réponse.
- **Idempotence acquise par construction** : la décision étant une fonction pure du payload, deux exécutions rendent le même verdict, et la précédence documentée est `deny > defer > ask > allow`. Deux `deny` identiques valent un `deny`. À tester, rien à concevoir.

## La règle de décision

```
target ∈ free_agents            → allow            (assistance, appelable par tous)
caller = None (fil principal)   → target == coordinator
caller == coordinator           → target ∈ leads ∪ agents des équipes sans lead
caller == un lead               → target ∈ agents de SON équipe
tout le reste                   → deny
```

Le `lead: Option<String>` de `TeamConfig` prend son sens dans les deux directions : il autorise le coordinateur à l'atteindre, et l'autorise lui à atteindre son équipe — sans permettre à un spécialiste de sous-déléguer latéralement.

**Le message de refus doit nommer la cible autorisée**, pas seulement refuser. Le spike l'a vérifié : face à une raison actionnable, le modèle réécrit son appel correctement du premier coup. Forme attendue :

> the declared topology allows only `dev-lead` from the main thread; hand the work to `dev-lead` instead

### Les cas où la policy se tait

Tous rendent `allow` :

- `orchestration.policy` absent ou `off` ;
- `coordinator` non renseigné (patterns `direct`, `blackboard`, `ring` — sans coordinateur, aucune topologie à violer) ;
- `orchestration` absent de la config, ou config absente / illisible ;
- payload illisible, `tool_input.subagent_type` absent.

**Un gate qui refuse parce qu'il n'a pas compris est un gate qu'on désinstalle le jour même.** Le refus vient d'une violation établie, jamais d'une incertitude.

`policy` est **indépendant** du champ `enabled` existant, qui gouverne le moteur `run --orchestrate`. À documenter explicitement : deux clés voisines aux effets distincts sont exactement le piège dont ce spec mesure le coût.

### Forme de la config

```yaml
orchestration:
  policy: strict            # off | strict — défaut : off
  coordinator: dev-lead
  teams:
    - agents: [core-specialist, provider-specialist, cli-specialist, ui-specialist, qa-specialist]
  free_agents: [Explore, Plan]
```

`policy` par défaut à `off` : aucune configuration existante ne change de comportement par le seul fait de mettre à jour ArmadAI.

## Installation

Une sous-commande cachée `armadai __claude-policy-gate` sert les deux véhicules.

- **Linker** : `armadai link --target claude` écrit l'entrée `PreToolUse` (`matcher: "Agent|Task"`) dans `.claude/settings.json`.

> **⚠ CORRECTION 2026-08-20.** Ce spec affirmait ici que `.claude/settings.json` étant versionnable,
> « la policy suit le dépôt et vaut pour l'équipe ». **C'est faux.** Le *hook* suivrait le dépôt,
> mais la *topologie* vit dans `.armadai/config.yaml`, qui **n'est pas versionné** sur ce projet.
> Deux développeurs auraient donc le même gate appliquant des règles différentes — ou aucune règle.
>
> Deux issues, à trancher hors de ce spec : versionner `.armadai/config.yaml` (décision de
> convention du projet), ou assumer que la policy est locale à chaque poste et retirer toute
> promesse d'application collective. En l'état, **la policy est locale.**
- **Plugin** : `crates/armadai/assets/claude-plugin/` gagne un `hooks/hooks.json` équivalent, avec `${CLAUDE_PLUGIN_ROOT}`, pour les projets jamais linkés.

**Le linker doit fusionner, jamais écraser.** `.claude/settings.json` peut porter d'autres hooks ; les perdre en silence serait inacceptable. La fusion doit aussi être idempotente : plusieurs `armadai link` ne dupliquent pas l'entrée.

La documentation confirme que les hooks de toutes les sources (settings utilisateur, projet, local, managed, plugins) **fusionnent** au lieu de s'écraser — les deux véhicules coexistent donc sans conflit, au prix d'une double exécution que l'idempotence rend inoffensive.

## Chemin de migration

Au premier `policy: strict`, rien de ce qu'un projet utilise réellement n'est déclaré. Ce spec n'ajoute aucun outil pour y remédier : **`armadai audit` répond déjà à la question**, via la règle `U02` (« ce sous-agent tourne mais n'est déclaré nulle part »), livrée le 2026-08-13.

Séquence :

1. `armadai audit` — `U02` liste les agents utilisés et non déclarés ;
2. déclarer chacun dans `teams` ou `free_agents` selon sa nature ;
3. passer `policy: strict`.

C'est aussi la raison de **ne pas** ajouter de mode `warn` au gate : l'audit *est* l'observation, hors bande et sans risque. Un `warn` intégré serait un second mécanisme d'observation pour le même besoin — et la mesure montre que le non-bloquant est ignoré.

Plus tard, `audit --propose` enrichi par l'usage (lots 2-3 de la feature observed-usage) automatiserait l'étape 2.

## Tests

- **Unitaires exhaustifs sur `check_delegation`** (fonction pure, aucun I/O) : fil principal → coordinateur (allow) et → spécialiste (deny) ; coordinateur → lead, → agent d'équipe sans lead, → agent d'une équipe à lead (deny) ; lead → son équipe (allow) et → une autre équipe (deny) ; spécialiste → quoi que ce soit (deny) ; `free_agents` depuis chaque position (allow) ; cible inconnue (deny) ; config sans `coordinator` (allow) ; `policy: off` (allow).
- **Gate bout-en-bout** via `assert_cmd` : payload JSON réel sur stdin → JSON attendu sur stdout. **Réutiliser les payloads capturés par le spike** plutôt que d'en inventer. Inclure le payload malformé → `allow`.
- **Idempotence** : deux exécutions sur le même payload → verdict identique.
- **Linker** : la fusion préserve un hook préexistant ; deux `link` successifs ne dupliquent pas l'entrée.
- **Essai réel sur ce dépôt** (demandé par Dimitri) : activer `policy: strict` sur ArmadAI avec sa topologie réelle et une session de travail effective, pour établir si la contrainte est vivable. Instructif par construction — la mesure montre 286 délégations vers `general-purpose`, qui devra être déclaré ou abandonné.

  **L'essai doit être mené en session interactive, pas en `claude -p`.** Le bench comportemental (8 sessions abouties, tâche multi-modules, tous outils) n'a produit **aucune** délégation en mode `-p` : l'agent y fait le travail lui-même. Le spike n'a obtenu une délégation qu'en la demandant explicitement. Un essai en `-p` ne solliciterait donc jamais le gate et donnerait une fausse impression de conformité.

Pas de cas gaveldrop : cet adaptateur ne réclame que les runs orchestrés (son `claims()` exige un `pattern`, son `build_command` produit toujours `run`, ses assertions portent sur des events JSON).

## Limites assumées

- **Un gate injoignable autorise tout, en silence.** Le hook référence l'exécutable par un chemin ;
  si celui-ci disparaît (nettoyage de `target/`, dépôt déplacé, binaire non installé), Claude Code
  n'obtient aucun avis et laisse passer. La défaillance va dans le bon sens — jamais de blocage
  injustifié — mais elle est **invisible** : rien ne distingue « aucune violation » de « gate
  absent ». Symptôme à connaître : plus aucun refus alors que la topologie devrait en produire.
  L'installation par le linker devra préférer un `armadai` résolu par le `PATH` à un chemin absolu
  vers `target/debug`, qui ne vaut que pour un poste de développement.

- **La contrainte ne vaut que pour Claude Code.** Les autres CLI cibles du linker (codex, copilot, gemini, opencode) n'ont pas de notion de sous-agent ni de hook équivalent. Aucun mécanisme portable n'est proposé ici.
- **Le gate ne juge pas la pertinence d'une délégation**, seulement sa légalité topologique. Envoyer une tâche TUI au `qa-specialist` reste autorisé si la topologie le permet — c'est le lot 4 (routes) qui traiterait cela.
- **La surface de hook n'est pas contractuelle.** Elle a été vérifiée empiriquement et sur la doc au 2026-08-20 ; un changement de Claude Code peut l'invalider. Le gate doit donc dégrader vers `allow` sur tout payload inattendu, ce que la section « cas où la policy se tait » impose déjà.
- **`agent_type` vide identifie le fil principal** par observation, non par contrat documenté. Si un jour ce champ portait autre chose, la règle du fil principal cesserait de s'appliquer — et dégraderait vers `deny` pour une cible non-coordinateur. À surveiller : c'est le seul endroit où une évolution silencieuse produirait un refus plutôt qu'une permission.

## Hors périmètre

- Mode `warn` et redirection par `updatedInput` (décisions 2 et 6).
- Les trois autres dimensions du moteur : scope fichiers, contraintes à état (budget, profondeur), routes sémantiques.
- La résolution de la contradiction documentaire entre `CLAUDE.md` racine et `.claude/CLAUDE.md` : le gate la rend inopérante en pratique, mais ne réécrit aucun des deux fichiers. Sujet distinct.
- La génération de la config depuis l'usage observé (lots 2-3 de la feature observed-usage).
