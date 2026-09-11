#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Run the actual Astra bridge against the actual map renderer while replacing only the provider
# CLI executable with a private, bounded capture shim. No provider credentials or network access
# are read by this script.

set -euo pipefail

die() {
  printf 'map-provider-capture: %s\n' "$1" >&2
  exit 2
}

usage() {
  cat >&2 <<'EOF'
usage: capture.sh --bridge BUILT_STS2_ASTRA_BRIDGE --renderer PINNED_MAP_VISUALIZER [options]
  --fixture FIXTURE_ROOT       defaults to the checked-in full demo fixture
  --provider-revision DIGEST   40/64 lowercase hex; defaults to harness HEAD
  --model-execution-id ID      defaults to capture-model-execution-001
  --report PATH                defaults to harness .orchestration/provider-cli-capture.md
EOF
  exit 2
}

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
harness_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
fixture="$harness_root/crates/harness/tests/fixtures/map-bundle-demo-v1"
bridge=''
renderer=''
provider_revision=''
model_execution_id='capture-model-execution-001'
report="$harness_root/.orchestration/provider-cli-capture.md"

while (($# > 0)); do
  case "$1" in
    --bridge|--renderer|--fixture|--provider-revision|--model-execution-id|--report)
      (($# >= 2)) || usage
      key=$1
      value=$2
      shift 2
      case "$key" in
        --bridge) bridge=$value ;;
        --renderer) renderer=$value ;;
        --fixture) fixture=$value ;;
        --provider-revision) provider_revision=$value ;;
        --model-execution-id) model_execution_id=$value ;;
        --report) report=$value ;;
      esac
      ;;
    -h|--help) usage ;;
    *) usage ;;
  esac
done

[[ -n "$bridge" ]] || usage
[[ -n "$renderer" ]] || usage
[[ -x "$bridge" && -f "$bridge" ]] || die 'the required built sts2-astra-bridge executable is absent or not executable'
[[ -x "$renderer" && -f "$renderer" ]] || die 'the required product renderer executable is absent or not executable'
[[ -d "$fixture" ]] || die 'the requested map fixture directory is absent'

for command_name in node timeout mktemp find cp chmod sha256sum git; do
  command -v "$command_name" >/dev/null 2>&1 || die "required command is unavailable: $command_name"
done

renderer_sha256=$(sha256sum "$renderer" | awk '{print $1}')
[[ "$renderer_sha256" == '782d0d24d795a35335b84dcb8b08458af5e97eeca71b853914cc8b590ad1462f' ]] || die 'renderer SHA-256 is not the pinned product artifact'
bridge_sha256=$(sha256sum "$bridge" | awk '{print $1}')

if [[ -z "$provider_revision" ]]; then
  provider_revision=$(git -C "$harness_root" rev-parse HEAD 2>/dev/null) || die 'cannot derive provider revision from harness HEAD'
fi
[[ "$provider_revision" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || die 'provider revision must be lowercase 40/64-hex'

bundle="$fixture"
if [[ ! -f "$bundle/visible-map.json" ]]; then
  mapfile -t bundle_candidates < <(find "$fixture" -mindepth 1 -maxdepth 1 -type d -print)
  [[ "${#bundle_candidates[@]}" == 1 ]] || die 'fixture must contain exactly one digest bundle directory'
  bundle=${bundle_candidates[0]}
fi
[[ -f "$bundle/visible-map.json" && -f "$bundle/manifest.json" ]] || die 'fixture bundle lacks required map artifacts'

tmp=$(mktemp -d "${TMPDIR:-/tmp}/sts2-map-provider-capture.XXXXXX") || die 'cannot create private temporary directory'
chmod 700 "$tmp"
cleanup() { rm -rf -- "$tmp"; }
trap cleanup EXIT
mkdir -m 700 "$tmp/capture" "$tmp/fake-bin"

timeout --signal=TERM --kill-after=3s 5s "$bridge" --describe >"$tmp/describe.json" 2>"$tmp/describe.err" || {
  cat "$tmp/describe.err" >&2
  die 'bridge --describe failed'
}
node "$script_dir/verify-describe.mjs" "$tmp/describe.json" || die 'bridge descriptor preflight failed'

timeout --signal=TERM --kill-after=3s 20s "$renderer" validate --bundle "$bundle" >"$tmp/validate.json" 2>"$tmp/validate.err" || {
  cat "$tmp/validate.err" >&2
  die 'product renderer rejected the full graph fixture'
}
timeout --signal=TERM --kill-after=3s 20s "$renderer" render --bundle "$bundle" --out "$tmp/rendered" --width 1600 --height 2400 >"$tmp/render.json" 2>"$tmp/render.err" || {
  cat "$tmp/render.err" >&2
  die 'product renderer failed to render the full graph fixture'
}

node "$script_dir/build-request.mjs" \
  --snapshot "$bundle/visible-map.json" \
  --png "$tmp/rendered/overview.png" \
  --rendered-manifest "$tmp/rendered/manifest.json" \
  --out "$tmp/request.json" \
  --metadata "$tmp/metadata.json" \
  --provider-revision "$provider_revision" \
  --model-execution-id "$model_execution_id" || die 'serialized Exo map request construction failed'

cat >"$tmp/fake-bin/codex" <<'FAKE_CODEX'
#!/usr/bin/env bash
set -euo pipefail
capture=${MAP_CAPTURE_DIR:?}
if [[ -e "$capture/invocation.count" ]]; then
  exit 71
fi
printf '1' >"$capture/invocation.count"
printf '%s\0' "$@" >"$capture/args.nul"
cat >"$capture/prompt.bin"
image_path=''
output_path=''
for ((index = 1; index <= $#; index += 1)); do
  argument=${!index}
  next=$((index + 1))
  case "$argument" in
    --image) image_path=${!next:-} ;;
    --output-last-message) output_path=${!next:-} ;;
  esac
done
[[ -n "$image_path" && -n "$output_path" ]] || exit 72
cp -- "$image_path" "$capture/image.bin"
node --input-type=module - "$capture/prompt.bin" "$output_path" "$capture/decision.bin" <<'NODE'
import fs from 'node:fs';
const prompt = fs.readFileSync(process.argv[2], 'utf8');
const marker = prompt.lastIndexOf('\n{');
if (marker < 0) process.exit(73);
const request = JSON.parse(prompt.slice(marker + 1));
const actionId = request.legal_action_ids?.[0];
if (typeof actionId !== 'string' || actionId.length === 0) process.exit(74);
const decision = Buffer.from(JSON.stringify({ action_ids: [actionId], rationale: 'bounded fake capture' }));
if (decision.length > 8192) process.exit(75);
fs.writeFileSync(process.argv[3], decision, { flag: 'w' });
fs.writeFileSync(process.argv[4], decision, { flag: 'w' });
NODE
FAKE_CODEX
chmod 700 "$tmp/fake-bin/codex"

PATH="$tmp/fake-bin:$PATH" MAP_CAPTURE_DIR="$tmp/capture" \
  timeout --signal=TERM --kill-after=3s 20s "$bridge" <"$tmp/request.json" >"$tmp/bridge-output.json" 2>"$tmp/bridge.err" || {
    cat "$tmp/bridge.err" >&2
    die 'bridge execution failed or exceeded the finite capture timeout'
  }

node "$script_dir/verify-capture.mjs" \
  --request "$tmp/request.json" \
  --prompt "$tmp/capture/prompt.bin" \
  --image "$tmp/capture/image.bin" \
  --source-png "$tmp/rendered/overview.png" \
  --args "$tmp/capture/args.nul" \
  --count "$tmp/capture/invocation.count" \
  --decision "$tmp/capture/decision.bin" \
  --bridge-output "$tmp/bridge-output.json" \
  --render-json "$tmp/render.json" \
  --metadata "$tmp/metadata.json" \
  --bridge-sha256 "$bridge_sha256" \
  --renderer-sha256 "$renderer_sha256" \
  --report "$report"
