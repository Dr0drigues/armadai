# Agents déclaratifs — le YAML comme source d'`Agent` — Design

**Date** : 2026-08-20
**Statut** : validé (design), à implémenter
**Cible** : lot 1 d'un moteur déclaratif. Les agents ; les fragments de prompt viennent au lot 2.

## Contexte : ce qui existe déjà, et ce qui manque

L'idée « config comme source de vérité » est capturée depuis le 2026-07-17 (mémoire
`project_vision_declarative`, idée B). Ce spec la reprend avec deux éléments qu'elle n'avait pas.

**Les briques sont là, en pièces détachées** :

| Brique | Où | Ce qu'elle fait déjà |
|---|---|---|
| Substitution de variables | `crates/armadai/templates/*.md` + `cli/new.rs` | `{{name}}`, `{{description}}`, `{{stack}}`, `{{model}}` |
| Fragments composables | `armadai-core/src/prompt.rs` | frontmatter YAML + prose, sélection par `apply_to` |
| Projection native | `linker/` | un `Agent` → 5 formats (claude, codex, copilot, gemini, opencode) |
| Rendu d'agent | `audit/proposal.rs` | `Agent` → `.md` au format ArmadAI |

**Le linker projette déjà.** `armadai link --target claude` *génère* `.claude/agents/{slug}.md`
(`linker/claude.rs:43`) — les fichiers d'un projet lié ne sont pas une copie maintenue à la main,
c'est une projection régénérable. Le mécanisme « source unique → projections » de l'idée B **tourne
donc déjà**. Ce qui manque n'est pas la génération : c'est que la source soit 77 fichiers Markdown.

**Ce chantier n'est donc pas un nouveau sous-système**, mais un format d'entrée qui assemble
l'existant.

### Mesures sur la bibliothèque (2026-08-20)

77 fichiers dans `~/.config/armadai/agents/`, dont 76 déclarent `provider` et `model`, et 72
`temperature` et `max_tokens`. Sur ceux qui déclarent la clé, une valeur domine largement :

| Clé | Valeur dominante | Occurrences | Sur combien de déclarations |
|---|---|---|---|
| `model` | `latest:pro` | 60 | 76 |
| `max_tokens` | `8192` | 57 | 72 |
| `temperature` | `0.3` | 49 | 72 |
| `provider` | `claude` | 47 | 76 (3 valeurs distinctes) |

## Ce que ce chantier ne fait PAS gagner

Établi par mesure, pour qu'on ne conçoive pas sur une prémisse fausse.

**Aucune économie de tokens.** Les métadonnées ne partent jamais au modèle : ArmadAI les parse pour
choisir provider/modèle/température. Les 5 152 tokens de blocs `## Metadata` de la bibliothèque
pèsent sur le disque et sur les yeux, pas sur la facture. Ce qui coûte, ce sont les ~52 460 tokens
de prose d'instruction, et un YAML n'y change rien puisque cette prose doit exister quelque part.

**Aucune réduction de la saturation de contexte** pour la même raison. Et la composition peut
l'aggraver : ~680 tokens de prompt par agent en moyenne, avec des fragments de 66 lignes — un agent
qui compose quatre fragments a un prompt plus long qu'un prompt écrit sur mesure. On échange de la
duplication sur disque contre de l'inclusion au runtime.

**Aucune garantie d'amélioration de la qualité des agents.** Un prompt assemblé de fragments
génériques est moins spécifique qu'un prompt taillé, et un agent aux instructions vagues comble les
vides. Le gain en cohérence peut se payer en précision. Seul un comparatif sur tâche réelle le
dira — mesurable, comme la prose l'a été pour le policy gate.

## Le gain réel : une seule vérité

Le seul mécanisme par lequel ce chantier réduit les hallucinations est la **suppression des vérités
en double**, et il est démontré sur ce projet même : `.claude/CLAUDE.md` contredisait le `CLAUDE.md`
racine, et le modèle a suivi la version actionnable tout en citant l'autre mot pour mot
(mémoire `project_orchestration_policy_gate`). Deux specs de ce dépôt ont affirmé des faits erronés
(« vaut pour l'équipe », « chaque sous-agent ») nés de la même information vivant à deux endroits.

C'est l'argument décisif de l'approche retenue.

## Décisions de cadrage (Dimitri, 2026-08-20)

1. **Le YAML porte les métadonnées et compose le prompt depuis des fragments**, avec paramètres.
   La prose reste écrite à la main, une fois, dans des fragments réutilisés.
2. **Prompt « généré » = substitution dans des fragments humains**, jamais rédaction depuis des
   énumérés. Un `role: reviewer, strictness: high` produirait des instructions fades, et un prompt
   fade fait un agent médiocre.
3. **Extension du moteur aux fragments de prompt** : souhaitée, reportée au lot 2. Concevoir les
   deux à la fois, c'est concevoir mal les deux.
4. **Aucun artefact intermédiaire.** Le YAML produit un `Agent` en mémoire, pas un `.md` sur disque.

## Approches écartées

**Le YAML génère des `.md` dans la bibliothèque.** Plus simple (réutilise `render_agent` tel quel),
et le résultat est inspectable. Écartée : elle crée un fichier éditable de plus, donc une seconde
source, donc la divergence — exactement le problème que ce chantier traite. La première fois qu'on
générera un `.md` « pour inspection », on l'aura réintroduit.

**Le YAML remplace le format `.md`.** Cohérent à terme, mais ArmadAI se présente comme un
orchestrateur d'agents Markdown : le standard passerait d'interface à format d'export. Décision
trop lourde pour un lot 1, et rien n'oblige à la prendre maintenant.

## Architecture

La chaîne actuelle est `AgentRef → resolve_agent() → PathBuf → parse_agent_file() → Agent`. Le
goulot est que `resolve_agent` (`armadai-core/src/project.rs:281`) rend un **chemin**, ce qui
suppose un fichier.

```
.armadai/agents.yaml
        │
        ▼  AgentRef::Declared
armadai_core::agent_source::load_agent(&AgentRef, root) -> Agent
        │   ├─ Named / Registry / Path → resolve_agent() + parse_agent_file()   (inchangé)
        │   └─ Declared                → defaults + composition + substitution
        ▼
     Agent (en mémoire)
        │
        ▼  inchangé
linker/  → .claude/agents/, .github/agents/, … (5 cibles)
```

`AgentRef` gagne un variant `Declared { declared: String }`. Une nouvelle fonction
`load_agent(&AgentRef, &Path) -> anyhow::Result<Agent>` rend un `Agent` là où `resolve_agent` rend
un chemin.

**Cette séparation est sémantique, pas une commodité.** `resolve_agent` sert ceux qui *manipulent
des fichiers* — `model_updater` et `pack_validation` réécrivent les modèles dépréciés en place ;
`load_agent` sert ceux qui *exécutent*. Un agent déclaré en YAML n'a pas de fichier, et il est juste
que `resolve_agent` échoue pour lui.

**Non touché** : le parseur `.md`, le linker (il consomme des `Agent`, l'origine lui est
indifférente), les 77 agents existants, `audit --propose`.

## Le format

```yaml
# .armadai/agents.yaml
defaults:
  provider: claude
  model: latest:pro
  temperature: 0.3
  max_tokens: 8192

agents:
  - name: core-specialist
    description: Core domain and orchestration engine
    scope: [src/core/**, src/parser/**]
    tags: [rust, domain]
    prompt:
      - specialist-base
      - { armadai-architecture: { module: core } }

  - name: ui-specialist
    description: TUI and Web dashboards
    temperature: 0.4          # le seul écart aux défauts
    scope: [src/tui/**, src/web/**]
    prompt: [specialist-base, ratatui-conventions]
```

**Fusion des défauts, peu profonde** : un champ absent prend la valeur de `defaults`. Les listes
(`tags`, `scope`) sont **remplacées, jamais fusionnées** — moins expressif, mais prévisible : un
agent qui déclare son `scope` a exactement celui-là, sans hériter silencieusement d'un périmètre
plus large.

## Composition et substitution

`prompt:` accepte deux formes, chaîne ou map à une clé — YAML idiomatique, sans niveau d'imbrication
superflu. Le `system_prompt` est la concaténation des corps de fragments, dans l'ordre déclaré,
séparés d'une ligne vide.

La substitution remplace `{{module}}` par sa valeur, plus les variables implicites de l'agent
(`{{name}}`, `{{description}}`). Le code existe : `cli/new.rs:58-68` fait déjà
`content.replace("{{name}}", …)`. Il est **extrait dans `armadai-core`** pour que templates et
fragments se comportent identiquement — pas réécrit.

`prompt.rs` garde son mécanisme `apply_to` pour les agents `.md`. Les deux coexistent : `apply_to`
est piloté par le fragment, le YAML est piloté par l'agent.

## Où l'on échoue — et pourquoi c'est l'inverse du policy gate

Un fragment introuvable, ou une variable `{{x}}` restée non substituée, sont des **erreurs dures**.
C'est l'exact opposé du principe du policy gate, où toute incertitude autorise. L'inversion est
délibérée :

| | Effet d'une défaillance |
|---|---|
| Policy gate | dégrader = laisser passer du travail légitime |
| Composition de prompt | dégrader = livrer un agent aux instructions incomplètes |

Un agent au prompt amputé ne signale rien : il comble les vides, c'est-à-dire qu'il hallucine. Et un
prompt contenant littéralement `{{module}}` est un texte que le modèle va tenter d'interpréter.
Mieux vaut refuser de construire l'agent.

Une variable fournie mais jamais utilisée reste un **avertissement** : c'est presque toujours une
faute de frappe, mais elle ne casse rien.

**Un nom présent à la fois dans `agents.yaml` et dans la bibliothèque `.md` est une erreur**, pas
une précédence. La tentation serait de faire gagner le local, comme partout — mais ce chantier
existe pour supprimer les vérités en double, et une précédence silencieuse en crée une : on
éditerait un `.md` sans effet, sans rien pour le signaler. La règle `C01` de l'audit détecte déjà
les collisions de noms ; le chargement, lui, doit refuser.

Un YAML malformé remonte la position que `serde_yaml_ng` fournit. Une référence `Declared` sans
entrée correspondante échoue.

## Frugalité : rien à inventer

La règle `A05` (`audit/rules/assets.rs:44`) flaggue déjà tout agent dont le prompt dépasse
`prompt_token_threshold` (défaut 4000, réglable par projet). Les agents composés devenant des
`Agent` comme les autres, le garde-fou existant dit quand une composition a trop gonflé — au lieu
d'une limite arbitraire dans le générateur.

Réserve : `A05` mesure aujourd'hui les agents importés depuis `.claude/`. Pour qu'il voie un agent
déclaré en YAML, l'audit devra le charger ; sinon la frugalité n'est mesurée que sur les
projections, ce qui reste utile mais indirect.

## Mise à jour des modèles dépréciés

`model_updater` détecte les modèles dépréciés et les corrige en place, et il est appelé
automatiquement par `run`, `link` et `init`. Un agent déclaré doit en bénéficier comme les autres,
sinon le YAML devient le seul endroit du projet où un modèle mort passe inaperçu.

**La réécriture est déjà format-agnostique.** `update_agent_file`
(`armadai-core/src/model_updater.rs:78`) opère par substitution textuelle — `content.replacen(": old",
": new", 1)` — et non par round-trip de parseur. Elle s'applique donc au YAML sans rien réécrire, et
surtout **sans effacer les commentaires**, ce qu'un aller-retour `serde_yaml_ng` ferait
inévitablement.

Deux pièges propres au YAML, à traiter explicitement :

**Occurrences multiples.** Dans un `.md` un agent déclare son `model` une fois ; dans `agents.yaml`
la clé apparaît dans `defaults` et dans chaque agent qui s'en écarte. Le `replacen(…, 1)` actuel ne
corrigerait que la première. La détection doit donc rendre un finding **par occurrence**, avec sa
position, et la réécriture les traiter toutes.

**Substitution accidentelle.** Un motif `: latest:pro` brut apparaîtrait aussi dans un commentaire ou
dans une `description`. La substitution doit être **bornée aux lignes dont la clé est `model` ou qui
sont un élément de `model_fallback`**, jamais appliquée au texte libre. C'est la différence entre
corriger une configuration et corriger de la prose.

**Tests dédiés** : un modèle déprécié dans `defaults` est corrigé ; un modèle déprécié dans deux
agents distincts est corrigé aux deux endroits ; un modèle déprécié cité dans un commentaire ou une
`description` n'est **pas** touché ; les commentaires et l'ordre des clés survivent à la réécriture.

## Tests

Du plus mécanique au plus porteur :

- **Fusion des défauts** : scalaire surchargé ; liste remplacée et non fusionnée ; agent sans aucun
  écart prenant tous les défauts.
- **Composition** : ordre déclaré respecté ; séparateur entre fragments ; fragment unique.
- **Substitution** : variable substituée ; variable manquante → erreur ; variable fournie mais
  inutilisée → avertissement, pas erreur.
- **Erreurs de chargement** : fragment introuvable ; nom en collision avec un `.md` de la
  bibliothèque ; référence `Declared` orpheline ; YAML malformé.
- **L'invariant qui garantit le reste** : un agent déclaré en YAML et son équivalent `.md`
  produisent une **projection linker identique**. C'est la preuve qu'il n'existe pas deux chemins
  divergents, et c'est la propriété la plus facile à mal vérifier.

## Limites assumées

- **La composition peut allonger les prompts** plutôt que les raccourcir (voir « ce que ce chantier
  ne fait pas gagner »). `A05` le mesure ; rien ne l'empêche.
- **Aucun convertisseur `.md` → YAML.** Migrer un agent, c'est le déclarer et supprimer son `.md`, à
  la main. Outiller une migration vers un format non encore éprouvé est le meilleur moyen de figer
  une erreur.
- **Le vrai levier contre le mauvais routage est ailleurs** : la similarité des descriptions
  d'activation, que la règle `C03` mesure déjà (Jaccard, seuil `activation_similarity`, défaut 0.6).
  Elle ne trouve rien sur les 6 agents de ce projet ; elle parlerait sur trente. Indépendant du
  format.

## Hors périmètre

- L'extension du moteur aux fragments de prompt — lot 2, souhaité par Dimitri.
- La génération de prose depuis des énumérés (`role`, `strictness`) — écartée en cadrage.
- Le remplacement du format `.md` — les 77 agents restent valides, le YAML est une entrée
  supplémentaire.
- Un convertisseur `.md` → YAML.
