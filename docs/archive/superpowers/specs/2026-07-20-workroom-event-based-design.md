# Workroom event-based (shell) + audit UX/UI transverse

> **Statut** : design validé (brainstorm 2026-07-20)
> **Cible** : rc.3 (avec C8, déjà livré). Spec séparé de C9/C8 ; consomme le protocole de coordination ArmadAI.
> **Base** : `release/1.0.0` (@ `7f00266`, après C8).

## 1. Objectif

La **Workroom** du shell (`src/shell/workroom.rs`) est un panneau latéral montrant la flotte d'agents (coordinateur > specialists) avec leur statut pendant une orchestration. Elle est aujourd'hui pilotée par `detect_mentions(text)` — un **scan heuristique flou** du texte streamé (noms d'agents + mots-clés « delegate »/« specialist »), fragile.

Objectif : piloter les statuts par les **marqueurs structurés du protocole ArmadAI** présents dans le stream (fiable, portable — le linker injecte le protocole quelle que soit la CLI), afficher les transitions en direct, rester **léger par défaut** avec un **drill-down** au détail. Plus un **audit UX/UI transverse** des deux TUI (shell + dashboard).

## 2. Machine à états pilotée par marqueurs (Lot A)

État actuel conservé : `AgentState { Working, Delegating, Done, Idle }`, `TrackedAgent { name, state, role, started_at, finished_at, spinner_frame }`, hiérarchie `AgentRole { Coordinator, Lead, Agent }`.

Nouvelle FSM alimentée en parsant chaque ligne du stream (extension de `parse_streaming_line`) :
- `<!--ARMADAI_DELEGATE:X-->` → coordinateur/lead émetteur = `Delegating` ; agent **X** = `Working` (`started_at`), X = « agent courant » ; transition enregistrée. (Comportement de `on_delegate` conservé + enrichi.)
- `<!--ARMADAI_META:status=...-->` → capture le statut/résultat de l'agent courant (alimente « dernière action »).
- `<!--ARMADAI_END-->` → agent courant → `Done` (`finished_at`) ; contrôle rendu au coordinateur. Le `END` final du coordinateur → coordinateur `Done`.
- `reset()` au tour suivant (inchangé).
- **`detect_mentions` supprimé** (le scan flou disparaît). Les appels dans `src/shell/app.rs` sont remplacés par le parsing marqueurs (via `parse_streaming_line`, qui gère déjà `DELEGATE` et est étendu à `META`/`END`).

## 3. Données enrichies — stockées, pas affichées inline (Lot A)

`TrackedAgent` gagne :
- durée dérivée de `started_at`/`finished_at` (déjà présents) ;
- **`last_action: Option<String>`** — extrait tronqué du contenu/statut entre le `DELEGATE` de l'agent et son `END` ;
- **`transitions: Vec<(AgentState, Instant)>`** — historique des transitions.

Ces champs alimentent le drill-down (Lot B). Ils ne sont **pas** rendus dans le panneau par défaut (anti-alourdissement).

## 4. Affichage (Lot B)

- **Défaut (léger)** : identique à aujourd'hui — hiérarchie coordinateur > specialists, état + spinner. Aucune durée/action inline.
- **Drill-down** : la Workroom devient **focusable** (flèches ↑/↓ pour surligner un agent, surlignage visible) ; **Entrée** ouvre un **overlay détail** (réutilise le pattern d'overlay de la command palette du shell) affichant durée, dernière action, historique des transitions de l'agent sélectionné ; **Échap** ferme. Le focus Workroom est activé par une touche dédiée (à choisir dans le plan, cohérente avec les raccourcis existants) et n'interfère pas avec la saisie normale du shell.

## 5. Polish visuel du panneau (Lot B)

Couleurs distinctes par état (Working/Delegating/Done/Idle) cohérentes avec le thème shell, spinner conservé, indentation hiérarchique, **légende/aide** courte, surlignage de sélection, style de l'overlay détail aligné sur l'existant.

## 6. Audit UX/UI transverse (Lot C)

Périmètre : **les deux TUI** — shell (`src/shell/tui.rs`, workroom, overlays, wizard) **et** dashboard (`src/tui/` : views, widgets, palette, shortcuts).

Approche **findings-first** :
1. Un subagent audite les deux TUI → **rapport priorisé** (`docs/proposals/` ou `docs/superpowers/`) : cohérence des couleurs/thème, espacements, cohérence des raccourcis et des barres d'aide, titres/bordures, accessibilité (contraste light/dark), redites.
2. J'applique les **fixes évidents/sûrs** (cohérence couleurs/thème, raccourcis harmonisés, légendes manquantes) en PR.
3. Les choix **subjectifs / plus lourds** (refontes de layout, nouveaux paradigmes de navigation) sont **remontés à l'utilisateur** pour arbitrage — pas appliqués unilatéralement.

Le rapport d'audit est un livrable en soi ; les fixes sûrs suivent dans la même PR (Lot C) ou une PR dédiée selon le volume.

## 7. Tests

- **Lot A** : FSM — feed d'une séquence de lignes de marqueurs (`DELEGATE:a`, texte, `META:status=complete`, `END`, `DELEGATE:b`, …) → asserts sur les états successifs, `last_action`, `transitions`, durées non nulles ; non-régression `reset()`/`init_from_config`/`on_complete` ; disparition de `detect_mentions` sans casser le rendu.
- **Lot B** : navigation de sélection (bornes, wrap), ouverture/fermeture de l'overlay, contenu de l'overlay reflète l'agent sélectionné ; le focus Workroom ne capture pas les frappes destinées au prompt shell hors mode focus.
- **Lot C** : pas de test unitaire (audit) ; les fixes de cohérence doivent compiler et ne pas régresser les tests TUI existants.

## 8. Contraintes CI

- Le shell/TUI est gated `tui`. Clippy 2 modes : `--no-default-features --features tui -- -D warnings` ET `--features tui,providers-api -- -D warnings`. Pour le dashboard web-adjacent, si Lot C touche `src/tui/` gated storage/web, vérifier aussi `--features tui,web,storage`. `cargo fmt -- --check`. `cargo test`.

## 9. Découpage

- **Lot A — FSM event-based** : marqueurs → états + données enrichies ; suppression `detect_mentions` ; câblage `app.rs` ; tests. (data/logic, testable)
- **Lot B — drill-down + polish** : sélection clavier + overlay détail + couleurs/légende. (UI interactif)
- **Lot C — audit UX/UI transverse** : rapport findings (shell + dashboard) → fixes sûrs, escalade du subjectif.

Chaque lot = une PR vers `release/1.0.0`, revue indépendante.

## 10. Hors périmètre

- Faire exécuter l'orchestration core (RunEvent sink) par le shell : hors scope — le shell pilote des CLI externes ; la source d'events est le protocole ArmadAI dans leur stream.
- Live vs replay dashboard TUI : la Workroom est déjà « live » (pendant le tour shell) ; pas de replay depuis storage ici.
- Refontes de navigation issues de l'audit : proposées, pas imposées (arbitrage utilisateur).
