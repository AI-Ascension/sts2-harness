#!/usr/bin/env bash
# Jev plays Slay the Spire 2, one episode at a time, restarting the whole session after each one.
#
# The game is started through the supported launcher,
# native-launch-inputs-steam-20260908/launch-linux-standard-campaign.py, which is what loads the
# mod. An earlier version of this loop started the binary directly, the mod never loaded, and the
# runtime session on 127.0.0.1:15626 never opened. The launcher must run as root, which is why the
# desktop entry uses pkexec; it drops to uid 1000 for the game itself.
#
# AUTHORIZATION: the operator plan this launcher was prepared under records a guard of "no provider
# call". These episodes deliberately make provider calls: the owner asked for a model-driven loop on
# this lane on 2026-09-18 and reaffirmed it after that guard was raised. Every episode records that
# fact in authorization.json beside its evidence, so no run of this loop can be mistaken for one
# prepared under the older guard.
#
# Each session generates its own credentials: a runtime credential the launcher hands to the game
# and the loop hands to the gateway, and a separate gateway credential. Nothing is read from a saved
# setting and no token has to be known in advance.
#
# Each episode writes its own directory under runs/, so a loop of N episodes produces N records.

set -uo pipefail

# Benchmark sessions retain their original fresh-process behavior. Streaming is an explicit
# operator mode: preserve the native game and pause on failure instead of discarding its run.
MODE="${1:-benchmark}"
if [ "$#" -gt 1 ] || [[ "$MODE" != benchmark && "$MODE" != stream ]]; then
    echo 'usage: jev-loop.sh [benchmark|stream]' >&2
    exit 2
fi

# The payload lives in the user's tree; this script lives in a root-owned directory so that the
# passwordless sudo rule cannot be pointed at something the desktop user is able to rewrite.
ROOT="${JEV_HOME:-/home/ubuntu/jev}"
WORK="${JEV_WORK_ROOT:-/home/ubuntu/sts2-native-map-g3-7d85w_n6}"
# The r2 bundle, not native-launch-inputs-steam-20260908: that older launcher pins the 09ef addon
# staging and refuses the addon actually installed now ("canonical addon identity mismatch"). This
# bundle is the operator's recorded production command for the current tree.
INPUTS="${JEV_BUNDLE:-$WORK/linux-rest-native-launch-ad927-r2-20260909}"
LAUNCHER="$INPUTS/launch-linux-native-rest-campaign.py"
LAUNCH_ARGS=()
if [ "$MODE" = stream ]; then
    LAUNCHER="$INPUTS/launch-linux-native-rest-stream.py"
    LAUNCH_ARGS=(--persistent-session)
fi
BRIDGE="$ROOT/sts2-jev-bridge"
TRANSPORT="$ROOT/systemone_transport.py"
HARNESS="$ROOT/sts2-harness-runtime"
GATEWAY="$ROOT/sts2-gateway-runtime"
MCP="$ROOT/sts2-mcp-server"
RUNS="$ROOT/runs"

# Digests the operator reviewed, taken verbatim from that bundle's operator-plan.json
# production_command and re-checked by the launcher against the files before every launch.
# The reviewed template was {"mod_settings":{"mods_enabled":false,"mod_list":[]}} and its digest
# was 58677fec...fec6. Two acknowledgement flags are added to it: without seen_ea_disclaimer the
# game shows its early-access notice on every launch, because the launcher creates a fresh profile
# each episode and the notice is acknowledged per profile. skip_intro_logo saves the logo each time.
# fullscreen is set here too, together with schema_version: without the version the game
# treats the template as a version-0 save, scavenges it, and falls back to default
# display settings, which is how a fullscreen flag here was silently dropped.
# fullscreen is set here too. The game takes its effective settings from this template,
# not from the profile copy: it rewrites live-campaign/settings.save from the mapped file
# before it applies display settings, so a fullscreen flag anywhere else is discarded.
# The digest below is the amended template; the operator plan still records the original.
SETTINGS_SHA="${JEV_SETTINGS_SHA:-4a0cd234aa5ab1095cc9a62b2cc5a0947a3ec0df7a7b236711597a4a32b5b967}"
MODLOAD_SHA="${JEV_MODLOAD_SHA:-a606d5f68634300f9c415b6e4abf76677bebcbaa23874c0243854e3a5208fe80}"

EPISODES="${JEV_EPISODES:-0}"          # 0 means keep going until this window is closed
# Gate 35 let Jev start a run and pick a map node, then refused every combat answer: in combat its
# calibrated confidence sits near 0.25, so it abstained and re-observed forever, asking the provider
# hundreds of times without acting. At 20 it plays: 20 dispatched actions in the first 46 decisions.
GATE_PERCENT="${JEV_GATE_PERCENT:-20}"
# 900s ended every episode mid-combat at exactly the 15 minute mark, exit 124, and the loop then
# tore the game down and launched a fresh one - which looks like the game finishing and restarting
# but is the bound firing. Measured episodes reached 117 to 123 decisions, five map nodes and real
# combat, and none of them ever saw a death or a victory. At about 7.7s per decision a run needs
# far longer than that, so the bound is now 90 minutes. The abstention and repeated-situation
# bounds still end a stuck run early; this one is only the backstop.
EPISODE_TIMEOUT="${JEV_EPISODE_TIMEOUT:-3300}"
# How long the launcher keeps the game session open. It must stay above EPISODE_TIMEOUT, or the
# game is killed out from under an episode that is still playing, and the launcher itself refuses
# anything outside 60..3600 ("max-seconds must be bounded between 60 and 3600"). 3600 is therefore
# the ceiling on a single run, and the episode bound sits just under it.
LAUNCH_SECONDS="${JEV_LAUNCH_SECONDS:-3600}"
LAUNCH_SECONDS_MAX=3600
GAME_READY_TIMEOUT="${JEV_GAME_READY_TIMEOUT:-240}"
WAIT_FOR_COMBAT="${JEV_WAIT_FOR_COMBAT:-180}"
# The runner waits on this barrier whenever an observation is not yet actionable, and the default
# of 8 polls x 1s expires while the game is still loading, failing the episode before the first
# decision. 40 x 3s gives the host two minutes to reach a screen Jev can act on.
BARRIER_MAX_POLLS=${JEV_BARRIER_MAX_POLLS:-40}
BARRIER_WAIT_MILLIS=${JEV_BARRIER_WAIT_MILLIS:-3000}
OBJECTIVE="${JEV_OBJECTIVE:-advance as far as possible in the run while preserving hit points}"

banner() {
    printf '\n\033[36m%s\033[0m\n' "$(printf '=%.0s' {1..78})"
    printf '\033[36m  %s\033[0m\n' "$1"
    printf '\033[36m%s\033[0m\n' "$(printf '=%.0s' {1..78})"
}

fail() { echo "  $1" >&2; }

credential() {
    # The launcher requires [A-Za-z0-9_-]{43,256}; this yields 64 characters from the OS CSPRNG.
    head -c 48 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n'
}

stop_stale() {
    for name in sts2-harness-runtime sts2-gateway-runtime sts2-mcp-server sts2-jev-bridge \
                SlayTheSpire2 launch-linux-standard-campaign; do
        pkill -f "$name" 2>/dev/null || true
    done
    sleep 2
}

wait_for_runtime() {
    local deadline=$(( $(date +%s) + GAME_READY_TIMEOUT ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if ss -tln 2>/dev/null | grep -q '127.0.0.1:15626'; then
            echo '  the mod runtime session is listening'
            return 0
        fi
        if ! kill -0 "$1" 2>/dev/null; then
            fail 'the launcher exited before the runtime session opened'
            return 1
        fi
        sleep 3
    done
    fail "runtime session never opened within ${GAME_READY_TIMEOUT}s"
    return 1
}

as_user() { sudo -u ubuntu "$@"; }

# The launcher expects the reviewed baseline profile and preserves it to a fixed path. Looping
# therefore needs two things per episode: the baseline back in place, and that fixed preservation
# path free. Both come from the launcher's own preservation, kept pristine here on first use.
PRESERVED="$WORK/rest-preservation-ad927-linux/profile"
PRESERVE_TARGET="$WORK/rest-preservation-ad927-linux/profile-before-rest-launch"
BASELINE="$ROOT/profile-baseline"

prepare_profile() {
    local run_dir="$1"
    if [ ! -d "$BASELINE" ]; then
        if [ -d "$PRESERVED" ]; then
            cp -a "$PRESERVED" "$BASELINE"
        elif [ -d "$PRESERVE_TARGET" ]; then
            cp -a "$PRESERVE_TARGET" "$BASELINE"
        else
            fail 'no reviewed baseline profile to restore from'
            return 1
        fi
        echo "  kept a pristine copy of the baseline profile at $BASELINE"
    fi
    # The launcher refuses to overwrite its preservation, so the previous one moves in with the
    # episode that produced it.
    if [ -e "$PRESERVE_TARGET" ]; then
        rm -rf "$run_dir/profile-preserved-by-launcher"
        mv "$PRESERVE_TARGET" "$run_dir/profile-preserved-by-launcher"
    fi
    rm -rf "$WORK/profile"
    cp -a "$BASELINE" "$WORK/profile"
    chown -R ubuntu:ubuntu "$WORK/profile"
    chmod 700 "$WORK/profile"
}

# The launcher compares the recorded authenticated session against the live one and refuses to
# run if any property differs, including the leader PID. gdm gives the desktop session a new
# leader every time it restarts, so a session record written once goes stale on the next boot and
# every episode then dies with "live loginctl properties differ from the authenticated session
# record". It is the same session either way, so re-record it here from the live one.
SESSION_USER="${JEV_SESSION_USER:-ubuntu}"

refresh_session_record() {
    local record="$INPUTS/session-env.json" sid uid name display remote type class state leader gid
    sid="$(loginctl list-sessions --no-legend | awk -v u="$SESSION_USER" '$3 == u { print $1; exit }')"
    if [ -z "$sid" ]; then
        echo "  no live login session for $SESSION_USER; leaving the recorded session alone" >&2
        return 0
    fi
    uid="$(loginctl show-session "$sid" -p User --value)"
    name="$(loginctl show-session "$sid" -p Name --value)"
    display="$(loginctl show-session "$sid" -p Display --value)"
    remote="$(loginctl show-session "$sid" -p Remote --value)"
    type="$(loginctl show-session "$sid" -p Type --value)"
    class="$(loginctl show-session "$sid" -p Class --value)"
    state="$(loginctl show-session "$sid" -p State --value)"
    leader="$(loginctl show-session "$sid" -p Leader --value)"
    gid="$(id -g "$name" 2>/dev/null)"
    for value in "$uid" "$name" "$type" "$class" "$state" "$leader" "$gid"; do
        if [ -z "$value" ]; then
            echo '  live session properties are incomplete; leaving the recorded session alone' >&2
            return 0
        fi
    done
    if [ "$remote" = 'no' ]; then remote=false; else remote=true; fi
    if [ ! -e "$record.first" ] && [ -e "$record" ]; then
        cp -p "$record" "$record.first"
    fi
    cat > "$record" <<JSON
{
  "schema": "sts2-linux-authenticated-session-env-v1",
  "user": {
    "uid": $uid,
    "gid": $gid,
    "name": "$name",
    "session_id": "$sid",
    "display": "$display",
    "remote": $remote,
    "type": "$type",
    "class": "$class",
    "state": "$state",
    "leader": $leader
  },
  "environment": {
    "XDG_SESSION_TYPE": "$type",
    "XDG_RUNTIME_DIR": "/run/user/$uid",
    "WAYLAND_DISPLAY": "wayland-0"
  }
}
JSON
    chown root:root "$record"
    chmod 644 "$record"
}

if [ "$LAUNCH_SECONDS" -gt "$LAUNCH_SECONDS_MAX" ]; then
    echo "JEV_LAUNCH_SECONDS ($LAUNCH_SECONDS) is above the launcher's own ceiling of" >&2
    echo "$LAUNCH_SECONDS_MAX; it would refuse with 'max-seconds must be bounded between 60 and 3600'." >&2
    exit 1
fi
if [ "$LAUNCH_SECONDS" -le "$EPISODE_TIMEOUT" ]; then
    echo "JEV_LAUNCH_SECONDS ($LAUNCH_SECONDS) must exceed JEV_EPISODE_TIMEOUT ($EPISODE_TIMEOUT)," >&2
    echo "or the launcher closes the game session while the episode is still playing." >&2
    exit 1
fi

[ "$(id -u)" -eq 0 ] || { echo 'this loop must run as root; start it from jev-start.sh' >&2; exit 1; }

# The launcher will not start Steam, and the game cannot authenticate without it: it reports
# "SteamAPI_Init(): did not locate a running instance of Steam" and never opens its runtime session.
# jev-start.sh brings Steam up in the desktop session before elevating to here.
if ! pgrep -u 1000 -x steam > /dev/null 2>&1; then
    echo 'Steam is not running as the desktop user; start it from jev-start.sh, not from here.' >&2
    echo 'Started from root it has no session keyring, cannot sign in, and exits.' >&2
    exit 1
fi
for required in "$LAUNCHER" "$BRIDGE" "$TRANSPORT" "$HARNESS" "$GATEWAY" "$MCP" "$ROOT/key.txt"; do
    [ -e "$required" ] || { echo "missing required component: $required" >&2; exit 1; }
done

DIGEST="$(sha256sum "$BRIDGE" | cut -d' ' -f1)"
exec 9<>/run/lock/jev-session.lock
flock -n 9 || { echo 'another Jev supervisor owns this session' >&2; exit 1; }
if [ "$MODE" = stream ]; then
    # Never close another game or attach an unverified native session.
    if pgrep -u 1000 -x SlayTheSpire2 >/dev/null 2>&1; then
        echo 'a game is already running; leave it open and stop before creating another stream' >&2
        exit 1
    fi
fi
banner "Jev plays Slay the Spire 2   |   $MODE   |   gate ${GATE_PERCENT}%"
echo "  bridge digest $DIGEST"
echo "  launcher      $LAUNCHER"

episode=0
failures=0
stream_started=false
stream_dir=''
while :; do
    episode=$((episode + 1))
    if [ "$EPISODES" -gt 0 ] && [ "$episode" -gt "$EPISODES" ]; then break; fi

    stamp="$(date +%Y%m%d-%H%M%S)"
    if [ "$MODE" = stream ]; then stamp="$stamp-$episode"; fi
    run_dir="$RUNS/episode-$stamp"
    mkdir -p "$run_dir/state"
    # The launcher refuses a readiness record whose parent is group/other accessible, and the
    # harness runs as the desktop user and keeps a durable branch store in its working directory.
    # Owner-only and owned by that user satisfies both; the runtime token inside stays root:root.
    chown -R ubuntu:ubuntu "$run_dir" 2>/dev/null
    chmod 700 "$ROOT" "$RUNS" "$run_dir" 2>/dev/null
    banner "episode $episode   |   $stamp"

    cat > "$run_dir/authorization.json" <<JSON
{
  "schema": "ascension.jev-loop-authorization.v1",
  "recorded_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "provider_calls": "authorized",
  "authorized_by": "repository owner, 2026-09-18",
  "note": "The operator plan this launcher was prepared under records a guard of 'no provider call'. This episode makes provider calls with the owner's explicit instruction, recorded here so the run is not mistaken for one prepared under that guard.",
  "provider": "typesafe", "model": "jev-latest", "confidence_gate_percent": $GATE_PERCENT
}
JSON

    if [ "$MODE" = benchmark ]; then stop_stale; fi
    if ! "$stream_started"; then
    if [ "$MODE" = stream ] && [ -d "$WORK/profile" ]; then
        # Preserve the previous stopped session before the reviewed fresh-profile launch.
        cp -a "$WORK/profile" "$run_dir/profile-before-stream" || break
    fi
    if ! prepare_profile "$run_dir"; then
        echo 'could not prepare the reviewed profile' > "$run_dir/outcome.txt"
        break
    fi

    runtime_token="$(credential)"
    printf '%s' "$runtime_token" > "$run_dir/runtime.token"
    chown root:root "$run_dir/runtime.token"
    chmod 600 "$run_dir/runtime.token"

    refresh_session_record
    echo '  starting the game through the supported launcher...'
    # DISPLAY is deliberately removed here. Steam needs it (it is an X11 client on XWayland) and
    # jev-start.sh leaves it in the environment, but the launcher refuses to run with an ambient
    # DISPLAY: "launcher refuses X11 fallback". The game is a Wayland client, so it loses nothing.
    env -u DISPLAY -u XAUTHORITY python3 "$LAUNCHER" \
        --session-env-json "$INPUTS/session-env.json" \
        --user-dir-mapping "$INPUTS/user-dir-mapping.json" \
        --settings-template "$INPUTS/settings-template.json" \
        --settings-sha256 "$SETTINGS_SHA" \
        --mod-loading-bin "$INPUTS/sts2-game-mod-loading" \
        --mod-loading-sha256 "$MODLOAD_SHA" \
        --runtime-token-file "$run_dir/runtime.token" \
        --readiness-file "$run_dir/readiness.json" \
        --max-seconds "$LAUNCH_SECONDS" \
        "${LAUNCH_ARGS[@]}" \
        --stop-file "$run_dir/stop" \
        > "$run_dir/launcher.log" 2>&1 &
    launcher_pid=$!

    if [ "$MODE" = stream ]; then
        stream_started=true
        stream_dir="$run_dir"
        printf '%s\n' "$stream_dir" > "$ROOT/stream-session.path"
        chown ubuntu:ubuntu "$ROOT/stream-session.path"
        chmod 600 "$ROOT/stream-session.path"
    fi

    if ! wait_for_runtime "$launcher_pid"; then
        echo 'runtime session never opened' > "$run_dir/outcome.txt"
        if [ "$MODE" = stream ]; then
            echo "  Stream readiness failed; game retained. Stop with: touch $stream_dir/stop"
            wait "$launcher_pid"
            break
        fi
        touch "$run_dir/stop"
        wait "$launcher_pid" 2>/dev/null
        if [ "$MODE" = benchmark ]; then stop_stale; fi
        echo "  record: $run_dir"
        failures=$((failures + 1))
        if [ "$failures" -ge 3 ]; then
            fail 'three launches failed in a row; stopping instead of spinning'
            fail "the launcher's own reason is in $run_dir/launcher.log"
            break
        fi
        sleep 5
        continue
    fi
    fi
    failures=0
    gateway_token="$(credential)"

    # A gateway left over from a previous episode keeps its allocation, and the harness then gets
    # "gateway returned HTTP 409" against a stale instance. Wait for the port to actually be free.
    for _ in $(seq 1 20); do
        ss -tln 2>/dev/null | grep -q '127.0.0.1:15525' || break
        if [ "$MODE" = benchmark ]; then
            pkill -f sts2-gateway-runtime 2>/dev/null || true
        fi
        sleep 1
    done

    # The gateway serves exactly one instance, defaulting to 'instance-1', while the harness asks
    # for the per-episode identity below. That mismatch is what the HTTP 409 was: the gateway held
    # instance-1 and refused the allocation for instance-jev-<stamp>. Both are now told the same one.
    # It also runs in its own directory, so the sqlite stores it opens belong to this episode alone
    # and no allocation state from a previous episode is found and reused.
    mkdir -p "$run_dir/gateway"
    chown ubuntu:ubuntu "$run_dir/gateway"
    echo '  starting the gateway...'
    # The scope is what the HTTP 401 was about: a gateway token carries no authority unless its
    # scope says so, and allocation needs all three. STS2_GATEWAY_TOKEN_EXPIRES_AT is not set,
    # because the reference fixture in tools/exact-restore-conformance omits it and an unverified
    # format here would only expire the token early. The identity below is the same one the harness
    # allocates with, including the caller id, which the harness defaults to "harness".
    ( cd "$run_dir/gateway" && exec sudo -u ubuntu env \
        STS2_GATEWAY_TOKEN="$gateway_token" \
        STS2_GATEWAY_TOKEN_SCOPE="read,mutate,control" \
        STS2_MOD_TOKEN="$runtime_token" \
        STS2_MOD_ADDR="127.0.0.1:15626" \
        STS2_CALLER_ID="harness" \
        STS2_INSTANCE_ID="instance-jev-$stamp" \
        STS2_SESSION_ID="session-jev-$stamp" \
        STS2_MCP_SESSION_ID="mcp-session-jev-$stamp" \
        STS2_LEASE_ID="lease-jev-$stamp" \
        STS2_LEASE_EPOCH="1" \
        STS2_DEPLOYMENT_ID="deployment-jev" \
        "$GATEWAY" ) > "$run_dir/gateway.out.log" 2> "$run_dir/gateway.err.log" &
    gateway_pid=$!
    sleep 5

    # Confirm it is serving this episode instance before the harness allocates against it.
    if grep -q "instance-jev-$stamp" "$run_dir/gateway.out.log" 2>/dev/null; then
        echo "  the gateway is serving instance-jev-$stamp"
    else
        echo "  note: the gateway did not report this episode instance; see gateway.out.log"
    fi

    # Campaign mode, not the combat demo: the demo only acts once the host is already in combat
    # and never leaves a menu, so against a freshly launched game it polls an unchanging main
    # menu and Jev is asked nothing. See AI-Ascension/sts2-harness#311.
    echo -e '  \033[32mhanding the run to Jev...\033[0m'
    HARNESS_COMMAND=(timeout "$EPISODE_TIMEOUT" "$HARNESS")
    if [ "$MODE" = stream ]; then HARNESS_COMMAND=("$HARNESS"); fi
    ( cd "$run_dir/state" && as_user env \
        STS2_GATEWAY_TOKEN="$gateway_token" \
        STS2_GATEWAY_TOKEN_SCOPE="read,mutate,control" \
        STS2_CALLER_ID="harness" \
        STS2_LEASE_EPOCH="1" \
        STS2_MOD_TOKEN="$runtime_token" \
        STS2_RUNTIME_PROFILE=runtime-v3-gameplay \
        STS2_MCP_BINARY="$MCP" \
        STS2_OBJECTIVE="$OBJECTIVE" \
        STS2_PROVIDER_KIND=typesafe-jev \
        STS2_EXO_ADMISSION=legacy \
        STS2_CAMPAIGN_EPISODE=true \
        STS2_BARRIER_MAX_POLLS="$BARRIER_MAX_POLLS" \
        STS2_BARRIER_WAIT_MILLIS="$BARRIER_WAIT_MILLIS" \
        STS2_EXO_BRIDGE_BINARY="$BRIDGE" \
        STS2_EXO_REVISION="$DIGEST" \
        STS2_EXO_BRIDGE_ARGS_JSON="[\"--model\",\"jev-latest\",\"--transport\",\"$TRANSPORT\",\"--gate\",\"$GATE_PERCENT\"]" \
        STS2_EXO_INHERITED_ENV_JSON='["TYPESAFE_API_KEY","JEV_CONTEXT_LOG"]' \
        JEV_CONTEXT_LOG="$run_dir/jev-context.jsonl" \
        TYPESAFE_API_KEY="$(cat "$ROOT/key.txt")" \
        STS2_RUN_ID="run-jev-$stamp" \
        STS2_EPISODE_ID="episode-jev-$stamp" \
        STS2_TRAJECTORY_ID="trajectory-jev-$stamp" \
        STS2_TRACE_ID="trace-jev-$stamp" \
        STS2_ARTIFACT_ID="artifact-jev-$stamp" \
        STS2_INSTANCE_ID="instance-jev-$stamp" \
        STS2_SESSION_ID="session-jev-$stamp" \
        STS2_MCP_SESSION_ID="mcp-session-jev-$stamp" \
        STS2_LEASE_ID="lease-jev-$stamp" \
        STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS="$WAIT_FOR_COMBAT" \
        "${HARNESS_COMMAND[@]}" ) \
            > "$run_dir/harness.out.log" 2> "$run_dir/harness.err.log"
    status=$?
    echo "exit $status" > "$run_dir/outcome.txt"
    if [ "$status" -eq 124 ]; then
        echo "  episode passed its ${EPISODE_TIMEOUT}s bound"
    else
        echo "  episode finished, harness exit $status"
    fi

    kill "$gateway_pid" 2>/dev/null || true
    if [ "$MODE" = stream ]; then
        wait "$gateway_pid" 2>/dev/null || true
        echo '  Streaming paused; the game and profile remain open.'
        echo '  No automatic retry and no application restart.'
        echo "  After resolving the stop (or returning to the main menu), explicitly resume with:"
        echo "  touch $stream_dir/resume"
        echo "  To close this owned game session: touch $stream_dir/stop"
        printf '%s\n' "paused; harness_exit=$status" > "$stream_dir/stream-status.txt"
        # The installed host has no terminal-screen transition in its action catalog. Do not
        # fabricate one, inject UI input, reset the profile, or spin new terminal episodes.
        while kill -0 "$launcher_pid" 2>/dev/null; do
            if [ -f "$stream_dir/resume" ]; then
                rm -- "$stream_dir/resume"
                printf '%s\n' 'resumed by operator' > "$stream_dir/stream-status.txt"
                break
            fi
            sleep 2
        done
        if ! kill -0 "$launcher_pid" 2>/dev/null; then
            wait "$launcher_pid" 2>/dev/null || true
            break
        fi
        continue
    fi
    touch "$run_dir/stop"
    wait "$launcher_pid" 2>/dev/null
    stop_stale
    echo "  record: $run_dir"
done

banner 'loop finished'
read -r -p 'press enter to close' _
