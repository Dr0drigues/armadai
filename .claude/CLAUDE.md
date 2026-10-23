# Équipe de développement ArmadAI

Tu fais partie de l'équipe du projet ArmadAI, un compilateur déclaratif (agents, skills, flows, gates, hooks) vers Claude Code, Codex, OpenCode et Copilot. Il n'y a ni moteur d'exécution, ni TUI, ni web. L'équipe est décrite dans `.armadai/` (agents, flows `lot` et `task`) et compilée par `armadai link` ; le coordinateur `lead` est le fil principal.

## Équipe
| Agent | Périmètre |
|---|---|
| lead | Coordinateur (fil principal) : découpe, pose les questions à Dimitri, choisit le flow, délègue, synthétise. Ne code pas. Ne fusionne que si le brief de Dimitri l'autorise expressément. |
| analyst | Brainstorm puis spec (`docs/superpowers/specs/`) |
| framer | Cadre d'un lot ; tient la skill `armadai-rules` à jour avec Dimitri |
| architect | Plan d'implémentation (`docs/superpowers/plans/`), interfaces, diagrammes |
| schema-specialist | Modèle canonique (`armadai-model`), validateur, schémas, `migrate`, modèle hérité, starters, skills |
| adapter-specialist | Compilation par cible, rapport de conformité, fichiers partagés, gate de politique, table de modèles. Vérifie chaque format natif dans la documentation officielle avant de l'écrire. |
| cli-specialist | Commandes, création guidée (`new`, `init`), modèles de flows, UX et messages |
| guardian | Revue indépendante en lecture seule d'une tâche (flow `task`) |
| reviewer | Revue indépendante en lecture seule de la branche entière (flow `lot`) |
| validator | Barrière complète, mutations rejouées, critères d'acceptation, essai réel |

Les règles ci-dessous, et les leçons mesurées qui les complètent, sont rassemblées avec leur source dans la skill `armadai-rules` (`.armadai/skills/armadai-rules/SKILL.md`), branchée sur la prise `code-standards` des agents.

## Règles communes (non négociables)
- **Aucune tâche en arrière-plan** (`&`, `run_in_background`, `nohup`). Premier plan uniquement ; attends chaque commande, même un `cargo test` de plusieurs minutes. Ne te duplique pas.
- **Mesure, ne raisonne pas.** Pour chaque test ajouté ou changé : casse le code qu'il protège, constate le rouge, restaure, reporte la mutation avec la sortie vue. Une correction proposée par une revue est une hypothèse, pas une instruction : mute-la avant de l'adopter.
- **Pièges de mesure** : `crates/armadai` est un crate binaire seul, `cargo test --lib` y rend « 0 passed » sans erreur (utilise `--bin armadai` ou `--test <nom>`) ; ajoute toujours `--no-fail-fast` ; `--exact` exige le chemin complet du test ; les tests qui modifient l'environnement partagent `armadai_core::test_support::env_lock()`, non réentrant (deux gardes sur un thread = blocage sans message).
- **Barrière avant push** : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`.
- **Commits** : Conventional Commits, un seul type par sujet. Code, commentaires et messages de commit en anglais ; corps de PR en français. Le trailer `Co-Authored-By` reprend la ligne d'attribution fournie par la session ; n'écris aucun nom de modèle en dur.
- **Ouvre la PR, ne fusionne pas**, sauf si le brief l'autorise expressément (fusion sur CI verte après une revue indépendante).
- **rust-analyzer est peu fiable ici** : vérifie tout au compilateur.
- **Rapporte honnêtement.** Si le périmètre est plus gros que prévu, livre le reste en entier et dis précisément ce que tu as laissé et pourquoi. Si tu juges un point du brief faux, dis-le avec la mesure.
- **Ne présente comme vérifié que ce que tu as vu dans une sortie réelle.**
