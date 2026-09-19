#!/usr/bin/env bash
# Reclaim stale cargo build caches under the session data directory.
#
# Only directories cargo itself marked with CACHEDIR.TAG, only ones untouched since the cutoff,
# and never anything under ~/.cache, ~/.local, ~/.cargo, ~/.rustup or ~/.npm - emptying those has
# broken toolchains on this host before. Nothing here is source: every one of these rebuilds.
#
#   sudo bash sweep-cargo-targets.sh            # show what would go, delete nothing
#   sudo bash sweep-cargo-targets.sh --apply    # actually delete
set -uo pipefail

D="${SWEEP_ROOT:-/home/completetrain/session.home.complete.tech/data}"
CUTOFF="${SWEEP_CUTOFF:-2026-09-18}"
LOG_DIR="${SWEEP_LOG_DIR:-/home/completetrain/jev-gpu-prep}"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG="$LOG_DIR/target-sweep-$STAMP.log"
APPLY=no
[ "${1:-}" = "--apply" ] && APPLY=yes

mkdir -p "$LOG_DIR"

# A build in flight must not have its output pulled out from under it. Refusing outright whenever
# any cargo is running is too blunt on a host where agents build around the clock - it would mean
# the sweep never runs. Instead each candidate is checked on its own: the cutoff already excludes
# anything touched today, and this additionally skips any directory a live process names.
in_use() {
    local dir="$1" cmd
    for cmd in /proc/[0-9]*/cmdline; do
        # A process can exit between the glob and the read; that is not an error worth printing.
        [ -r "$cmd" ] || continue
        # grep -a rather than translating the NUL separators first: a path never spans a NUL,
        # and passing one through an argument does not survive - the shell truncates there.
        grep -qaF -- "$dir" "$cmd" 2>/dev/null && return 0
    done
    return 1
}

total=0
count=0
: > "$LOG"
while IFS= read -r p; do
    [ -d "$p" ] || continue
    [ -f "$p/CACHEDIR.TAG" ] || continue          # re-checked at delete time, not just at survey
    touched="$(stat -c %y "$p" | cut -c1-10)"
    [ "$touched" \< "$CUTOFF" ] || continue
    if in_use "$p"; then
        printf '  in use, skipping   %s
' "$p"
        continue
    fi
    mib="$(du -xs --block-size=1M "$p" 2>/dev/null | cut -f1)"
    total=$((total + mib))
    count=$((count + 1))
    printf '%s\t%s MiB\t%s\n' "$touched" "$mib" "$p" >> "$LOG"
    if [ "$APPLY" = yes ]; then
        rm -rf -- "$p" && printf '  removed %6s MiB  %s\n' "$mib" "$p"
    else
        printf '  would remove %6s MiB  %s\n' "$mib" "$p"
    fi
done < <(find "$D" -type d -name target -prune \
    -not -path "*/.cache/*" -not -path "*/.local/*" \
    -not -path "*/.cargo/*" -not -path "*/.rustup/*" -not -path "*/.npm/*" 2>/dev/null)

printf '\n%s: %d directories, %.1f GiB\n' \
    "$([ "$APPLY" = yes ] && echo removed || echo "would remove")" "$count" \
    "$(awk -v t="$total" 'BEGIN {printf "%.1f", t/1024}')"
echo "list: $LOG"
df -h / | tail -1
