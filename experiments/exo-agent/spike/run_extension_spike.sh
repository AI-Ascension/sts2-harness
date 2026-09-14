#!/usr/bin/env bash
# Real pinned-Exo execution spike for the selected TypeScript STS2 extension.
#
# Env:
#   EXO_BIN       path to the pinned, already-built `exo` binary (required)
#   EXO_SRC_ROOT  Exo candidate checkout (required; the extension is copied into it)
#   SPIKE_OUT     output directory outside the repository (required)
#   SPIKE_PORT    local port (default 47813)
#   NODE_BIN_DIR  directory containing `node` (optional; added to PATH)
#
# Runs one turn through experiments/exo-agent/extension/src/index.ts against an
# original synthetic model endpoint and asserts exactly one model call with a
# correlated turn record and the terminal decision text. No real provider or game
# is contacted.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTENSION_SRC="$SCRIPT_DIR/../extension/src/index.ts"
: "${EXO_BIN:?set EXO_BIN to the built pinned exo binary}"
: "${EXO_SRC_ROOT:?set EXO_SRC_ROOT to the pinned Exo candidate checkout}"
: "${SPIKE_OUT:?set SPIKE_OUT to an output directory outside the repository}"
PORT="${SPIKE_PORT:-47813}"
if [[ -n "${NODE_BIN_DIR:-}" ]]; then
  export PATH="$NODE_BIN_DIR:$PATH"
fi
mkdir -p "$SPIKE_OUT"

SYNTH_BIN="$SPIKE_OUT/synthetic_model"
"${RUSTC:-rustc}" -O "$SCRIPT_DIR/synthetic_model.rs" -o "$SYNTH_BIN"

MODULE_REL="experiments/exo-agent/extension/src/index.ts"
mkdir -p "$EXO_SRC_ROOT/$(dirname "$MODULE_REL")"
cp "$EXTENSION_SRC" "$EXO_SRC_ROOT/$MODULE_REL"

DECISION='{"decision":"wait","rationale":"synthetic spike decision"}'
ROOT="$(mktemp -d "$SPIKE_OUT/root.XXXXXX")"
XDG="$(mktemp -d "$SPIKE_OUT/xdg.XXXXXX")"
REQLOG="$SPIKE_OUT/requests.jsonl"
EVENTS="$SPIKE_OUT/events.json"
REPORT="$SPIKE_OUT/report.txt"
: > "$REQLOG"

"$SYNTH_BIN" "$PORT" "$REQLOG" "$DECISION" > "$SPIKE_OUT/synth.out" 2>&1 &
SYNTH_PID=$!
trap 'kill "$SYNTH_PID" 2>/dev/null || true' EXIT
sleep 1

cd "$EXO_SRC_ROOT"
export XDG_CONFIG_HOME="$XDG"
export OPENAI_API_KEY="sk-synthetic-key"
EXO=("$EXO_BIN" --root "$ROOT" --secret-backend file --harness typescript)

{
  echo "exo_source_revision=$(git -C "$EXO_SRC_ROOT" rev-parse HEAD)"
  echo "exo_executable_digest=$(sha256sum "$EXO_BIN" | awk '{print $1}')"
  echo "extension_sha256=$(sha256sum "$EXTENSION_SRC" | awk '{print $1}')"
  echo "node_version=$(node --version 2>/dev/null || echo unavailable)"
} > "$REPORT"

"${EXO[@]}" secret set test-key --env OPENAI_API_KEY >> "$REPORT" 2>&1
"${EXO[@]}" model register gpt-test --secret test-key --base-url "http://127.0.0.1:$PORT" >> "$REPORT" 2>&1
"${EXO[@]}" agent create --slug spike-agent --model gpt-test --provider local-process \
  --max-tool-round-trips 0 --module "$MODULE_REL" "Spike Agent" >> "$REPORT" 2>&1
"${EXO[@]}" conversation create spike-agent first >> "$REPORT" 2>&1
timeout 150 "${EXO[@]}" conversation send spike-agent first "hello from the sts2 extension spike" >> "$REPORT" 2>&1
"${EXO[@]}" conversation events spike-agent first > "$EVENTS" 2>> "$REPORT"

MODEL_CALLS=$(grep -c -e '"path":"/responses"' -e '"path":"/chat/completions"' "$REQLOG" || true)
echo "model_calls=$MODEL_CALLS" >> "$REPORT"
if [[ "$MODEL_CALLS" != "1" ]]; then
  echo "ASSERT FAILED: expected exactly one model call, got $MODEL_CALLS" >&2
  exit 1
fi
if ! grep -q '"turn_id"' "$EVENTS"; then
  echo "ASSERT FAILED: no correlated turn_id in events" >&2
  exit 1
fi
if ! grep -q "synthetic spike decision" "$EVENTS"; then
  echo "ASSERT FAILED: terminal decision text missing from events" >&2
  exit 1
fi

echo "wrote $REPORT"