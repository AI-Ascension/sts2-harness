# ADR 0011: Historical recovery evidence binding

## Status and ownership

Implemented consumer candidate; independent and integrated release-set validation are pending.
The harness owns retained experiment/operation identity. MCP owns the transport translation;
gateway and host own authoritative operation outcomes. This decision changes no wire schema,
host authority, frozen Runtime-v3 artifact or gameplay provider policy.

## Decision

Parse bounded raw recovery JSON with duplicate-member rejection before projecting a value.
Require the expected contract/digest, response kind, closed shapes, strict wire values and a
gateway actor matching its auth principal. Reject malformed input with bounded messages that
do not reflect supplied fields, proofs or values. The outer MCP request identity remains checked;
the inner sideband correlation remains a separate MCP-boundary responsibility and is not newly
claimed to be independently correlated by this consumer.

Use one bounded action decoder for validation and retained-byte comparison. The existing
artifact permits standard and URL-safe alphabets, with or without padding. Gateway producers
use unpadded standard encoding; host producers may use padded encoding. Reject invalid padding
and nonzero tail bits without narrowing the accepted artifact to one producer's encoding.
The decoded bytes must still exactly equal the durable canonical action and its SHA-256 digest.

Historical lookup must return mutation_authorized=false. Compare any retained record with the
requested original context, operation ID, payload digest and expected state/catalog boundary.
An unresolved lookup proceeds to authenticated reconciliation using that same reference, never
to ordinary gameplay polling or redispatch. NOT_FOUND remains unresolved. If reconciliation
returns an authoritative terminal settlement under the same instance incarnation, the consumer
first treats the historical result as resolved for this recovery decision, then must obtain a
fresh ordinary gameplay observation and transition witness before exposing a usable settled
receipt. Durable closure is deferred until that fresh receipt is validated; a failed, unresolved
or no-observation read leaves the operation unresolved. The retained-witness read is one
immediate bounded request, not the ordinary 120-second transition barrier, so it cannot silently
extend the recovery deadline.

The gateway may preserve an already terminal SETTLED/REJECTED record when asked to reconcile.
Accept those states and RECONCILED only when the response result agrees with the record and its
terminal ticket. Bind ticket operation/digest, original boot/incarnation/epoch and witness fence.
SETTLED or RECONCILED requires an operation-specific witness. Its operation/digest,
original boot/incarnation and host fence must match; its generation must be greater than the
original expected generation. That comparison alone is never evidence: it is checked only as
one field of an otherwise bound authoritative witness. A terminal REJECTED ticket may lack an
effect witness. Top-level and nested reconcile witnesses must agree.

## Verification and limitations

Unit tests reject the previously accepted settled ticket without a witness, context and ticket
substitution, unrelated generation movement, inconsistent witnesses and statuses. Synthetic
stdio subprocess tests assert unresolved state is retained and no gameplay poll/dispatch occurs.
The producer state/encoding behavior was inspected in gateway source; these harness tests are
not an execution of the combined gateway/MCP/host release set.

The legacy environment-supplied original context is still insufficient for multiple historical
boots. Execution-store schema v6 now persists the immutable original context with each operation
before possible dispatch, while fresh allocation/recovery authority is constructed separately for
every recovery sideband. Legacy operations with NULL context remain unresolved and block recovery;
no context is inferred from the current allocation or environment. This closes the consumer-side
cross-boot identity dependency, but does not claim live gateway/host reboot settlement or a
complete watchdog release set.
