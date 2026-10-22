# Étape 4, lot B0 : modèles d'agents (templating)

## 1. Intention

Plusieurs agents d'une équipe ne diffèrent que par quelques phrases : `guardian` et `validator` du pack « concept-to-code » sont tous deux des vérificateurs qui lisent, n'écrivent pas et rendent un verdict au format fixe. Aujourd'hui chacun est un fichier complet, recopié à la main ; une correction du verdict doit être faite partout. B0 permet d'écrire **une fois** un modèle d'agent paramétré, et d'en tirer plusieurs agents par quelques lignes. C'est la première marche de l'idée « config comme source de vérité » (agents générés depuis un modèle et des valeurs). Dimitri l'a placé avant B1 pour s'en servir dans le pack de B2.

Tout le rendu se fait **au chargement**, dans `armadai-model` : après `load_checked`, un agent instancié est un agent ordinaire. Les adaptateurs (Claude, OpenCode, Codex, Copilot), le gate, `graph`, `inspect` n'ont rien à connaître des modèles.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs :
1. `.armadai/templates/<id>.md` : un modèle d'agent. Frontmatter = les mêmes clés qu'un agent, **toutes facultatives** ; corps = prompt. Les deux peuvent contenir `{{var}}`.
2. Un agent `.armadai/agents/<id>.md` peut déclarer `from: <modèle>` et `vars: { nom: valeur }` (valeurs : chaînes). Il hérite du modèle rendu avec ses `vars`.
3. Fusion : les clés de frontmatter de l'instance **remplacent** celles du modèle, clé de premier niveau par clé (une `capabilities` de l'instance remplace toute celle du modèle) ; le corps de l'instance, s'il n'est pas vide, s'ajoute après le corps rendu du modèle, séparé par une ligne vide.
4. Après fusion, l'agent doit être valide comme tout agent (`description` obligatoire, etc.) ; les diagnostics pointent le fichier de l'instance, et celui du modèle quand l'erreur vient du modèle.
5. L'écrivain canonique réécrit une instance **sous sa forme d'instance** (`from` + `vars` + ses propres clés + son propre corps), jamais aplatie.
6. `list` affiche une section `Templates` et, pour un agent instancié, son modèle ; `inspect <agent>` affiche l'agent rendu et `from: <modèle>`.
7. `armadai new agent <id> --from <modèle> --var k=v` (répétable) écrit une instance ; le menu guidé propose de partir d'un modèle du projet quand il y en a.
8. `init --pack` installe `templates/*.md` d'un pack natif ; schémas JSON (`template.schema.json`, clés `from` et `vars` dans `agent.schema.json`), doc `docs/wiki/project-format.md`, CHANGELOG.

Non-objectifs du MVP (décision de Dimitri, 2026-10-22 : livrer un MVP pour finir les lots B0 → B2, puis revoir l'ensemble ; ces points sont reportés, pas refusés) :
- Modèle qui hérite d'un autre modèle (`from` dans un modèle) : erreur.
- Logique dans les modèles (conditions, boucles, filtres, valeurs par défaut de variables).
- Modèles de flows, de skills ou de commandes.
- Génération de N agents depuis une liste dans `project.yaml` : une instance = un fichier (quatre lignes suffisent).
- Les gabarits de `armadai new agent` livrés dans le binaire (`crates/armadai/templates/*.md`, `{{name}}`, `{{stack}}`, `{{description}}` rendus à la création) restent ce qu'ils sont ; ce sont des points de départ copiés, pas des modèles liés.

Anti-objectifs :
- Un modèle modifié qui change un agent **sans** que `link` le reflète, ou l'inverse : le rendu est pur, refait à chaque chargement.
- Un `{{` légitime dans un prompt (exemple Handlebars, Jinja, Mustache) devenu impossible à écrire.
- Une commande (`new`, `migrate`) qui aplatit une instance en réécrivant le projet.

## 3. Contraintes

- `crates/armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars).
- Syntaxe : `{{nom}}`, espaces autorisés à l'intérieur (`{{ nom }}`), `nom` = règle d'id existante. Échappement : `\{{` produit `{{` littéral et n'est pas une variable.
- Substitution dans le corps et dans **les valeurs chaînes** du frontmatter (y compris dans `native`) ; jamais dans les clés.
- Le rendu du frontmatter se fait sur les **valeurs**, après analyse en YAML générique et avant l'analyse typée, jamais sur le texte YAML : une variable qui contient `:` ou un saut de ligne reste une chaîne et ne peut pas injecter de structure.
- Les modèles ne sont pas des agents : ils ne sont pas compilés, ne peuvent pas être une étape de flow, un `coordinator` ni un `free_agents`.

## 4. Décidé avec Dimitri

- Ajouter le templating, avant B1 (2026-10-22).
- B0 est un MVP : les non-objectifs du §2 sont des améliorations reportées après les lots B, pas des refus.
- Forme proposée et montrée : `templates/checker.md` + `agents/guardian.md` (`from: checker`, `vars:`) ; frontmatter de l'instance remplace clé par clé ; corps ajouté à la suite ; variable manquante = erreur ; prévoir un échappement.

## 5. Diagnostics (codes stables, nouveaux)

| Code | Sévérité | Cas |
|---|---|---|
| `unknown-template` | erreur | `from` nomme un modèle absent (avec suggestion, comme `unknown-agent`) |
| `missing-template-var` | erreur | `{{x}}` dans le modèle sans `x` dans `vars` |
| `unused-template-var` | avertissement | une clé de `vars` qu'aucun `{{…}}` du modèle n'utilise |
| `vars-without-template` | erreur | `vars` sans `from` |
| `nested-template` | erreur | `from` dans un modèle |
| `template-agent-collision` | erreur | un modèle et un agent ont le même id (évite toute ambiguïté dans `list`, `new`) |
| `bad-id` | erreur | id de modèle ou nom de variable invalide |

Une erreur de clé inconnue ou de type dans le frontmatter d'un modèle est signalée sur le modèle, comme pour un agent. Une variable dont le nom est dans `vars` mais dont l'agent n'est jamais rendu (modèle introuvable) ne produit que `unknown-template`.

## 6. Laissé au constructeur

- Les types internes (où vit la forme d'instance pour l'écrivain), à condition que `Project.agents` contienne les agents rendus.
- Le texte exact des messages, dans le style des diagnostics existants.
- La présentation de `list` / `inspect`.

## 7. Critères d'acceptation

1. Barrière complète verte ; une mutation vue rouge par test ajouté.
2. Un projet avec `templates/checker.md` et deux instances lié vers les 4 cibles donne exactement les mêmes fichiers que le même projet écrit à la main avec les deux agents rendus (test d'égalité des plans).
3. Chaque code du §5 a son test à message exact ; l'échappement `\{{` et une valeur contenant `:` et un saut de ligne ont leur test.
4. `write_project` d'un projet chargé avec instances le réécrit à l'identique (aller-retour).
5. `armadai new agent guardian --from checker --var subject=x` écrit l'instance attendue et refuse un modèle inconnu ou une variable manquante sans rien laisser.

## 8. Risques

- Les utilisateurs liront l'agent généré (`.claude/agents/guardian.md`) et le modifieront à la main : comme aujourd'hui, `link` le refusera au prochain passage (fichier modifié) ; le message doit nommer la source, ici le modèle et l'instance.
- Les rapports de conformité parlent de l'agent, pas du modèle : un `approximated` dû au modèle se lit sur chaque instance. Acceptable.
