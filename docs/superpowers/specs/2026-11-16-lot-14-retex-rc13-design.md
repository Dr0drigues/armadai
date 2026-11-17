# Lot 14 : défauts relevés après la rc.13

## 1. Intention

Un retour d'expérience sur un projet réel (équipe de plusieurs agents, un chantier par worktree, lots menés dans des worktrees liés au chantier) a relevé cinq défauts d'ArmadAI après la rc.13 : un journal non partagé, un texte compilé qui contredit un coordinateur prudent, un état de `grant` partagé entre worktrees, une barre d'état enchaînée écartée sans un mot, et un plugin d'interface non chargé alors que `link` ne dit rien. Chacun fait perdre du temps à l'équipe ou donne une fausse assurance. Décision de Dimitri, 2026-11-16 : « Défauts avant C, évolutions après ». Ce lot ne traite que ces cinq défauts.

La source du retour d'expérience est un dépôt privé : cette spec décrit les cas de façon générique et n'en recopie rien.

Chaque défaut est mesuré ci-dessous sur `master` (`6c3de66`, worktree du lot) ; une mesure corrige parfois le dire du retour d'expérience, et c'est signalé.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs :
1. D1. Faire viser un même journal à toutes les sessions d'un chantier, quelle que soit la topologie des worktrees.
2. D2. Rendre le texte compilé de `sessions.journal` et de `session_link` compatible avec un coordinateur qui remonte toute décision à l'utilisateur.
3. D3. Donner à chaque checkout son propre état de `grant`.
4. D4. Ne plus écarter en silence la barre d'état de l'utilisateur.
5. D5. Avertir au `link` quand le marketplace `armadai-ui` du registre global ne mène pas au plugin du projet.

Non-objectifs (évolutions décidées pour après le lot C) : `journal add` délégable à un agent sans marqueur de flow, `--ts` sur `journal add`, entrées de journal multi-lignes, sévérités et plafond de tours de revue, `wait:`, état `interrupted` dans `flow status`, `lead` sans Bash au niveau du harness, flow `sync`, et tout ce que le retour d'expérience propose d'autre (commits de merge, accords `ask` non demandés : à vérifier à part).

Anti-objectifs : aucun changement de comportement pour un projet qui n'utilise pas les clés concernées ; aucune nouvelle version du manifeste ou d'un format machine sans champ de version conservé ; aucun accès réseau ajouté à `link`.

## 3. Contraintes

- Règles de `armadai-rules` qui pèsent : clé de compilation en opt-in (absente, `link` n'écrit rien) ; un hook de sécurité ne rend jamais `allow` sur un état illisible ou un doute ; un niveau de `grant` ne lève jamais un `deny` ; manifeste en `version: 1`, champs optionnels seulement ; sortie machine versionnée ; code de diagnostic en kebab-case, publié jamais renommé, ajouté à la table de `docs/wiki/project-format.md` ; le texte d'une ligne nouvelle de sortie est proposé par le plan et validé par Dimitri ; rien de privé dans le dépôt public ; chaque test se mute ; essais au binaire avec `HOME`, `ARMADAI_CONFIG_DIR` et `CLAUDE_CONFIG_DIR` isolés.
- `armadai-model` reste pur ; les schémas JSON se régénèrent (`cargo run -p armadai-model --example gen_schemas`).
- Le format du registre global de Claude Code (D5) n'est pas décrit dans le dépôt : il se vérifie dans la documentation officielle avant d'être lu, et l'extrait se consigne dans le plan.

## 4. Conception

### D1. Journal non partagé

Mesure. `journal::state_dir(root)` (`crates/armadai/src/journal.rs:51-65`) redirige vers `.armadai/state` du checkout principal seulement si `root` est un worktree lié et que le même projet du checkout principal porte `.armadai/project.yaml` ; sinon il rend le `.armadai/state` de `root`. Les tests `state_dir_follows_the_main_checkout_only_when_it_has_a_project` et `a_project_in_a_subfolder_keeps_its_own_journal_in_the_main_checkout` le fixent. Le retour d'expérience est donc exact : si le checkout principal n'a pas de projet, chaque worktree écrit son journal. Seul `cli/journal.rs` appelle `journal::add` et `journal::show` ; `state_dir` n'a pas d'autre appelant. Le modèle `Sessions` (`crates/armadai-model/src/types.rs:120-126`) ne porte que `journal: bool`, avec `deny_unknown_fields`.

Comportement attendu (décidé par Dimitri, 2026-11-16 : « Dossier des journal*.jsonl »). Un réglage optionnel `sessions.journal_dir` dans `project.yaml` désigne le dossier qui contient les fichiers du journal (`journal.jsonl`, `journal-<lot>.jsonl`). Il prend le pas sur la redirection vers le checkout principal. Un chemin relatif se résout depuis la racine du projet ; un chemin absolu est admis. Toutes les sessions d'un chantier lisent le même `project.yaml` (un worktree de lot a son `.armadai` lié au chantier), donc visent le même dossier. Absent, rien ne change. `journal add` et `journal show` refusent, avec un message qui nomme le chemin, un dossier absent ou lié par un lien symbolique, comme ils refusent déjà un `state/` lié. Le dossier n'est pas créé par `journal add` s'il n'existe pas : l'équipe prépare le worktree dédié, et un dossier absent est le plus souvent une faute de frappe.

Crates : `armadai-model` (champ, validateur, schémas, doc `project-format.md`), `armadai` (`journal.rs`, `cli/journal.rs`), docs `flow-status.md` et `link.md`.

### D2. Texte compilé

Mesure. Le retour d'expérience attribue « instead of asking the user » aux deux sections ; le code dit autre chose. Section `### Journal` (`crates/armadai/src/compile/claude.rs:619`) : « Before a decision, read `armadai journal show --lot <lot> --last 20` ; … After a decision, record it: `armadai journal add --lot <lot> --kind decision --by {coordinator} "<one line>"` ». Elle ne dit rien d'éviter l'utilisateur, mais elle ordonne au coordinateur de lancer ces commandes. Section `### Sessions` (`claude.rs:563-585`), par flow : « If `{to}` is not listed or does not answer, use the journal (`armadai journal show`, `armadai journal add`) instead of asking the user. » C'est ici que la contradiction se trouve. Tests existants : `a_session_link_adds_the_sessions_section_with_the_journal_fallback_only`, `PARA_TAIL_A`, la constante `T15`.

Comportement attendu (décidé par Dimitri, 2026-11-16 : « Journal + décision à l'utilisateur »). Le texte compilé ne dit plus de contourner l'utilisateur ; une décision qui sort du périmètre du coordinateur reste soumise à l'utilisateur, le journal sert à la consigner. Le texte ne prétend pas non plus que le coordinateur lance lui-même les commandes `armadai` : il décrit ce qui doit être consigné ou lu, sans désigner qui tape la commande (déléguer l'écriture à un autre agent est une évolution hors lot). Les commandes restent écrites telles quelles, pour qu'un agent qui a le droit de les lancer sache lesquelles. Les golden et tests de `claude.rs` suivent, relus ligne à ligne.

Crate : `armadai` (`compile/claude.rs`, goldens), doc `link.md` (section « The lot journal and the session link »).

### D3. État de `grant` partagé

Mesure. `grant` et `revoke` (`crates/armadai/src/cli/grant.rs`, `open_project`, l. 385-390) et le hook (`grant_hook.rs:265`) prennent la racine par `policy_gate::resolve_project(cwd)` (`policy_gate.rs:72-74`) : la première racine qui porte `.armadai/project.yaml`, en suivant les liens (`is_file`), sans `canonicalize`, puis, à défaut, la racine du checkout principal. L'état est `root.join(".armadai/state/grants.yaml")` (`grant_hook.rs:42`, `write_state` `grant.rs:362-371`, `read_state` `grant_hook.rs:255`). Un worktree dont `.armadai` est un lien vers un autre checkout écrit donc dans cet autre checkout : `docs/wiki/link.md:259-263` le documente comme le comportement actuel. Un worktree qui porte son propre `.armadai` réel a déjà son propre état. Un worktree sans `.armadai` lit l'état du checkout principal. Aucun test de grant ne met en scène un worktree. La fin d'un niveau par `--until <étape>` lit les journaux de flow sous la même racine (`marked_since_in_logs`), donc eux aussi traversent le lien.

Comportement attendu (décidé par Dimitri, 2026-11-16 : « Sous le dossier git »). Un niveau ouvert dans un checkout ne vaut que pour ce checkout. L'état est rangé par checkout, non par cible d'un lien : sous le dossier git du checkout (même précédent que le pin de pack, `<git dir>/armadai/pack.lock`). `grant`, `revoke`, `grant` en liste et le hook lisent et écrivent le même endroit. Hors dépôt git, repli sur l'emplacement actuel. Un ancien `.armadai/state/grants.yaml` n'est plus lu : un niveau ouvert avant la mise à jour est fermé (direction sûre, rien n'est autorisé à tort), et `grant` le dit une fois s'il le trouve. Les gardes de `link` (les deux règles `deny` sur `grants.yaml` et sur `armadai grant`) et `names_grant_machinery` couvrent le nouvel emplacement ; `unlink` le nettoie. `docs/wiki/link.md:259-263` est réécrit. À mesurer par le plan, avant d'écrire : que les deux règles `deny` de `link` (et `names_grant_machinery`) couvrent bien le nouveau chemin, qui sort du dossier du projet ; si la mesure montre que `Edit(...)` ne s'y applique pas, le plan le dit à Dimitri avant de continuer.

Crate : `armadai` (`cli/grant.rs`, `claude_adapter/grant_hook.rs`, `cli/unlink.rs`, `compile/claude_settings.rs` pour les `deny`), `worktree.rs`.

### D4. Barre d'état enchaînée

Mesure. `CHAIN_LIMIT` vaut 1 s (`crates/armadai/src/cli/flow.rs:607`) ; il n'est utilisé qu'à `flow.rs:618`. `run_user` (`claude_adapter/statusline_chain.rs:45`) rend `None` si la commande sort en erreur, dépasse la limite ou ne ferme pas son tube ; la barre de l'utilisateur disparaît alors sans message, seule la ligne d'ArmadAI s'affiche, et la commande sort toujours 0. Le test `statusline_prints_the_flow_line_alone_when_the_user_bar_fails_or_is_too_slow` (`tests/flow_status_cli.rs:863`) fixe ce comportement. Hors Unix, `run_user` rend toujours `None`. La mesure de durée d'une barre tierce réelle (0,5 à 2 s) vient du retour d'expérience, non rejouée ici.

Comportement attendu (décidé par Dimitri, 2026-11-16 : « 3 s + marqueur visible »). La limite passe à 3 s. Quand la barre de l'utilisateur échoue ou dépasse la limite, la ligne d'ArmadAI porte un court marqueur visible qui le dit, au lieu de rien. Sans barre de l'utilisateur configurée, aucun marqueur. Le code retour reste 0 (Claude Code vide la barre sur tout autre code).

Crates : `armadai` (`cli/flow.rs`, `claude_adapter/statusline_chain.rs`), doc `link.md` ou `flow-status.md`.

### D5. Marketplace `armadai-ui`

Mesure. Aucun code du dépôt ne lit `known_marketplaces.json`. `link` écrit dans `.claude/settings.json` du projet `extraKnownMarketplaces["armadai-ui"]` (source `directory`, chemin `./.armadai/plugins/armadai-ui`) et `enabledPlugins["armadai-ui@armadai-ui"] = true` (assertions dans `tests/link_subagent_status_line.rs:131-160`). Le rapport donne `compiled` pour le plugin de `ui.subagent_status_line` (« Claude Code loads it once you trust the folder », `cli/link.rs` ~l. 270-310) et `approximated` pour le panneau et la bande (« never compiled: link cannot know whether the mod loads »). Le retour d'expérience dit `compiled`/`approximated` pour `ui.panel` et `ui.flow_band` : les deux étiquettes existent bien, mais aucune ne détecte un registre global qui garde un `armadai-ui` ailleurs. Le mécanisme exact de collision (le registre de l'utilisateur gagne sur la déclaration du projet) est celui que rapporte le retour d'expérience, à vérifier dans la documentation officielle avant de coder.

Comportement attendu (décidé par Dimitri, 2026-11-16 : « Avertir au link »). Nommer le marketplace par projet est écarté de ce lot. Quand une clé `ui.*` qui compile le plugin est présente, `link` lit le registre global de Claude Code (`$CLAUDE_CONFIG_DIR/plugins/known_marketplaces.json`, sinon `$HOME/.claude/plugins/`), sans réseau et sans l'écrire. S'il y trouve un `armadai-ui` dont la source est un dossier absent ou autre que celui du projet, `link` ajoute au rapport une ligne d'avertissement (`approximated`, avec la raison et la commande qui répare : `claude plugin marketplace remove armadai-ui`, puis la réinscription du projet). Un registre absent, illisible ou d'un format inattendu ne produit aucun avertissement et n'échoue jamais `link`. À mesurer par le plan : le format de `known_marketplaces.json` et le mécanisme de collision, dans la documentation officielle de Claude Code (extrait exact consigné dans le plan).

Crate : `armadai` (`cli/link.rs`, `compile/claude.rs`), doc `link.md`.

## 5. Critères d'acceptation

D1
1. Avec `sessions.journal_dir` défini et un chantier dont le checkout principal n'a pas de `project.yaml`, une entrée écrite par `armadai journal add` dans une session A apparaît dans `armadai journal show` d'une session B (autre worktree du même chantier), et le fichier est dans le dossier désigné.
2. Sans `sessions.journal_dir`, le journal est écrit aux mêmes endroits qu'avant (redirection vers le checkout principal quand il a un projet, sinon `.armadai/state` propre).
3. Un `sessions.journal_dir` qui désigne un dossier absent ou un lien symbolique fait échouer `journal add` et `journal show` avec un code non nul et un message qui nomme le chemin ; aucun fichier n'est écrit.

D2
4. Le `CLAUDE.md` compilé pour un projet avec `sessions.journal` et un `session_link` ne contient plus l'expression « instead of asking the user », et dit qu'une décision hors périmètre revient à l'utilisateur.
5. Le même texte ne dit pas que le coordinateur lance lui-même les commandes `armadai journal` ; les deux commandes y figurent toujours.

D3
6. Après `armadai grant <niveau>` dans le worktree A, `armadai grant` (liste) dans un worktree B dont `.armadai` est un lien vers le même chantier montre le niveau fermé, et le hook de B ne rend aucun avis ; A garde son niveau ouvert.
7. `armadai revoke` dans A ne ferme pas un niveau ouvert dans B ; `armadai unlink` dans un checkout supprime l'état de ce checkout et laisse celui des autres.

D4
8. Une barre d'utilisateur qui met 2 s à répondre est affichée avant la ligne d'ArmadAI (elle était écartée au-delà de 1 s).
9. Une barre d'utilisateur qui sort en erreur ou dépasse 3 s laisse la ligne d'ArmadAI avec un marqueur visible, et la commande sort 0.
10. Sans barre d'utilisateur configurée, la sortie est celle d'avant, sans marqueur.

D5
11. Avec un registre global (isolé par `CLAUDE_CONFIG_DIR`) dont `armadai-ui` pointe vers un dossier absent, `armadai link` rapporte une ligne d'avertissement qui nomme le marketplace, la raison et la commande de réparation ; le code de sortie de `link` est inchangé.
12. Avec un registre absent, illisible ou dont `armadai-ui` pointe vers le plugin du projet, `armadai link` n'ajoute aucune ligne.

Essai réel avec Dimitri : un journal partagé entre deux worktrees d'un chantier réel (D1), puis la barre d'état avec une barre tierce lente (D4).

## 6. Risques

- Douze critères : c'est la limite. Le lot se coupe naturellement en deux (D1 et D2, texte et journal ; D3, D4 et D5) si le plan s'avère trop gros.
- D1. Un chemin absolu dans `project.yaml` est propre à une machine et ne se partage pas : la doc recommande un chemin relatif. Un journal dont les anciennes entrées restent dans les journaux locaux n'est pas migré (non-objectif) : le dire dans la doc.
- D2. Les projets qui comptent sur le repli « journal plutôt que l'utilisateur » changent de comportement à la prochaine `link` ; à annoncer dans les notes de version.
- D3. Les `deny` de `link` visent `.armadai/state/grants.yaml` : un chemin sous le dossier git sort du dossier du projet, la règle `Edit(...)` peut ne plus s'y appliquer. À mesurer avant de choisir l'emplacement (voir Q3). Deux niveaux ouverts avant la mise à jour sont fermés en silence sans le message de `grant`.
- D4. Une limite de 3 s ralentit la barre quand la commande tierce est lente ; Claude Code peut avoir son propre délai, non mesuré ici.
- D5. Le format de `known_marketplaces.json` est celui d'un outil tiers, sans garantie de stabilité : la lecture doit tout ignorer en cas de doute. Le mécanisme exact par lequel le registre global l'emporte sur la déclaration du projet n'est pas mesuré ici.
- Un test sur un lien ou un registre se mute (supprimer la lecture du registre, rétablir la racine par lien) : les tests de ce lot sont à écrire avec leur mutation.

## Questions ouvertes

Les cinq choix produit sont tranchés (Dimitri, 2026-11-16 : D1 dossier des `journal*.jsonl`, D2 journal et décision à l'utilisateur, D3 sous le dossier git, D4 3 s et marqueur visible, D5 avertir au `link`). Reste à mesurer par le plan :
- la garde `deny` sur le nouveau chemin de l'état de `grant` (D3) ;
- le format du registre global de Claude Code et le mécanisme de collision, dans la documentation officielle (D5) ;
- la garde de lien symbolique du journal sur le composant `.armadai` d'une racine (D1) ;
- le texte exact des lignes nouvelles de sortie (marqueur de D4, avertissement de D5, message de `grant` sur un ancien état), proposé par le plan et validé par Dimitri.
