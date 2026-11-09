# Cadre du lot « interface dans les orchestrateurs »

Étape `frame`. Spec : `2026-11-04-orchestrator-ui-design.md` (commit `bf70471`) ; recherche : `2026-11-04-orchestrator-ui-research.md`. Aucune règle n'a été ajoutée à `armadai-rules` : rien n'est enregistré avant l'accord de Dimitri.

Limite de cette passe : lecture seule, sans shell. J'ai lu la spec, la recherche, `docs/wiki/flow-status.md`, `ci.yml`, `cli/graph.rs`, `compile/report.rs`. Je n'ai pas relu `.githooks/`, `tests/golden/`, `compile/patch.rs` ni le code de `flow_status/` : ce que j'en dis vient de la spec ou de `CLAUDE.md`, marqué « à vérifier au plan ».

## 1. Règles qui s'appliquent

| Sujet du lot | Règle | Source |
|---|---|---|
| `--json` stable | Aucune règle de `armadai-rules` ne fixe la stabilité d'un `--json`. Seul précédent : le `--json` du rapport de `link`, un tableau nu `[{target, element, status, reason}]`, **sans champ `schema`**, testé par `json_has_stable_fields`. Le `schema` et « ajout permis, retrait interdit » de la spec sont donc une convention nouvelle (spec 4.1, risque 6), pas une règle du dépôt. | `compile/report.rs` ; `CLAUDE.md`, Architecture (« conformance report, `--json` ») |
| `link` ne touche pas le réseau | `link` n'a jamais accès au réseau ; seuls `init --pack`, `pack *`, `skills sync/add`, `update` le font. S'applique à L2, L4, L5 (plugin et mod écrits localement, rien de téléchargé ; le HTML embarqué de la tranche ultérieure ne tire aucun CDN). | `CLAUDE.md`, Architecture |
| Golden | Le texte d'une ligne nouvelle (rapport de `link`, diagnostic, erreur) est proposé par le plan, validé par Dimitri ; les golden se mettent ensuite à jour mécaniquement, relus ligne à ligne contre la spec. Un `flow status` sans nouveau drapeau doit rester identique. | `armadai-rules`, Diagnostics, sorties et manifeste (Dimitri, 2026-10-27) ; spec 5, critère 5 |
| Log de `flow status` | Chaque événement est une ligne JSON de `.armadai/state/claude/<session>.jsonl` : timestamp, start/stop, id, agent, mot `STATE`, ou nom de commande. « Nothing else : no prompt, no reply text, no command arguments. » `state/` est ignoré par git. Ce lot n'ajoute aucun événement ; l'ajout d'un événement `tool` est le lot watch, et `--json` doit l'accueillir par un champ ajouté. Les chemins de documents produits se parcourent au disque et ne s'enregistrent pas (décision Q3 de la recherche). | `docs/wiki/flow-status.md`, What is recorded ; spec 2 (non-objectifs) |
| Format natif vérifié à la doc officielle | L'adapter-specialist vérifie chaque format natif dans la documentation officielle avant de l'écrire. Pour ce lot : `statusLine`, `subagentStatusLine`, `extraKnownMarketplaces`/`enabledPlugins`, format de plugin, API des mods. Tout fait **C** de la spec est un critère d'essai, pas une hypothèse de code. | `.claude/CLAUDE.md`, tableau d'équipe (adapter-specialist) ; spec légende V/C |
| Codes de diagnostic | kebab-case, préfixe de famille, sans verbe d'action, jamais renommés ; chaque code entre dans la table de `docs/wiki/project-format.md`. S'applique à la clé d'opt-in (L2, L5). | `armadai-rules` (Dimitri, 2026-10-27) |
| Schémas et wiki | Les JSON Schemas (`schemas/v1/`) se régénèrent par `cargo run -p armadai-model --example gen_schemas` ; le modèle reste pur. La clé d'opt-in y entre. Doc : `docs/wiki/flow-status.md` (branchement ccstatusline, limites) et `project-format.md`. | `CLAUDE.md`, Where things live |
| Fichiers partagés | `.claude/settings.json` s'écrit par `compile/patch.rs` ; `link` refuse en conflit (`merge_settings`) ; fichiers entiers par `manifest::write_files`. | `CLAUDE.md`, Architecture |
| Manifeste | `link-manifest.yaml` reste `version: 1`, champs optionnels seulement, un test lit un manifeste de la version précédente. S'applique si `unlink` doit retirer le plugin (L4) et le mod (L5). | `armadai-rules` (Dimitri, 2026-10-27) |
| Pureté | `armadai-model` reste pur ; `cargo build -p armadai-core` et `-p armadai-model` passent. Le JSON et la ligne se calculent dans le binaire (`flow_status/`). | `CLAUDE.md` ; `armadai-rules`, Barrière |
| Tests | Mutation vue rouge par test ; égalité exacte plutôt que `contains` ; aucun test sur le réseau ; `--bin armadai` ou `--test`, `--no-fail-fast` ; `env_lock()` non réentrant. Pour `--line` : « ne bloque jamais sur stdin » se teste avec une borne de temps choisie par rapport au seuil du défaut, répétée 5 fois. | `armadai-rules`, Tests et mutations |
| Essais réels | `HOME` et `ARMADAI_CONFIG_DIR` isolés ; rien de privé ni de chemin de machine dans fixtures, docs, wiki. Les mesures de Dimitri (ccstatusline, trust dialog) se reportent dans le wiki sans son chemin local ni sa configuration. | `armadai-rules`, Environnement |
| Gel | `frozen/armadai-audit/` : seulement le garder compilable si un crate dont il dépend change. Ce lot n'y touche a priori pas. | `CLAUDE.md`, Architecture |
| Git/PR | Une PR par livraison (L1 à L5), Conventional Commits, PR ouverte sans fusion sauf brief. | `armadai-rules`, Git et PR ; spec 8 |

Contradictions entre `armadai-rules`, `CLAUDE.md` et `.claude/CLAUDE.md` : aucune relevée pour ce lot.

Points d'attention du dépôt à ne pas perdre (sources ci-dessus) : le texte du wiki dit que la politique de log est « Nothing else » ; le critère d'une liste de documents produits (L5) la respecte seulement si le mod parcourt le disque lui-même et n'ajoute rien au journal. Le mod lisant `flow status --json`, la valeur de `schema` devient un contrat entre binaire et JS livré : voir question 2.

## 2. Ce que la spec demande et qu'aucune règle ne fixe

Décidé par Dimitri, déjà dans la spec, **à enregistrer** dans `armadai-rules` si Dimitri le veut (sinon la spec reste la source) : `--line` sans `session_id` se rabat sur le dernier journal du projet (Q1, 2026-11-05) ; opt-in par clé dédiée validée de `project.yaml` (Q2, 2026-11-05) ; L1 avant le lot watch animé (Q3, 2026-11-05).

Encore ouvert (questions ci-dessous) : noms des drapeaux et de la clé ; forme du `schema` ; repli de `--line` en sessions parallèles ; contenu, langue et marques de la ligne ; emplacement du plugin et de son marketplace ; dossiers parcourus et forme du JS livré (L5). Le plan doit aussi proposer le texte des lignes de rapport (`statusLine`, plugin, mod) : c'est déjà règle, Dimitri valide à la relecture du plan.

Observées, non décidées (à proposer, pas à enregistrer) : les marques `✓ ● ○ ? ✗ ~ -` du wiki (table unique à partager avec le watch) ; sortie sans ANSI par défaut ; sous-commandes cachées préfixées `__` (`__record-flow-event`, `__claude-policy-gate`) qui ne font jamais échouer bruyamment. Ces deux dernières habitudes sont imposées par rien d'outillé ; le lot les suit si Dimitri les confirme.

## 3. Règle durable candidate

Proposition (non écrite) : « Une sortie lisible par une machine (`--json`, schéma de fichier, manifeste) porte un champ de version ; on y ajoute des champs, on n'en retire ni renomme jamais sans nouvelle version ; un test lit un document écrit par la version précédente. » Elle généralise ce que `armadai-rules` dit déjà du manifeste (`version: 1`, champs optionnels) et la stabilité des codes. Elle est à dire à Dimitri : je ne l'écris pas. Une seconde candidate : « une clé de compilation à opt-in : absente, `link` n'écrit rien (diff nul sur ce point) ; présente et en conflit avec une valeur de l'utilisateur, `link` refuse et rapporte `not-compiled` » (spec 4.2, critères 6 et 8, appuyée sur le comportement de `merge_settings`).

## Réponses de Dimitri (2026-11-05, mot pour mot) et points laissés au plan

Les sept questions ci-dessous sont répondues ; elles restent pour mémoire des options.

- Q1 : « A : --json/--line/--format md + ui.status_line (Recommandé) ». Décidé : `flow status --json|--line|--format md`, `graph --format md`, clé `ui.status_line`.
- Q2 : « Chaîne nommée (Recommandé) ». Décidé : `"schema": "armadai.flow-status/1"` ; le `--json` de `link` n'est pas touché dans ce lot.
- Q3 : « Dernier journal, puis voir (Recommandé) ». Décidé : repli sur le dernier journal modifié ; on revoit après l'essai 10b.
- Q4 : « Avec couleurs ANSI ». Décidé : anglais, ANSI quand stdout est un terminal (pas ma recommandation). **Points ouverts pour le plan, non tranchés** : (a) la ligne garde-t-elle les marques `✓ ● ○ ? ✗ ~ -` en plus de la couleur ; (b) le widget ccstatusline et la `statusLine` de Claude Code lancent la commande avec stdout en tube, donc la détection de terminal ne colore jamais : faut-il forcer la couleur dans ces usages, et comment (drapeau, variable d'environnement, valeur de `ui.status_line`) ; à mesurer à l'essai 10b(5) (ANSI et multi-ligne dans le widget, encore **C**). Conséquence sur la spec : critères 4 et 4b disent « sans séquence d'échappement » ; ils ne valent que hors terminal, ou sans la couleur forcée, et sont à réécrire avec la réponse à (b). La spec (4.1, risque 9) parle aussi d'ANSI « sans couleur par défaut » : à réaligner.
- Q5 : « .armadai/plugins/armadai-ui/ » (pas ma recommandation). Vérifié au dépôt : `.armadai/.gitignore` exclut seulement `link-manifest.yaml`, `.migrated/`, `state/` (et `vars.local.yaml` que `link` ajoute à un fichier existant, `CLAUDE.md` Config). `.armadai/plugins/` serait donc **versionné**. Implications à traiter au plan : (1) le plugin est un fichier produit par `link` mais commité, alors que le manifeste qui dit ce que `link` a écrit est ignoré : sur un clone frais, `unlink` n'a pas le manifeste et ne saura pas que le plugin vient de `link` ; à décider : l'ignorer (ligne dans `.armadai/.gitignore`, comme `state/`) ou le garder versionné et accepter que `unlink` le laisse (la règle de repli par contenu de #342 existe pour d'autres fichiers : à lire avant de choisir) ; (2) `.armadai/` est le modèle canonique lu strictement par `armadai_model::load_checked` : à vérifier qu'un dossier `plugins/` n'y soulève pas un diagnostic `unknown-*` ni ne se confond avec les dossiers connus (`agents/`, `skills/`, `flows/`, `automations/`, `templates/`) ; sinon, le loader et le schéma ont une tâche de plus ; (3) manifeste en `version: 1`, champ optionnel pour les fichiers du plugin (règle existante) ; (4) pas de chemin de machine dans le plugin versionné (règle « rien de privé »). Je n'ai pas lu le loader : (2) est à vérifier, non constaté.
- Q6 : « Liste dans project.yaml, défaut docs/ (Recommandé) ». Décidé : clé de liste dans `project.yaml` (nom, p. ex. `ui.documents`, **à fixer au plan**, avec code de diagnostic et schéma), défaut `docs/`.
- Q7 : « JS embarqué qui appelle --json (Recommandé) ». Décidé : un JS inclus dans le binaire, qui lance `armadai flow status --json` ; la ligne de rapport dit que le mod exécute du JS et un sous-processus avec les permissions de l'utilisateur.
- Règles durables : acceptées (« Sortie machine versionnée, Clé de compilation opt-in »), écrites dans `armadai-rules`, section « Diagnostics, sorties et manifeste », source « Dimitri, 2026-11-05 ». Il faut relancer `armadai link --target claude` pour que `.claude/skills/armadai-rules/` suive.

Restent ouverts pour le plan : nom de la clé des documents ; forme exacte de la valeur de `ui.status_line` (booléen ou objet, si la couleur forcée y entre) ; la table de marques partagée avec le watch ; le texte de chaque ligne de rapport ; `tests/golden/` ; le sort du plugin dans `.gitignore` et `unlink` (Q5).

## Questions pour Dimitri (historique : toutes répondues, voir ci-dessus)

Classées de la plus bloquante à la moins bloquante.

**1. Noms des drapeaux et de la clé d'opt-in** (bloque : plan L1, L2, L3, schéma, wiki). Deux choses à nommer ensemble pour que `flow status` et `graph` parlent pareil.
- A. `flow status --json`, `flow status --line`, `flow status --format md`, `graph --format md` ; clé `ui.status_line: true` dans `project.yaml` (groupe `ui` pour accueillir plus tard `ui.panel`, `ui.mod`).
- B. Même chose côté CLI ; clé à plat `status_line: true`, puis `panel`, `mod` à plat.
- C. `flow status --format json|line|md` (un seul drapeau, trois valeurs) ; clé comme A. Moins de drapeaux, mais `--line` change de nature (texte unique) et se mêle à `--watch`.
Recommandation : A. Elle suit la spec telle quelle, et `--format md` reprend `graph --format md` (même enum `GraphFormat::Md`, pas de nouveau mot). Attention : le lot watch ajoute un `Ascii` à la même enum ; conflit de rebase attendu, pas de conflit de sens.

**2. Forme du champ `schema` du `--json`** (bloque : plan L1, tests de stabilité, mod L5).
- A. Entier de version à la racine, `{"schema":1,"sessions":[{...}]}`.
- B. Chaîne nommée, `{"schema":"armadai.flow-status/1", ...}` : le document dit de quoi il s'agit même sorti de son contexte.
- C. Tableau nu comme le `--json` de `link`, sans `schema` : cohérent avec l'existant, mais contredit la spec (`schema` versionné) et ne peut plus recevoir de champs de racine.
Recommandation : B. L'ajout de champs ne change pas la valeur ; seule une rupture la fait passer à `/2`. Dire aussi si `link --json` doit, un jour, être enveloppé de la même façon : je recommande de ne pas y toucher dans ce lot (changerait une sortie publiée).

**3. Repli de `--line` quand deux sessions tournent en parallèle** (bloque : plan L1, doc, critère 4b).
- A. Dernier journal modifié du projet, comme dans la spec ; la doc dit que la ligne peut montrer l'autre session.
- B. Dernier journal modifié **et** ouvert depuis moins de N minutes ; au-delà, ligne vide (évite d'afficher une session finie depuis longtemps).
- C. Si plusieurs journaux ont une activité récente, ligne vide plutôt que de deviner (règle « `?` plutôt qu'une supposition »).
Recommandation : A pour L1, C comme seuil à ajouter si l'essai 10b montre la confusion. A suit la décision déjà prise ; B et C demandent de fixer N ou « récent ».

**4. Contenu, langue et marques de la ligne** (bloque : texte à valider au plan L1, golden).
- A. Anglais, marques du wiki, sans ANSI : `deliver ● guard 3/4` (flow, marque de l'étape en cours, nom, avancement) ; `loops 1` seulement si > 0 ; ligne vide sans run.
- B. Comme A, mais texte nu sans marques (`deliver guard 3/4`) : survit à une police sans glyphes.
- C. Anglais avec couleurs ANSI quand stdout est un terminal.
Recommandation : A. Même table de marques que `flow status` et le watch ; l'anglais suit la langue de toutes les sorties du binaire (messages d'erreur et rapports sont en anglais) ; ANSI repoussé tant que 10b(5) n'a pas dit si ccstatusline l'accepte.

**5. Emplacement du plugin et de son marketplace local** (bloque : plan L4 ; non bloquant L1 à L3).
- A. `.armadai/ui-plugin/` (sous `.armadai/`, versionné ou non selon `.gitignore` ; `link` le compile, `unlink` le retire).
- B. `.claude/armadai-ui/` (où Claude Code lit déjà ses réglages ; mais il ne scanne pas `.claude/plugins/`, V, donc le chemin n'a rien de magique).
- C. `.armadai/plugins/armadai-ui/` (famille de plugins de ce genre à venir).
Recommandation : B ou C selon que vous voulez qu'un futur lot ajoute d'autres plugins ; je penche pour B : tout ce que `link` écrit vers Claude Code reste sous `.claude/`, et l'écriture par fichier entier passe déjà par `manifest::write_files`. À mesurer à l'essai 16 (forme **C** d'une source `directory` relative).

**6. Dossiers parcourus pour les documents produits (L5)** (bloque : plan L5 ; non bloquant avant).
- A. `docs/` du projet seul, récursif, fichiers `.md` modifiés depuis le début de la session.
- B. `docs/superpowers/{specs,plans}/` seulement (ce que l'équipe d'ArmadAI produit).
- C. Liste déclarée dans `project.yaml` (`ui.documents: [docs, notes]`), défaut A.
Recommandation : C avec défaut A. B n'aide pas les projets qui ne suivent pas cette arborescence. Plus de clé à valider (code de diagnostic, schéma), donc à ne prendre que si vous visez d'autres projets que celui d'ArmadAI.

**7. JS livré par le binaire (L5)** (bloque : plan L5).
- A. Un seul fichier JS inclus dans le binaire (`include_str!`), écrit tel quel par `link`, qui appelle `armadai flow status --json` : le JS ne lit pas le journal.
- B. Le JS lit directement `.armadai/state/claude/*.jsonl` : pas de dépendance au binaire à l'exécution, mais un deuxième lecteur du journal qui peut diverger de `runs()`.
- C. Pas de JS livré : seulement un exemple dans le wiki, copié par l'utilisateur.
Recommandation : A. Un seul lecteur du journal (`runs()`), et le contrat de `schema` (question 2) protège le JS. Le JS n'est écrit que sur opt-in, avec la ligne de rapport « JS exécuté avec vos permissions » (spec 4.3, critère 17). Précisez si vous acceptez que le mod appelle le binaire en sous-processus : c'est un process lancé par le mod avec vos permissions, à dire dans la ligne de rapport.

## À la sortie de ces réponses

Quand `lead` renvoie les réponses mot pour mot, j'enregistre dans `armadai-rules` (une ligne par règle, source « Dimitri, AAAA-MM-JJ »), après quoi `lead` relance `armadai link --target claude`. Je n'ai rien écrit dans la skill pour l'instant.

Non vérifié dans cette passe : `.githooks/`, contenu de `tests/golden/`, code de `compile/patch.rs` et de `flow_status/` (pas de shell ; seulement la lecture de six fichiers).
