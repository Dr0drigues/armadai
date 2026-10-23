# L'équipe d'ArmadAI décrite avec ArmadAI (dogfooding du pack concept-to-code)

## 1. Intention

Le dépôt a son propre `.armadai/` (dev-lead + quatre spécialistes, flow `delivery` à plat), mais le travail réel suit un autre processus, porté par le coordinateur à la main : brainstorm → spec → plan → une tâche par sous-agent → revue indépendante par tâche → revue finale → essai réel avec Dimitri. Les règles du dépôt sont recopiées dans chaque brief (`repo-rules.md`, mémoire `feedback_*`). Dimitri a validé (2026-10-23) d'adapter le pack `concept-to-code` à ce processus : les règles deviennent une skill branchée sur une prise, les étapes deviennent un flow suivi par `armadai flow status`, et l'équipe sert de banc d'essai réel aux lots suivants (d'abord le linter d'agents).

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs :
1. Réécrire `.armadai/` du dépôt (cible `claude`, `coordinator: lead`, `policy.delegation: strict`) avec l'équipe et les flows du §4, en réutilisant le modèle `checker` du pack et ses skills utiles (`briefing`, `cadre`, `diagrams`).
2. Une skill `armadai-rules` (`metadata: { armadai-slot: code-standards }`) qui porte les règles du dépôt et les leçons mesurées (§5).
3. `armadai link` sur une branche ; le bloc ArmadAI généré dans `CLAUDE.md`, les agents `.claude/agents/` et les hooks de `.claude/settings.json` sont relus par Dimitri avant fusion.
4. `armadai validate` : 0 erreur, 0 avertissement.

Non-objectifs :
- Changer le code d'ArmadAI (si un manque du modèle apparaît, il est noté, pas corrigé ici).
- Boucler sur les N tâches d'un plan dans un flow (le modèle ne sait pas répéter un sous-flow) : le coordinateur garde la boucle par tâche ; chaque tâche est une exécution du flow `task`.
- OpenCode, Codex, Copilot pour cette équipe.
- Le pack livré `concept-to-code` n'est pas modifié.

Anti-objectifs :
- Une équipe qui contredit `CLAUDE.md` et `.claude/CLAUDE.md` du dépôt (deux sources de règles divergentes).
- Une délégation `strict` qui empêche ce que le coordinateur fait aujourd'hui (revue en lecture seule en parallèle d'une tâche, ré-revue ciblée).

## 3. Contraintes

- `free_agents` : le validateur ne connaît pas les agents intégrés (`Explore`, `Plan`, `general-purpose`) et lève `unknown-free-agent` (constaté en B2) ; les garder exige soit d'accepter l'avertissement, soit une issue sur le validateur. Choix : les garder (le dépôt les utilise) et ouvrir l'issue ; l'objectif 4 devient « 0 erreur, avertissements `unknown-free-agent` seulement ».
- Les noms d'agents existants (`schema-specialist`, `adapter-specialist`, `cli-specialist`, `qa-specialist`, `dev-lead`) sont référencés par `.claude/CLAUDE.md` et la mémoire : les spécialistes gardent leur id ; `dev-lead` devient `lead` (coordinateur) et `.claude/CLAUDE.md` est mis à jour en conséquence.
- Le bloc `CLAUDE.md` généré par `link` s'ajoute au `CLAUDE.md` versionné du dépôt : il ne doit pas répéter ni contredire ses sections.

## 4. Équipe et flows

| Agent | Origine | Rôle |
|---|---|---|
| `lead` | écrit | coordinateur (fil principal) : découpe, pose les questions à Dimitri, choisit le flow, synthétise ; ne code pas |
| `analyst` | du pack, adapté | brainstorm puis spec (`docs/superpowers/specs/`) ; skill `briefing` |
| `framer` | du pack, adapté | cadre et règles ; tient `armadai-rules` à jour avec Dimitri ; skills `cadre` |
| `architect` | du pack, adapté | plan d'implémentation (`docs/superpowers/plans/`), interfaces, revue focus ; skill `diagrams` |
| `schema-specialist`, `adapter-specialist`, `cli-specialist` | existants, prompts revus | constructeurs par domaine ; prise `code-standards` |
| `guardian` | instance de `checker` | revue indépendante d'une tâche ou d'une branche (spec + qualité), lecture seule ; prise `code-standards` |
| `validator` | instance de `checker` (ex-`qa-specialist`) | barrière complète, mutations, préparation de l'essai réel ; `shell: ask` ; prise `code-standards` |

Flow `lot` (start `spec`) : `spec(analyst) → frame(framer) → plan(architect) → [tâches] → review(guardian: approved|rejected) → validate(validator: approved|rejected)` ; `review → plan` si `rejected` (correction de plan) ; `validate → plan` si `rejected`.
Flow `task` (start `build`) : `build` en fan-out vers les trois spécialistes (une étape par domaine, `build-schema`, `build-adapter`, `build-cli`), chacune → `guard(guardian: approved|rejected)` ; `guard → build-*` si `rejected` (même domaine : un lien par domaine).

`qa-specialist` disparaît comme agent (son rôle est `validator`) ; la mémoire et `.claude/CLAUDE.md` sont mis à jour.

## 5. La skill `armadai-rules`

Contenu, source par source :
- `CLAUDE.md` (barrière, crate binaire seul, Conventional Commits, master-only, squash, fusion sur accord) ;
- `.claude/CLAUDE.md` (règles communes non négociables) ;
- leçons de la mémoire, en une ligne chacune avec leur raison : tests qui ne peuvent pas échouer (mutation obligatoire), restauration qui garde la mtime (touch), lecture sous le verrou, bornes temporelles des tests, remède d'une revue = hypothèse, ne pas relayer un fait non vérifié, target cargo privé par worktree, jamais `git stash`, deux copies d'un crate dans un même `CARGO_TARGET_DIR` faussent les tests, `tail` qui masque un code retour.
Une source par règle ; aucune règle inventée.

## 6. Critères d'acceptation

1. `armadai validate` : 0 erreur ; seuls avertissements tolérés : `unknown-free-agent` (issue ouverte).
2. `armadai link` sur la branche : diff relu (bloc `CLAUDE.md`, `.claude/agents/`, `settings.json`) ; Dimitri relit le bloc avant fusion.
3. Barrière du dépôt verte (aucun code changé, mais les étalons qui lisent le `.armadai/` du dépôt, s'il y en a, sont vérifiés).
4. Essai réel : le lot suivant (linter d'agents) est mené avec cette équipe, suivi par `armadai flow status`.

## 7. Risques

- Le flow `task` ne modélise pas « une tâche = un domaine » de façon certaine : une tâche qui touche deux domaines lance deux étapes `build-*` ; `flow status` les montrera en fan-out.
- Le gate `strict` bloquera une délégation non déclarée ; le premier essai réel dira s'il manque des liens.
