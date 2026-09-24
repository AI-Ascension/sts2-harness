# Changelog archive: 2026-09-24

This file preserves completed `## Unreleased` history that was moved out of
[`CHANGELOG.md`](../CHANGELOG.md) when the active changelog reached its preferred Markdown size
budget. Entries are unchanged from the revision that introduced them apart from relative link paths,
which are corrected so they resolve from this directory; this file is a verbatim record, not a
supported release or a second normative changelog.

### Archived from CHANGELOG.md

- Derive the **presented option set from the state** instead of offering a provider the whole legal
  catalog. `context_control::OptionSelection` folds catalog entries that are identical under the
  admitted action vocabulary — the same kind aimed at the same target, differing only in which copy
  of a card in hand it names — into one presented option, and records every fold with the option it
  folded into, so a replay can show exactly what the provider was and was not offered. It reports
  `forced` when one action is legal and no question is needed, `single` when the presented options
  fit one question, and `two_stage` above a declared bound, where a kind is asked before an action
  within it. Presented and withheld entries partition the catalog; presented order is catalog order
  and nothing here ranks, scores, or prefers an action. A selection that would leave fewer than two
  options presents the catalog unchanged, because one option is not a question. Affordability is
  deliberately not a withholding rule: the host lists an action only when it is legal, so filtering
  on cost could only ever overrule that authority, and affordability stays in the derived-exact facts
  beside the state. Compatibility: additive; one new module and its re-exports, no change to an
  existing record, route, or digest. Refs #290.
