# Table de modèles embarquée (#455)

Date : 2026-11-09 · Statut : brouillon repris le 2026-11-10, décisions Q1 à Q5 de Dimitri intégrées le 2026-11-10, aucune question ouverte côté produit · Étape : spec du lot #455, v1 (périmètre fixé le 2026-10-27)

## 1. Intention

L'étape 0 a retiré le réseau d'`armadai-providers` : plus rien n'écrit `models-cache.json` (`crates/armadai-core/src/config.rs:32-35`), et la table de repli de `model_resolution` n'a pas bougé depuis (`step0-retrait-surfaces-design.md`, sections 4.4 et 8, « à traiter avant l'étape 2 », ce qui n'a pas été fait). L'issue #455 demande de rafraîchir son contenu, de décider d'un mécanisme de rafraîchissement (hors ligne par défaut, `link` jamais réseau), de couvrir `RegistryKind::Models` et la clé `models:` de `registries.yaml`, et de tester que `link` résout chaque palier pour chaque cible.

Lecture du code du 2026-11-10 (worktree sur `89905a7`) : **la prémisse de l'issue est en grande partie périmée**. `link` ne « résout » plus de `latest:*` : un agent canonique porte un `tier` et, par cible, un `native.<cible>.model` repris tel quel. Sur les quatre cibles, une seule passe par la table embarquée, Codex, qui est hors v1 (4.1). Le lot doit donc d'abord dire ce qu'il protège. Décisions de Dimitri (2026-11-10) : rafraîchir la table de Codex ; mécanisme **double** (donnée versionnée avec `as_of`, rafraîchie à la release par un script hors binaire, **et** une commande explicite `armadai models sync` qui va sur le réseau) ; retirer les clés `models:` et `agents:` de `registries.yaml` ; garder `models-cache.json` et le chemin cache de `model_resolution`. Vérifié au dépôt par `lead` (grep) : dans le binaire, les seuls appelants de `fallback_model_for_tier` et `resolve_model_for_tier` sont dans `compile/legacy_targets.rs` (lignes 39 et 302) ; `resolve_tier_placeholder` et `load_cached_models` n'ont aucun appelant hors de `model_resolution.rs`. Le chemin cache n'est donc lu par aucune commande de `link` aujourd'hui.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs (sous réserve des réponses ; la forme suit les recommandations) :

1. Le contenu daté de la table est rafraîchi à partir de la documentation officielle de chaque fournisseur, lue au plan (aucun identifiant n'est donné ici comme vrai).
2. Un seul endroit porte les identifiants (une donnée versionnée avec sa date, pas un `match` recopié) et la table de `docs/wiki/models.md` en est tirée ou comparée par un test.
3. Chaque palier (`fast`, `pro`, `max`) est testé pour chaque cible que `link` compile : l'identifiant écrit est celui attendu, ou l'absence de modèle est rapportée (`approximated` / `not-compiled`), jamais un identifiant vide ou inventé.
4. Les clés `models:` et `agents:` de `registries.yaml` sont retirées de `RegistriesConfig` (un fichier qui les contient se lit encore sans erreur) ; `models-cache.json` et le chemin cache de `model_resolution` sont gardés.
5. `armadai models sync` : commande explicite, seule à accéder au réseau pour les modèles (4.3).
6. `docs/wiki/models.md` décrit l'état final, date de la table et `models sync` compris.

Non-objectifs :
- Aucun accès réseau dans `link`, `unlink`, `validate`, `inspect`, `graph`, `flow`, `models check|update|list` ; `models sync` est la seule exception.
- Pas de nouvelle cible, pas de support Codex/Copilot au-delà de leur sortie actuelle (hors v1).
- Pas de refonte de la classification (`classify_model_tier`, `compare_candidates`, issues #404/#412) ; touchée seulement si les nouveaux identifiants la font échouer.
- `frozen/armadai-audit/` n'est pas étendu.

Anti-objectifs :
- Que la sortie de `link` dépende de la machine (cache local lu par `link`) : `legacy_targets.rs:24-25` l'interdit déjà.
- Qu'un identifiant non vérifié contre la documentation officielle entre dans la table.
- Qu'`armadai-model` gagne une dépendance ou une notion de fournisseur : il reste pur.

## 3. Contraintes

- `link` hors réseau (décision du 2026-10-09). Seules les commandes déjà au réseau (`init --pack`, `skills sync/add`, `pack sync/add/update`, `update`) y ont droit, plus `models sync` (décision du 2026-11-10).
- **Client HTTP de `models sync`** : `curl` lancé en sous-processus, comme le reste du binaire (`cli/update.rs:26` et `:48`, `starters_registry/mod.rs:371` ; `reqwest` n'est dans aucun `Cargo.toml`, vérifié par `lead`). Le programme est injectable, comme dans `update`, ce qui permet le faux registre du critère 10. Aucun paquet nouveau au lock.
- `armadai-model` pur ; `armadai-core` et `armadai-model` compilent seuls.
- Codex et Copilot hors v1 (ancien linker, `compile/legacy_targets.rs`).
- Le crate gelé vendore `MODELS_CACHE_FILE` (`frozen/armadai-audit/src/vendored/model_registry.rs:12`) et lit `ModelTier` et `parse_latest_placeholder` (`.../vendored/linker.rs:8`) : ils restent disponibles, vérifié par `cd frozen/armadai-audit && cargo check --all-targets && cargo test`.
- `armadai-rules` : mutation par test, aucun réseau réel en test, sorties machine versionnées, texte des nouvelles lignes de sortie validé au plan, paquet nouveau au lock par Dimitri.

## 4. Conception

### 4.1 État mesuré (code lu le 2026-11-10)

Modèle écrit par `link`, par cible :

| Cible | Où le modèle vient | Passe par la table ? |
|---|---|---|
| `claude` | `native.claude.model`, sinon alias du palier `fast`→`haiku`, `pro`→`sonnet`, `max`→`opus` (`compile/claude.rs:257-263`, `:366-369`). Coordinateur : aucun modèle, ligne `agent <id> tier` `not-compiled` (`:330-332`, `:345-351`) | Non |
| `opencode` | `native.opencode.model`, sinon `model_tiers.opencode.<palier>` de `~/.config/armadai/config.yaml` (`compile/opencode.rs:421-456`, `config.rs:114-125`). Sans entrée : ligne `agent <id> model` `approximated` « inherited from the OpenCode session » (`:438-444`) ; avec : `compiled` « from model_tiers in your config » (`:454`) ; un id sans `/` est ignoré, `approximated` (`:449-453`) | Non |
| `codex` | `native.codex.model`, sinon `fallback_model_for_tier("openai", palier)`, palier `pro` par défaut (`compile/legacy_targets.rs:26-43`, appel `:38-40`) | **Oui, seul consommateur** |
| `copilot` | `model_for` rend `None` (`legacy_targets.rs:41`) ; un agent à `tier` reçoit la ligne `agent <id> tier` `not-compiled` « Copilot agent files carry no model » (`:154-161`) | Non |

Conséquences :
- Une valeur `latest:*` dans `native.<cible>.model` est reprise **telle quelle** par les trois adaptateurs (`legacy_targets.rs:33-35`, `claude.rs:366-367`, `opencode.rs:423-435`) ; seul OpenCode la refuse, faute de `/`. `link` ne la résout donc nulle part. Si le validateur la rejette ou non : à vérifier au plan.
- `fallback_model_for_tier` (`armadai-core/src/model_resolution.rs:232-246`) est appelée par `link` pour Codex uniquement. `resolve_model_for_tier` (`:438-466`, chemin « cache models.dev puis repli ») et `resolve_tier_placeholder` (`:219-222`) : les appelants dans le binaire **n'ont pas été énumérés** (l'analyste n'a pas d'outil de recherche) ; à établir au plan par grep. S'il n'y en a pas hors tests, le chemin cache (`load_cached_models` `:277-316`, `compare_candidates` `:369-401`, `generation_key` `:113-128`) est du code que plus aucune commande n'appelle.

Ce qui est daté, à rafraîchir :
- `fallback_model_for_tier` (`model_resolution.rs:234-244`) : trois fournisseurs (`anthropic`, `google`, `openai`) × trois paliers, `google` Max replié sur Pro par `effective_tier` (`:143-148`) ; le motif `(_, palier)` rend un modèle Anthropic pour tout autre fournisseur (`:242-244`).
- `embedded_aliases` (`armadai-core/src/model_aliases.rs:4-19`, lu par `resolve_alias` `:31-56`) : six alias, dont `gemini-3.0-pro` → `latest:pro` (`:11`), que `models update` **rapporte sans réécrire** (`cli/models.rs:48-51`, `:245-252`). Un fichier local `~/.config/armadai/model-aliases.json` les surcharge (`model_aliases.rs:22-26`).
- `docs/wiki/models.md:30-34` : la table de Codex y est recopiée à la main (pas d'autre fournisseur).
- Tests qui figent les valeurs : `test_fallback_models_per_tier` (`model_resolution.rs:474-501`), `resolve_tier_placeholder_maps_each_tier…` (`:530-552`, par la table elle-même), `model_aliases.rs:62-90`.
- Le catalogue `armadai-core/src/model_resolution/catalog-2026-08-28.json` est une **fixture de test** (`fetched_at` 1787000000) ; ses ids ne sont pas une source de vérité sur les fournisseurs et ne sont pas repris ici.

Ne sont pas affirmés ici : que les identifiants actuels soient obsolètes ou valides chez les fournisseurs, ni quels sont les identifiants courants. Cela se vérifie au plan, dans la documentation officielle.

`registries.yaml` et `models:` : `RegistriesConfig` (`armadai-core/src/registries.rs:56-67`) analyse `agents`, `skills`, `models`, `starters` ; `RegistryKind` n'a **qu'une variante, `Skills`** (`:70-73`) et `sources()` ne sert que celle-là (`:75-81`). Il n'existe pas de `RegistryKind::Models` (l'issue le suppose) : la clé `models:` est analysée et lue par personne (même constat pour `agents:`). Les tests la parsent seulement (`:266-300`).

`models-cache.json` : `MODELS_CACHE_FILE` est lu par `load_cached_models` (`model_resolution.rs:295`) et par le crate gelé ; aucun code actif ne l'écrit (`config.rs:33-34`).

`armadai models` (`crates/armadai/src/cli/models.rs`) : `check` et `update` comparent les `native.<cible>.model` du projet aux alias de `model_aliases` (`:54-79`) et réécrivent le frontmatter (`:178-237`). **`models list` ne montre pas la table** : il liste les projets enregistrés par `link` (`:356-381`). Aucun réseau, aucune table de paliers.

### 4.2 Forme proposée (à confirmer par les réponses)

Option recommandée, « donnée versionnée, rafraîchie à la release, hors binaire » :

- `armadai-core` : les identifiants de repli et les alias passent dans une seule donnée embarquée (JSON ou YAML sous `crates/armadai-core/`, avec `version` et `as_of: AAAA-MM-JJ`), lue par `fallback_model_for_tier` et `embedded_aliases` à la place des `match` et `HashMap` écrits à la main. Le code remplacé est supprimé, pas dupliqué.
- Un test de complétude : chaque paire (fournisseur, palier) que `link` résout pour une cible compilée a une entrée, et chaque entrée passe `classify_model_tier` vers le bon palier.
- Tests de `link` par palier et par cible : Claude (alias), OpenCode (avec et sans `model_tiers`), Codex (table), Copilot (`not-compiled`).
- Rafraîchissement à la release : une étape outillée mais hors binaire (script ou tâche `mise`) qui réécrit la donnée et son `as_of`.
- `registries.yaml` : `models` et `agents` sortent de `RegistriesConfig` (`registries.rs:56-67`) et de ses tests (`:266-318`) ; `#[serde(default)]` sans `deny_unknown_fields` doit garder la lecture tolérante (à vérifier par test). Le commentaire d'en-tête du module (`:1-10`) est corrigé.
- `models-cache.json` et le chemin cache : gardés, inchangés.
- Message visible : la date `as_of` de la table est affichée là où l'utilisateur la lit. `models list` ne s'y prête pas (liste de projets, 4.1) : emplacement et texte de la ligne à proposer au plan, validés par Dimitri. Question de forme laissée au plan : `docs/wiki/models.md` seulement, ou aussi une ligne d'une commande.

### 4.3 `armadai models sync`

- **Comportement** : interroge la source, valide la réponse, écrit le fichier, affiche le nombre de fournisseurs et de modèles écrits et la date. Sans sous-option, rien d'autre.
- **Où il écrit** : `<config_dir>/models-cache.json` (`config.rs:35`), le fichier que lit déjà `load_cached_models`, au format que ce lecteur et le crate gelé acceptent (`providers: {fournisseur: [{id, cost?}]}`, `fetched_at`). Il n'écrit ni la table embarquée, ni rien dans le projet, ni `.armadai/`.
- **`link` n'y touche jamais** : `link` pour Codex lit `fallback_model_for_tier`, pas le cache (`legacy_targets.rs:24-25`, `:38-40`) ; l'exécution de `sync` ne change donc aucune sortie de `link`. Conséquence assumée par Dimitri : le cache écrit par `sync` n'est lu que par `resolve_model_for_tier` (sans appelant de `link`) et par le crate gelé.
- **Hors ligne ou échec réseau** : code de sortie non nul, message d'une ligne qui nomme la source et la cause, **fichier existant intact** (écriture dans un temporaire puis renommage, réponse validée avant écriture : JSON lisible, au moins un fournisseur). Pas de nouvelle tentative silencieuse, pas de repli sur un autre hôte.
- **Source** : le catalogue models.dev, la source de l'ancien cache (décision du 2026-11-10). L'URL exacte se lit dans l'historique du code retiré (tag `pre-v1-declaratif`, `armadai-providers`) et se vérifie au plan.
- **Transport** : `curl` en sous-processus à programme injectable (voir 3). Code de sortie et message d'échec de `curl` rapportés dans la ligne d'erreur.

## 5. Critères d'acceptation

(Pour la forme recommandée ; « selon Q » : change avec la réponse.)

1. `armadai link --target codex` sur un projet témoin à trois agents (`tier: fast`, `pro`, `max`) écrit dans les trois fichiers les identifiants de la table courante, ceux de `docs/wiki/models.md`.
2. `armadai link --target claude` écrit `model: haiku`, `sonnet`, `opus` pour ces trois agents, et aucun `model` pour un agent sans palier ni `native`.
3. `armadai link --target opencode` : sans `model_tiers`, la ligne `agent <id> model` est `approximated` ; avec `model_tiers.opencode.<palier>`, l'identifiant configuré est écrit et la ligne est `compiled`.
4. `armadai link --target copilot` n'écrit aucun modèle et le rapport dit `not-compiled` pour chaque agent à `tier`.
5. Aucun identifiant de la table ne contient `latest` ni un nom que `classify_model_tier` rend `None` pour son fournisseur ; un test énumère toutes les entrées.
6. Chaque identifiant de la table a une source : le plan liste, par fournisseur, la page de documentation officielle et la date de lecture, et le reviewer la relit.
7. `link` lancé avec un proxy invalide et sans réseau produit la même sortie qu'avec réseau (aucune connexion ouverte), y compris après un `models sync` réussi qui a écrit un `models-cache.json` différent : `armadai link --target codex` écrit les mêmes fichiers avant et après.
8. `docs/wiki/models.md` affiche la même table que le code et la même date ; un test compare les deux.
9. Un `registries.yaml` contenant `models:` et `agents:` se lit sans erreur ni avertissement, et ces clés n'existent plus dans `RegistriesConfig`.
10. `armadai models sync` contre un faux registre local (programme `curl` injecté qui sert un fichier local, aucun réseau réel) : code 0, `models-cache.json` écrit avec les fournisseurs et modèles du faux registre, et relu par `resolve_model_for_tier`.
11. `armadai models sync` contre une source injoignable ou qui répond un JSON invalide : code non nul, message d'une ligne nommant la source, `models-cache.json` préexistant octet pour octet identique, aucun fichier temporaire laissé.
12. La barrière complète passe (fmt, clippy, test, `cargo build -p armadai-core`, `cargo build -p armadai-model`) et `cd frozen/armadai-audit && cargo check --all-targets && cargo test` aussi.

Essai réel avec Dimitri : `link` sur un projet réel pour Codex et OpenCode, lecture des identifiants écrits, avant et après.

## 6. Risques

- Prémisse de l'issue en grande partie périmée : sans réponse à Q1, le lot peut rafraîchir une table que seule une cible hors v1 lit. Se voit à la liste des consommateurs établie au plan.
- Identifiants écrits de mémoire : un identifiant faux compile et ne se voit qu'à l'usage. Parade : source officielle citée par entrée (critère 6).
- Changer la table change la sortie de `link` pour Codex et casse les golden et tests qui figent les anciens identifiants : mise à jour mécanique, relue ligne à ligne contre la spec.
- Un identifiant plus récent classé `preview`, ou à date dans son nom, change la résolution depuis un cache utilisateur : couvert par les tests #404/#412, à relancer.
- Le crate gelé vendore `MODELS_CACHE_FILE` : retirer la constante ou le lecteur du cœur peut le casser en silence, d'où le critère 10.
- Réintroduire un chemin réseau « par commodité » dans `link` : anti-objectif, vérifié au critère 7.

## Questions ouvertes

Tranchées le 2026-11-10 : Q1 rafraîchir la table, Q2 les deux mécanismes, Q3 retirer les clés, Q4 garder le cache, Q5 models.dev vers le cache.

- À vérifier au plan, sans Dimitri : si le validateur accepte `latest:*` dans `native.<cible>.model` ; identifiants courants de chaque fournisseur dans leur documentation officielle ; URL de models.dev. Le module `model_updater` n'existe pas dans `armadai-core/src/lib.rs:1-18` : la réécriture des ids est dans `cli/models.rs`.
