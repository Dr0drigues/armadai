# L'équipe d'ArmadAI décrite avec ArmadAI : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** réécrire `.armadai/` du dépôt avec l'équipe et les flows de la spec, une skill `armadai-rules` sur la prise `code-standards`, puis lier vers Claude sur la branche pour relecture.

**Architecture:** uniquement du contenu sous `.armadai/` (agents, modèle `checker`, flows `lot` et `task`, skills) et les fichiers générés par `armadai link --target claude` (bloc `CLAUDE.md`, `.claude/agents/`, `.claude/skills/`, `.claude/settings.json`) ; mise à jour de `.claude/CLAUDE.md`. Aucun code Rust.

**Spec:** `docs/superpowers/specs/2026-10-23-dogfood-team-design.md`

## Global Constraints

- `project.yaml` : `schema_version: 1`, `name: armadai`, `targets: [claude]`, `coordinator: lead`, `policy: { delegation: strict, free_agents: [Explore, Plan, general-purpose] }`.
- Ids : `lead analyst framer architect schema-specialist adapter-specialist cli-specialist guardian validator` ; `dev-lead` et `qa-specialist` disparaissent.
- Modèle `templates/checker.md` repris du pack `crates/armadai-core/starters/concept-to-code/templates/checker.md`, adapté (rejets actionnables ; revue d'un diff ou d'une branche ; lecture seule) ; `guardian` et `validator` en sont des instances.
- Skills : `briefing`, `cadre`, `diagrams` reprises du pack et adaptées (specs dans `docs/superpowers/specs/`, plans dans `docs/superpowers/plans/`, cadre = `armadai-rules`) ; nouvelle skill `armadai-rules` avec `metadata: { armadai-slot: code-standards }`.
- Prise `code-standards` sur les trois spécialistes, `guardian`, `validator`.
- Flows `lot` et `task` exactement comme la spec §4.
- Prompts en français (comme les agents actuels du dépôt), agents qui rendent leurs questions à `lead` et ne délèguent jamais.
- `armadai validate` : 0 erreur ; seuls avertissements admis : `unknown-free-agent`.
- Aucune règle inventée dans `armadai-rules` : chaque règle cite sa source (fichier et section, ou mémoire).

---

### Task 1 : la skill `armadai-rules`

**Files:** Create `.armadai/skills/armadai-rules/SKILL.md`

- [ ] **Step 1 :** rassembler les règles depuis `CLAUDE.md` (Build & Test, Non-obvious facts, Git Conventions), `.claude/CLAUDE.md` (Règles communes), et les leçons de la mémoire `/Users/bl209054/.claude/projects/-Users-bl209054-work-misc-armadai/memory/feedback_{tests_qui_ne_peuvent_pas_echouer,harnais_de_mutation_qui_ment,lire_sous_le_verrou,bornes_temporelles_dans_les_tests,le_remede_d_une_revue_est_une_hypothese,relayer_un_fait_perime,independent_review,graphify_usage}.md`, plus les faits mesurés pendant la session (target cargo privé par worktree ; jamais `git stash` ; deux copies d'un crate dans un même `CARGO_TARGET_DIR` faussent les tests ; un `tail` masque un code retour ; ne présenter comme vérifié que ce qui est vu dans une sortie réelle).
- [ ] **Step 2 :** une ligne par règle, avec sa raison courte et sa source ; sections : Barrière, Tests et mutations, Git et PR, Environnement, Rapport. Description de la skill avec ses déclencheurs (avant d'écrire, tester, relire ou commiter du code dans ce dépôt).
- [ ] **Step 3 :** commit `docs(armadai): the team rules as a skill`.

### Task 2 : l'équipe, les flows et le link

**Files:** `.armadai/project.yaml`, `.armadai/agents/*`, `.armadai/templates/checker.md`, `.armadai/flows/{lot,task}.yaml` (supprimer `delivery.yaml`), `.armadai/skills/{briefing,cadre,diagrams}/`, `.claude/CLAUDE.md` ; fichiers générés par `armadai link --target claude`.

- [ ] **Step 1 :** écrire l'équipe et les flows (Global Constraints, spec §4) ; prompts des spécialistes repris de l'existant et revus (domaine, prise, rendre la main à `lead`).
- [ ] **Step 2 :** `armadai validate` avec le binaire de master (`cargo build -p armadai`, target privé) : 0 erreur, avertissements limités à `unknown-free-agent` ; `armadai graph` des deux flows relu.
- [ ] **Step 3 :** `armadai link --target claude` à la racine du worktree (HOME et ARMADAI_CONFIG_DIR isolés pour la config utilisateur) ; relire le bloc ajouté à `CLAUDE.md` (pas de contradiction avec le reste du fichier), les `.claude/agents/`, `.claude/skills/`, les hooks de `.claude/settings.json` (gate + enregistrement de flow) ; retirer les anciens `.claude/agents/dev-lead.md` et `qa-specialist.md` s'ils ne sont plus générés.
- [ ] **Step 4 :** `.claude/CLAUDE.md` : tableau d'équipe et règles alignés (renvoi vers la skill `armadai-rules` au lieu de dupliquer).
- [ ] **Step 5 :** barrière du dépôt (aucun code changé, mais des tests lisent peut-être le `.armadai/` du dépôt) ; commits `feat(armadai): describe the ArmadAI team with its own model` et `docs: …`.

### Task 3 : essai réel (contrôleur)

Mener le lot suivant (linter d'agents) avec cette équipe : `lead` = le coordinateur, `armadai flow status --watch` ouvert ; noter les liens manquants signalés par le gate.
