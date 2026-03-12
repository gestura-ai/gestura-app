# Phase 9 — Agent-loop maturity jump

## Purpose

Phase 9 defines the next maturity step after the validated v2 orchestration baseline. The goal is not to replace the current pipeline, orchestrator, memory bank, or A2A surfaces. The goal is to **upgrade loop resilience, observability, learning quality, memory governance, and multi-agent coordination** while preserving the correctness/safety guarantees already validated in Phase 8.

This document is the execution contract for Phase 9. It defines success criteria, safety invariants, rollout boundaries, benchmark scenarios, and acceptance metrics before deeper implementation work begins.

## Non-goals

- Do **not** introduce unbounded supervisor hierarchy.
- Do **not** silently replay dangerous side-effecting tools after restart.
- Do **not** make GUI or CLI invent core lifecycle decisions locally.
- Do **not** replace durable memory with opaque heuristic mutation.
- Do **not** break backward-compatible deserialization for persisted runs, environments, pauses, approvals, collaboration threads, or A2A state.

## Priority order

1. Crash-resumable delegated execution
2. Local subagent observability/control parity
3. Outcome-linked ERL with bounded corrective execution
4. Memory governance and retrieval quality
5. Shared multi-agent working memory and supervisor steering
6. Final rollout hardening, docs, and validation

## Done criteria

### D1. Crash-resumable delegated execution

Phase 9 may claim resumability complete only when all of the following are true:

- delegated local tasks persist a **checkpointed execution state** at safe boundaries
- restart reconciliation can distinguish:
  - resumable work
  - replay-safe restart-from-boundary work
  - operator-intervention-required work
- no duplicated side effects are observed across restart simulations
- operator surfaces expose the last safe boundary and the reason a task is resumable or blocked
- persisted runs/environments remain backward compatible with pre-Phase-9 state files

### D2. Local observability/control parity

Phase 9 may claim local observability parity complete only when:

- local delegated tasks emit structured progress/lifecycle events while executing
- per-tool timing and waiting states are visible to supervisors
- local tasks support bounded pause/cancel/resume controls at safe boundaries
- CLI, GUI, and core observer APIs render equivalent local-progress explanations
- local and remote task telemetry use coherent naming for phases, progress, artifacts, and failure reasons

### D3. Outcome-linked ERL

Phase 9 may claim ERL maturity complete only when:

- reflections can be linked to downstream outcomes such as approval, rejection, review pass/fail, and test pass/fail
- reflection quality scoring combines immediate heuristics with later verified outcomes
- corrective re-execution is bounded, explicit, replay-safe, and policy-aware
- failing strategies decay or are deprioritized over time
- learning changes are observable and explainable in audit/history surfaces

### D4. Memory governance

Phase 9 may claim durable memory governance complete only when:

- duplicate and conflicting memories can be detected and surfaced
- retrieval ranking uses more than raw text matching and metadata presence
- stale or low-confidence memories can decay/archive without destructive silent mutation
- promotion, merge, archive, and pin actions are explainable and reversible
- operator surfaces can inspect why a memory was injected or excluded

### D5. Shared multi-agent cognition

Phase 9 may claim shared cognition complete only when:

- supervisors and subagents can exchange bounded, durable mid-task findings
- shared cognition records authorship, timestamps, visibility scope, and confidence
- supervisors can steer active delegated work through explicit audited channels
- partial findings can be promoted into durable memory without losing provenance
- multi-agent coordination does not flood prompt context with low-signal chatter

### Current implementation checklist

- shared-cognition notes are persisted on `SupervisorRun.shared_cognition` with serde-safe defaults for legacy runs
- collaboration-message promotion mirrors scoped notes into durable memory under the `shared_cognition` category
- GUI workflow + memory-console surfaces expose authorship, confidence, task/directive provenance, and quick filtering for shared cognition
- CLI `/task tree` surfaces compact run-level shared-cognition summaries for steering and unresolved-hypothesis visibility
- prompt enrichment only injects scoped shared-cognition memory and caps retrieval to three entries per request
- focused tests cover supervisor steering, partial blocker/discovery publication, and conflicting multi-agent hypotheses

## Safety invariants

These invariants must hold for every implementation slice.

### Replay and side-effect invariants

- Never automatically replay a tool/action classified as `non_replayable_side_effect`.
- Never publish a delegated task result twice after restart.
- Never create duplicate artifact manifests/records for the same checkpoint boundary.
- Always store whether a step is resumable, replayable, or operator-gated.

### Persistence invariants

- New persisted records must deserialize cleanly from older on-disk state using sensible defaults.
- Old binaries are not required to understand new Phase-9-only fields, but new code must tolerate missing fields indefinitely.
- Recovery must remain idempotent: repeated reconciliation produces convergent state, not duplicate blocked reasons or duplicate cleanup actions.

### Visibility invariants

- Every supervisor-visible control must be represented in core-owned state.
- GUI/CLI are renderers and command initiators, not lifecycle decision engines.
- Any automatic learning or memory-governance action must leave an inspectable audit trail.

### Policy invariants

- Approval/review/test policies remain authoritative over corrective re-execution.
- Reflection-driven retries may only execute replay-safe or explicitly idempotent steps.
- Shared working memory must obey visibility scope and not leak supervisor-private reasoning into child prompts unless explicitly marked shareable.

## Replay-safety classes

Every checkpoint boundary and corrective retry path must classify work into one of these buckets.

1. `pure_readonly`
   - examples: file reads, code search, web fetch/search, status inspection
   - replay default: allowed
2. `idempotent_write`
   - examples: overwrite-safe state update with explicit idempotency key
   - replay default: allowed only with guard/key verification
3. `checkpoint_resumable`
   - examples: streamed model turn that can resume from saved state without repeating the prior side effect
   - replay default: resume from checkpoint only
4. `operator_gated`
   - examples: ambiguous side-effect state, partially completed external mutation
   - replay default: blocked pending operator action
5. `non_replayable_side_effect`
   - examples: shell command that mutated unknown external state without idempotency contract
   - replay default: never auto-replay

## Rollout and feature flags

Phase 9 must ship behind explicit rollout controls before any defaults change. Recommended config surface:

- `pipeline.resumable_delegation.enabled`
- `pipeline.resumable_delegation.max_checkpoints_per_task`
- `pipeline.local_subagent_streaming.enabled`
- `pipeline.local_subagent_controls.enabled`
- `pipeline.reflection.outcome_linking_enabled`
- `pipeline.reflection.corrective_reexecution_enabled`
- `memory.governance.enabled`
- `memory.governance.auto_decay_enabled`
- `memory.governance.auto_merge_suggestions_enabled`
- `orchestrator.shared_working_memory.enabled`

### Rollout phases

#### R0 — dark launch
- write/read new fields
- no user-visible behavior changes
- benchmark harness active in tests only

#### R1 — opt-in developer mode
- feature flags disabled by default
- CLI/GUI can display Phase 9 state when enabled
- telemetry and audit surfaces validated against fixtures

#### R2 — guarded opt-in production
- user-configurable flags available in config/UI where appropriate
- restart/resume and learning paths allowed only for explicitly safe classes

#### R3 — default-on for proven-safe slices
- only after benchmark acceptance thresholds are met
- keep fallback behavior available for at least one release window

## Compatibility boundaries

### Persisted state

- supervisor runs, environments, paused execution state, collaboration threads, approvals, and remote execution metadata must remain forward-tolerant to missing Phase 9 fields
- new checkpoint records should live in additive files/fields rather than replacing existing persisted state formats outright

### Remote peers

- local Phase 9 observability upgrades must not assume remote peers implement the same streaming/checkpoint semantics
- A2A compatibility negotiation remains authoritative for remote capabilities
- local checkpoint/resume metadata should not be advertised to peers unless stable protocol semantics exist

### Operator surfaces

- older sessions/runs lacking checkpoint data must remain inspectable and actionable
- CLI and GUI must gracefully render `not available`, `legacy run`, or `unsupported by task mode` states rather than implying missing data is an error

## Benchmark and replay fixture matrix

The initial fixture matrix lives in:

- `crates/gestura-core-pipeline/testdata/phase9/benchmark_scenarios.json`

Every Phase 9 implementation slice must update or add fixtures when behavior changes.

### Required benchmark categories

1. `restart_resume`
   - crash during planning
   - crash between tool calls
   - crash after tool completion before result persistence
   - crash during streamed local delegated execution
2. `observability`
   - local tool-heavy delegated task
   - local paused task resumed from checkpoint
   - local vs remote equivalent task parity comparison
3. `erl_outcomes`
   - heuristic low-quality answer later validated as good
   - answer rejected by approval/review/test outcome
   - bounded corrective retry improves result without unsafe replay
4. `memory_governance`
   - duplicate lessons
   - contradictory lessons
   - stale but historically successful strategy
   - scope conflict between directive-local and repo-global memory
5. `shared_cognition`
   - subagent publishes partial blocker mid-task
   - supervisor steering updates child execution scope
   - conflicting hypotheses from multiple agents

## Observability scorecard

Phase 9 acceptance should compare pre- and post-implementation behavior using these metrics.

### Resilience

- resumable-task recovery success rate
- operator-blocked recovery rate (expected for unsafe cases)
- duplicate-side-effect incidence (target: zero)
- average time to reconcile restart state

### Local execution observability

- percentage of delegated tasks with live phase/progress updates
- percentage of tool calls with non-zero duration/timing data
- supervisor UI/API visibility latency for local task progress
- pause/resume success rate at safe boundaries

### Learning quality

- fraction of promoted reflections later correlated with successful outcomes
- fraction of decayed/archived reflections correlated with failed outcomes
- improvement delta for bounded corrective retries on benchmark scenarios
- unsafe corrective retry count (target: zero)

### Memory quality

- duplicate memory detection precision/recall on fixture corpus
- conflicting memory surfacing rate
- retrieval precision at top-k for benchmark tasks
- operator override reversibility and audit completeness

### Multi-agent cognition

- time from subagent blocker discovery to supervisor visibility
- ratio of useful shared-memory publications vs noisy discarded publications
- successful supervisor steering interventions without context corruption

## Implementation order

1. Add checkpoint schema + replay-safety classification
2. Persist checkpoint state + recovery scheduler
3. Move local delegated execution onto streaming telemetry
4. Add pause/cancel/resume controls for local delegated work
5. Add outcome model and ERL linkage
6. Add memory governance jobs and retrieval ranking
7. Add shared working-memory domain + supervisor steering
8. Run full validation and decide default-on rollout per slice

## Definition of complete for Phase 9

Phase 9 is complete only when:

- all benchmark categories have deterministic fixtures
- focused validation passes for each capability slice
- workspace-level validation passes after the last slice lands
- docs explain the operator model and failure modes
- any remaining non-blocking work is explicitly tracked as a follow-on, not left implicit

Recommended final validation for this slice:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- frontend regression coverage for workflow + memory console surfaces (`npm run test:unit` in `crates/gestura-gui/frontend`)