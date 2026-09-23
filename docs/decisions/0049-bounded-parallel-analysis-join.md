# ADR 0049: Bounded Parallel Analysis Execution with Identity-Ordered Joins

## Status

Accepted for the Harness dynamic-analysis planning boundary. This record does not
authorize a provider, native-host, game, deployment, or paid-call lane, and it
does not change the host's authority over legality, mutation, or observations.

## Decision

The admitted dynamic analysis DAG (`DynamicPlan`, executed by `execute_plan`) gains a
second execution route, `execute_plan_bounded`, that runs independent `Analyze`
nodes concurrently under an owner-enforced in-flight cap and joins their results
by node identity. Until this record `WorkflowLimits::max_parallel_analyses` was
declared, range-checked (`1..=4`) and shown in Studio, but no executor read it:
`execute_plan` walked a topological order serially with one `&mut` executor, a
failing analysis aborted the whole plan, and a graph drawing was the only
statement of concurrency.

No new `NodeKind` is added. Fork and join are structural facts of the existing
plan (edges and `Decide.inputs`), so no schema, palette or Studio pin changes.
Compatibility: `additive-compatible`. New items only (`ParallelAnalysisExecutor`,
`AnalysisFault`, `ParallelCap`, `BranchOutcome`, `JoinedResult`,
`execute_plan_bounded`, three `DynamicPlanError` variants); no existing field,
route, durable record or published schema changes, and `execute_plan` keeps
its behaviour. Budget reservation, cancel/restart and browser branch states
stay open (below).

## Cap

`ParallelCap` is built from `WorkflowLimits::max_parallel_analyses`
(`ParallelCap::from_limits`) or directly (`ParallelCap::new`); `0` and anything
above `MAX_PARALLEL_ANALYSES = 4` is refused as `Capacity`. The scheduler loop in
`crates/harness/src/workflow/dynamic_parallel.rs` dispatches a ready node only
while `in_flight < cap`, so the bound is enforced by the owner, not by the
executor and not by the plan author. Ready nodes are dispatched in `NodeId`
order. A `Decide` node's `inputs` list is a dependency even when the plan draws
no edge for it, which is stricter than the serial route's edge-only order. Every
executor call — analyses and the decision itself — counts against the cap. `ParallelCap::SERIAL` (cap=1) is the compatibility route: one branch at
a time, in the same topological order the serial route uses.

## Executor contract

`ParallelAnalysisExecutor: Send + Sync` takes `&self` and returns
`Result<AnalysisValue, AnalysisFault>` where `AnalysisFault::Failed(error)` is a
typed refusal and `AnalysisFault::Unknown` means the executor cannot say whether
the analysis ran (a lost provider response, for example). Implementations own
their interior synchronization. `PureAnalysisExecutor` and `execute_plan` are
unchanged.

A branch receives exactly its **declared** inputs: the values of its edge
predecessors and, for `Decide`, its `inputs` list. The serial route passes every
value settled so far, which is order-visible and would make results depend on
completion timing once the cap exceeds one. Consequently the two routes produce
identical values at cap=1 for an executor that reads declared inputs; an executor
that relies on undeclared, earlier-settled values must stay on the serial route.

## Join, outcomes and policy

Each node ends in one `BranchOutcome`: `Settled(value)`, `Failed(error)` or
`Unknown`. The route never aborts on a branch fault: every node that can be
scheduled runs, and the result is a `JoinedResult { plan_digest, outcomes,
join_digest }` where `outcomes` is a `BTreeMap<NodeId, BranchOutcome>`.
`join_digest` is the SHA-256 of a canonical record of the plan digest and the
identity-ordered entries (node id, state, settled value or error text), so a
permuted completion order yields byte-identical `outcomes` and `join_digest`.

Policy for a node whose declared input did not settle (failed, unknown, or
absent): the node is **not dispatched** and is recorded as
`Failed(DynamicPlanError::UnsettledInput)`. This applies to dependent `Analyze`
nodes and to `Decide`, so a failed or unknown branch can never be a decision
input and the decision profile is never invoked with a partial input set.
`JoinedResult::into_plan_result` projects the join onto the serial result type
and succeeds only if every node settled; otherwise it returns the first
non-settled node's error in identity order (`Unknown` maps to `BranchUnknown`).
A plan whose nodes never all become ready is refused as `Cycle`; a branch worker
that ends without reporting is `BranchLost`. The executor is caller-supplied, so
a branch that unwinds is caught at the worker boundary and recorded as
`Failed(BranchLost)` rather than stalling the join: every dispatched branch
reports exactly one outcome, which is what lets the owner loop wait without a
deadline.

## Out of scope for this record

Per-branch provider budget reservation, cancellation and restart of branches
(sts2-harness#98 items 2–3, lane 98-B) are decided by ADR 0065. Branch-state
reporting to Studio (closed `RunSnapshot`/`RunEvent` schemas, lane 98-C) remains a
separate decision in `ascension-workflow-studio`. Game mutations stay serialized
at the protected action boundary; this route executes read-only analyses only.
Native, provider and browser behavior are not established by the controlled
executor used in `tests/workflow_dynamic_parallel.rs`.
