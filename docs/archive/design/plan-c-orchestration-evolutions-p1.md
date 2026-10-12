# Plan C — Évolutions orchestration P1 (C1-C3)

> **Statut** : À faire
> **Effort** : 🔴 Élevé (8-12 jours)
> **Version cible** : 0.12.0+

---

## C1 — Parallel dispatch (`tokio::join!`)

### Contexte

Actuellement, les délégations dans `HierarchicalEngine::invoke_agent()` sont traitées **séquentiellement** :

```rust
// hierarchical.rs lignes 199-225
let mut results = Vec::new();
for action in &actions {
    match action {
        DelegationAction::Delegate { target, task } => {
            let result = self.invoke_agent(target, task, depth + 1, agent_name).await?;
            results.push((target.clone(), result));
        }
        // ...
    }
}
```

Quand un coordinateur délègue à 3 agents indépendants, ils s'exécutent l'un après l'autre au lieu de tourner en parallèle.

### Implémentation

#### Phase 1 — Parallélisation des Delegate indépendants

1. Collecter toutes les `DelegationAction::Delegate` d'une même réponse
2. Les exécuter en parallèle via `tokio::task::JoinSet` ou `futures::future::join_all`
3. Collecter les résultats et les ré-injecter dans le contexte du coordinateur
4. Les `AskPeer` et `Escalate` restent séquentiels (dépendances implicites)

```rust
use tokio::task::JoinSet;

let mut join_set = JoinSet::new();
for action in delegates {
    let engine = self.clone(); // ou Arc<Mutex<Self>>
    join_set.spawn(async move {
        engine.invoke_agent(&target, &task, depth + 1, agent_name).await
    });
}

let mut results = Vec::new();
while let Some(result) = join_set.join_next().await {
    results.push(result??);
}
```

#### Phase 2 — Gestion de la concurrence

- `HierarchicalEngine` doit devenir `Arc<Mutex<...>>` ou utiliser des compteurs atomiques pour les métriques
- Les conversations par agent (`HashMap<String, Vec<ChatMessage>>`) doivent être protégées par des locks
- Le compteur `iteration_count` doit être atomique (`AtomicU32`)

#### Challenges

- La méthode `invoke_agent` est récursive avec `Pin<Box<dyn Future>>` — la parallélisation nécessite `Send + Sync`
- Les providers sont déjà `Send + Sync` (trait bound)
- Les métriques agrégées doivent être thread-safe

### Fichiers impactés

- `src/core/orchestration/hierarchical.rs` — Refactor invoke_agent + état partagé
- `src/core/orchestration/e2e_tests.rs` — Tests avec délégations parallèles

### Tests

- Scénario : 3 délégations indépendantes → vérifier que le temps total ≈ max(durée) et non sum(durées)
- Scénario : délégation + peer question → le peer attend la délégation
- Non-régression : tous les tests séquentiels existants passent toujours

---

## C2 — TUI visualization (arbre de délégation)

### Contexte

Le storage a déjà les tables nécessaires :
- `orchestration_runs` : pattern, config_json, outcome_json, rounds, halt_reason
- `board_entries` : entrées blackboard par round
- `ring_contributions` : contributions ring par lap

Des queries existent dans `storage/queries.rs` (lignes 298-406) marquées "reserved for future UI".

### Implémentation

#### Nouveau tab `Orchestration` dans le TUI

1. **Ajouter** `Orchestration` et `OrchestrationDetail` au `Tab` enum dans `app.rs`
2. **Créer** `src/tui/views/orchestration.rs`
3. **Touche** `8` pour accéder au tab (après Models = `7`)
4. **État** : `orchestration_runs: Vec<OrchestrationRunRecord>` dans `App`

#### Vue liste (tab Orchestration)

| Colonne | Source |
|---------|--------|
| Date | `orchestration_runs.created_at` |
| Pattern | `orchestration_runs.pattern` |
| Coordinator | extrait de `config_json` |
| Rounds | `orchestration_runs.rounds` |
| Halt reason | `orchestration_runs.halt_reason` |
| Coût | extrait de `outcome_json` |

#### Vue détail (OrchestrationDetail)

Selon le pattern :

**Hierarchical** :
- Arbre de délégation (tree view) : coordinator → leads → agents
- Chaque noeud affiche : agent, message envoyé, réponse résumée, tokens, coût
- Navigation : `j/k` pour parcourir l'arbre, `Enter` pour expand/collapse

**Blackboard** :
- Timeline des rounds avec les entrées par agent
- État du blackboard à chaque round

**Ring** :
- Visualisation des laps avec le token passé entre agents
- Score de consensus par lap

#### Widgets réutilisables

- `TreeWidget` : arbre collapsible pour la hiérarchie de délégation
- Réutiliser les widgets existants de `src/tui/widgets/` (table, detail)

### Fichiers impactés

- `src/tui/app.rs` — Nouveau tab, état, chargement données
- `src/tui/views/mod.rs` — Nouveau module orchestration
- `src/tui/views/orchestration.rs` — Vues liste et détail (nouveau)
- `src/tui/widgets/` — TreeWidget potentiel (nouveau)
- `src/storage/queries.rs` — Activer les queries réservées

### Tests

- Test rendu de la liste orchestration (mock data)
- Test rendu de l'arbre hiérarchique
- Test navigation dans l'arbre

---

## C3 — Cost budgets orchestration

### Contexte

Le champ `token_budget: Option<u64>` existe déjà dans `OrchestrationConfig` (défaut : 100 000), mais il n'est **jamais vérifié** pendant l'exécution. Les métriques sont agrégées dans `HierarchicalEngine` (`total_tokens_in`, `total_tokens_out`, `total_cost`) mais sans enforcement.

### Implémentation

#### Phase 1 — Enforcement token budget

1. **Dans `invoke_agent()`** (hierarchical.rs), avant chaque appel LLM :
   ```rust
   if let Some(budget) = self.config.token_budget {
       let used = self.total_tokens_in + self.total_tokens_out;
       if used as u64 >= budget {
           return Err(anyhow!("Token budget exceeded: {used}/{budget}"));
       }
   }
   ```
2. **Halt graceful** : au lieu de `Err`, remonter un résultat partiel avec `halt_reason: "budget_exceeded"`
3. **Même logique** pour Blackboard et Ring (dans leurs engines respectifs)

#### Phase 2 — Cost limit orchestration

Ajouter un nouveau champ `cost_limit: Option<f64>` à `OrchestrationConfig` :
```yaml
orchestration:
  enabled: true
  token_budget: 100000
  cost_limit: 5.0  # $ max pour l'orchestration complète
```

Vérification similaire sur `self.total_cost` avant chaque appel.

#### Phase 3 — Feedback et observabilité

- Afficher le budget restant dans les logs (`RUST_LOG=info`)
- Stocker `halt_reason: "budget_exceeded"` dans `orchestration_runs`
- Afficher dans le TUI (tab Orchestration C2) : barre de progression budget consommé / total

### Fichiers impactés

- `src/core/orchestration/mod.rs` — Ajout `cost_limit` à `OrchestrationConfig`
- `src/core/orchestration/hierarchical.rs` — Checks budget dans `invoke_agent` et `call_llm`
- `src/core/orchestration/blackboard.rs` — Même checks
- `src/core/orchestration/ring.rs` — Même checks
- `src/storage/schema.rs` — Stocker halt_reason si pas déjà fait

### Tests

- Test : orchestration avec budget de 100 tokens → halt après 1-2 appels
- Test : orchestration avec cost_limit de 0.01 → halt rapide
- Test : orchestration sans budget → pas de limite (non-régression)
- Test : halt graceful retourne un résultat partiel exploitable

---

## Ordre d'implémentation recommandé

```
C3 (cost budgets) → C1 (parallel dispatch) → C2 (TUI visualization)
```

**Justification** :
1. C3 est le plus simple et apporte de la sécurité immédiate
2. C1 est un refactor structurel qui doit être fait avant d'ajouter de l'UI
3. C2 bénéficie des données de C3 (budgets) et de la stabilité de C1

## Délégation

| Item | Agent |
|------|-------|
| C1 Parallel dispatch | @core-specialist |
| C2 TUI visualization | @ui-specialist |
| C3 Cost budgets | @core-specialist + @qa-specialist |

## Critères de complétion

### C1
- [ ] Délégations multiples exécutées en parallèle
- [ ] Métriques thread-safe
- [ ] Tests de performance (parallèle vs séquentiel)
- [ ] Non-régression complète

### C2
- [ ] Tab Orchestration dans le TUI (touche `8`)
- [ ] Vue liste des orchestration runs
- [ ] Vue détail avec arbre de délégation (Hierarchical)
- [ ] Vue détail Blackboard et Ring
- [ ] Navigation fonctionnelle

### C3
- [ ] Token budget enforced dans les 3 engines
- [ ] `cost_limit` ajouté et enforced
- [ ] Halt graceful avec résultat partiel
- [ ] Halt reason stocké en base
- [ ] Tests unitaires
