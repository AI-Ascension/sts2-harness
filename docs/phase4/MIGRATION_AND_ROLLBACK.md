# Migration and rollback boundary

The provider-session records are additive and versioned. Existing capture, Phase 2 control and
Phase 3 memory records remain unchanged; legacy stateless runs remain unbound. A downgrade must
hold execution and keep active bindings read-only until the session records are reconciled or
retired. Restoring a backup applies revocation and retirement epochs before any dependent binding
is admitted. Native state cleanup is separate from local record deletion and may remain unknown.
