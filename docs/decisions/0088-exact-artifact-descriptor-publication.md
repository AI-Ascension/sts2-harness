# ADR 0088: Publish exact artifacts through held descriptors without added privilege

Status: supervisor-accepted design and source candidate; runtime qualification at this source-review increment: NOT_RUN.

## Context

Issue [#847](https://github.com/AI-Ascension/sts2-harness/issues/847) reproduces a normal
unprivileged Linux failure: `O_TMPFILE` creation, write, and file sync succeed, but
`linkat(..., AT_EMPTY_PATH)` returns `ENOENT` without `CAP_DAC_READ_SEARCH`. Treating that write
failure as `Missing` misstates an unavailable publication operation as an absent artifact. Linux
[`linkat(2)`](https://man7.org/linux/man-pages/man2/linkat.2.html) documents procfs descriptor
resolution with `AT_SYMLINK_FOLLOW` as the alternative.

## Decision

`ExactArtifactStore` keeps its descriptor-confined directory walk, no-follow component and leaf
opens, size and digest verification, and unnamed `O_TMPFILE` staging. It first links the still-open
temporary through `AT_EMPTY_PATH` into the already-held destination directory. This preserves the
existing successful path without requiring procfs. Only `ENOENT` selects the unprivileged fallback;
all other errors retain their persistence or `EEXIST` idempotency behavior.

The fallback opens `/proc` as a no-follow directory and verifies that held root with `fstatfs`
against `PROC_SUPER_MAGIC` before deliberately resolving its kernel-owned `self` entry. It then
opens `fd` relative to the held process directory, without following that final component, and
verifies the held descriptor directory is procfs. It links the decimal name of the still-open
temporary descriptor relative to that held directory, with `AT_SYMLINK_FOLLOW`, into the already-held
destination directory descriptor. The source name is derived only from the live descriptor; callers
cannot select it. The temporary and destination descriptors remain held through publication.

The temporary is synced before linking and the destination directory is synced after linking. A
concurrent `EEXIST` is resolved by opening the final leaf without following links, verifying a
regular file with identical bounded bytes, then syncing it and its directory. Nothing is
overwritten. No named staging file or caller-selected source path is used. Publication, procfs
lookup, or sync failure returns a persistence error; only an ordinary absent read remains `Missing`.
Existing content is idempotent, and the 16 MiB bound and private file mode remain unchanged.

## Compatibility and limits

This is Linux-only and depends on `O_TMPFILE` and a filesystem that supports hard-link publication.
The original `AT_EMPTY_PATH` route does not depend on procfs; the fallback additionally requires
real procfs mounted for the calling process. Unsupported platforms, filesystems, or fallback
environments fail closed. Descriptor handles are retained only for the current operation and close
on return. Failure before linking leaves no named temporary; if directory sync fails after linking,
the content-addressed entry may be visible and a retry verifies its bytes before syncing again. No
privilege, uid, configuration, wire, or schema change is introduced. The harness owns artifact bytes
and retention metadata, not game or host state; this path has no game-process access.

At this source-review increment, the supervisor-reviewed source candidate and existing
unprivileged publication/confinement tests were source-only; locked Linux Rust checks and
corrected-path runtime qualification were NOT_RUN. No later gate result or runtime, filesystem,
native, provider, or game compatibility is claimed. A synthetic syscall reproduction and the
production failure receipt identify the old failure; neither proves the corrected runtime path,
filesystem portability, or game compatibility.

Refs #847.
