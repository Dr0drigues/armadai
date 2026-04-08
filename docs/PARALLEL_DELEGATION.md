# Parallel Delegation Implementation Plan

## Context

Task C1 from the orchestration evolutions plan requires implementing parallel dispatch for the `HierarchicalEngine`.

Currently, when a coordinator delegates to multiple agents, they execute sequentially in a loop. This document outlines the challenges and proposes a solution for parallel execution.

## Current Architecture Limitations

The current implementation has several constraints that prevent straightforward parallelization:

1. **Mutable Self**: `invoke_agent(&mut self, ...)` takes exclusive mutable access, preventing concurrent calls
2. **Recursive Calls**: `invoke_agent` calls itself recursively for nested delegations, using `Pin<Box<dyn Future>>`
3. **Shared State**: Multiple fields are mutated during execution:
   - `conversations`: Per-agent conversation history
   - `trace`: Delegation events log
   - `iteration_count`, `total_tokens_in/out`, `total_cost`, `invocation_count`: Aggregated metrics
4. **Budget Checks**: Token/cost budgets must be checked before each delegation to avoid races

## Proposed Solution

### Phase 1: State Extraction (Required for Parallelization)

Extract mutable state into a separate structure that can be shared:

```rust
struct EngineState {
    conversations: HashMap<String, Vec<ChatMessage>>,
    trace: Vec<DelegationEvent>,
    iteration_count: u32,
    total_tokens_in: u32,
    total_tokens_out: u32,
    total_cost: f64,
    invocation_count: u32,
}

pub struct HierarchicalEngine {
    config: OrchestrationConfig,
    agents: HashMap<String, Agent>,
    providers: HashMap<String, Arc<dyn Provider>>,
    agents_info: HashMap<String, AgentInfo>,
    state: Arc<Mutex<EngineState>>,  // Shared mutable state
}
```

### Phase 2: Parallel Dispatch Implementation

Modify `invoke_agent` to:

1. Collect all `Delegate` actions from a response
2. Check budget limits BEFORE spawning parallel tasks
3. Spawn concurrent tasks using `tokio::spawn` or `futures::join_all`
4. Each task:
   - Acquires the state mutex only when needed (not for the whole duration)
   - Calls the LLM provider (outside the mutex)
   - Updates metrics atomically
5. Aggregate results after all tasks complete
6. Continue with synthesis step

```rust
// Pseudo-code
if !delegate_actions.is_empty() {
    // Check budget before parallel batch
    {
        let state = self.state.lock().unwrap();
        if state.total_tokens_in + state.total_tokens_out >= budget {
            return partial_result();
        }
    }
    
    // Spawn parallel tasks
    let mut handles = vec![];
    for delegate in delegate_actions {
        let state = Arc::clone(&self.state);
        let provider = Arc::clone(&self.providers[&delegate.target]);
        // ... clone other needed data
        
        let handle = tokio::spawn(async move {
            // Call invoke_agent recursively (supports nested delegations)
            // This requires invoke_agent to take Arc<Mutex<State>> instead of &mut self
        });
        handles.push(handle);
    }
    
    // Wait for all to complete
    let results = join_all(handles).await;
}
```

### Phase 3: Metrics Aggregation

After parallel execution:
- Collect metrics from all completed tasks
- Update the shared state atomically
- Ensure `trace` events are in a sensible order (maybe use timestamps)

## Challenges

1. **Recursive Calls with Shared State**: `invoke_agent` must work with `Arc<Mutex<State>>` instead of `&mut self`
   - Requires significant refactoring of the method signature
   - All callers must be updated
   
2. **Nested Delegations**: When an agent itself delegates (recursively), the nested calls must also support parallelization
   - Solution: Make `invoke_agent` a standalone function that takes `Arc<Mutex<State>>`

3. **Budget Race Conditions**: If multiple parallel tasks check the budget simultaneously, they might all pass the check before any updates the state
   - Solution: Use a "reserved budget" approach where we pre-decrement an estimated amount

4. **Trace Ordering**: With parallel execution, trace events might be out of chronological order
   - Solution: Add timestamps to `DelegationEvent` and sort after collection

5. **Error Handling**: If one parallel task fails, should we cancel others or wait for all?
   - Current approach: Wait for all, return first error encountered
   - Alternative: Use `tokio::select!` to cancel on first error

## Testing Strategy

1. **Sequential Compatibility**: All existing tests must pass
2. **Parallel Execution**: New test with `DelayedMockProvider` to verify concurrent execution
3. **Metrics Correctness**: Verify that parallel execution produces same metrics as sequential
4. **Budget Enforcement**: Verify budget checks work correctly with parallelization
5. **Nested Delegations**: Test that agents can delegate while being part of a parallel batch

## Alternative: Message-Passing Architecture

A more radical refactor would use actor model / message passing:

- Each agent runs in its own tokio task
- Agents communicate via `tokio::sync::mpsc` channels
- The coordinator dispatches messages to agent tasks
- No shared mutable state (each task owns its data)
- Natural parallelism without locks

This would be Phase 4 / Future Work.

## Implementation Status

**Current**: Sequential execution only
**Next**: Phase 1 (state extraction) + Phase 2 (parallel dispatch for leaf nodes only)
**Future**: Phase 3 (full recursive parallelization) + Phase 4 (message-passing architecture)

## Decision Log

**2026-04-01**: After initial implementation attempt, identified that full parallelization with nested delegation support requires architectural refactoring. Documenting the path forward in this design doc.
