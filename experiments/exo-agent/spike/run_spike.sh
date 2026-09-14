#!/usr/bin/env bash
# Reproducible driver for the STS2-Exo contract spike (#139).
#
# Env:
#   EXO_BIN       path to the pinned, already-built `exo` binary (required)
#   EXO_SRC_ROOT  Exo source checkout (required for the TypeScript harness path)
#   SPIKE_OUT     output directory outside the repository (required)
#   SPIKE_PORT    first local port (default 47811)
#   NODE_BIN_DIR  directory containing `node` (optional; added to PATH)
#
# For each harness it runs one turn against experiments/exo-agent/spike/synthetic_model.rs
# and asserts exactly one model call with a correlated turn record. No real provider or
# game is contacted.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${EXO_BIN:?set EXO_BIN to the built pinned exo binary}"
: "${EXO_SRC_ROOT:?set EXO_SRC_ROOT to the pinned Exo source checkout}"
: "${SPIKE_OUT:?set SPIKE_OUT to an output directory outside the repository}"
BASE_PORT="${SPIKE_PORT:-47811}"
if [[ -n "${NODE_BIN_DIR:-}" ]]; then
  export PATH="$NODE_BIN_DIR:$PATH"
fi
mkdir -p "$SPIKE_OUT"

SYNTH_BIN="$SPIKE_OUT/synthetic_model"
"${RUSTC:-rustc}" -O "$SCRIPT_DIR/synthetic_model.rs" -o "$SYNTH_BIN"

SYNTH_PID=""
cleanup() { [[ -n "$SYNTH_PID" ]] && kill "$SYNTH_PID" 2>/dev/null || true; }
trap cleanup EXIT

run_path() {
  local mode="$1" port="$2"
  local root xdg reqlog report events
  local -a global_flags=() agent_flags=()
  if [[ "$mode" == "exo" ]]; then
    global_flags=(--harness exo)
    agent_flags=(--module exo/harness.ts)
  fi

  root="$(mktemp -d "$SPIKE_OUT/${mode}-root.XXXXXX")"
  xdg="$(mktemp -d "$SPIKE_OUT/${mode}-xdg.XXXXXX")"
  reqlog="$SPIKE_OUT/${mode}-requests.jsonl"
  report="$SPIKE_OUT/${mode}-report.txt"
  events="$SPIKE_OUT/${mode}-events.json"
  : > "$reqlog"

  "$SYNTH_BIN" "$port" "$reqlog" "synthetic decision" \
    > "$SPIKE_OUT/${mode}-synth.out" 2>&1 &
  SYNTH_PID=$!
  sleep 1

  cd "$EXO_SRC_ROOT"
  export XDG_CONFIG_HOME="$xdg"
  export OPENAI_API_KEY="sk-synthetic-key"

  {
    echo "mode=$mode"
    echo "exo_source_revision=$(git -C "$EXO_SRC_ROOT" rev-parse HEAD)"
    echo "exo_binary_sha256=$(sha256sum "$EXO_BIN" | awk '{print $1}')"
    echo "node_version=$(node --version 2>/dev/null || echo unavailable)"
  } > "$report"

  local -a exo=("$EXO_BIN" --root "$root" --secret-backend file "${global_flags[@]}")
  "${exo[@]}" secret set test-key --env OPENAI_API_KEY >> "$report" 2>&1
  "${exo[@]}" model register gpt-test --secret test-key --base-url "http://127.0.0.1:$port" >> "$report" 2>&1
  "${exo[@]}" agent create --slug spike-agent --model gpt-test --provider local-process \
    --max-tool-round-trips 0 "${agent_flags[@]}" "Spike Agent" >> "$report" 2>&1
  "${exo[@]}" conversation create spike-agent first >> "$report" 2>&1
  timeout 150 "${exo[@]}" conversation send spike-agent first "hello from the sts2 spike" >> "$report" 2>&1
  "${exo[@]}" conversation events spike-agent first > "$events" 2>> "$report"

  # Count only the two model-execution endpoints, not any health/liveness probe.
  local model_calls
  model_calls=$(grep -c -e '"path":"/responses"' -e '"path":"/chat/completions"' "$reqlog" || true)
  echo "model_calls=$model_calls" >> "$report"
  if [[ "$model_calls" != "1" ]]; then
    echo "ASSERT FAILED ($mode): expected exactly one model call, got $model_calls" >&2
    exit 1
  fi
  if ! grep -q '"turn_id"' "$events"; then
    echo "ASSERT FAILED ($mode): no correlated turn_id in events" >&2
    exit 1
  fi
  if ! grep -q "synthetic decision" "$events"; then
    echo "ASSERT FAILED ($mode): assistant text missing from events" >&2
    exit 1
  fi

  cleanup
  SYNTH_PID=""
  echo "wrote $report"
}

run_path basic "$BASE_PORT"
run_path exo "$((BASE_PORT + 1))"