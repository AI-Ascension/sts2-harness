# Jev plays Slay the Spire 2 on Windows, one episode at a time, restarting the session after each.
#
# This is the Windows counterpart of jev-loop.sh and is deliberately kept in step with it: the same
# credentials per session, the same gateway and harness identity, the same bounds. Where the two
# differ, it is because the platforms differ, and the comment says why.
#
# Every session generates its own credentials from the operating system CSPRNG: a runtime credential
# the game and the gateway share, and a separate gateway credential the gateway, the MCP server and
# the harness share. Nothing is read from a saved setting and no token has to be known in advance.
#
# The provider credential is read from key.txt beside this script and reaches only the transport the
# bridge spawns. It is never an argument and never logged.
#
# AUTHORIZATION: these episodes make provider calls. The owner asked for a model-driven loop on this
# lane and reaffirmed it, and every episode records that in authorization.json beside its evidence.

[CmdletBinding()]
param(
    # 0 means keep going until the window is closed.
    [int]$Episodes = 0,

    # Confidence at or above which the bridge acts, as an integer percentage. Measured on the Linux
    # runs: at 35 Jev starts a run and picks a map node but refuses every combat answer, because its
    # calibrated confidence in combat sits near 0.25 and it abstains and re-observes instead. At 20
    # it plays.
    [ValidateRange(0, 100)][int]$GatePercent = 20,

    [int]$EpisodeTimeoutMinutes = 10,

    # How long the mod has to open its runtime session before the episode is abandoned. The default
    # 8 x 1s barrier expires while the game is still loading, so this is generous by comparison.
    [int]$GameReadySeconds = 240,

    # Carried into the choice question the bridge asks, so it is part of what Jev is told.
    [string]$Objective = 'advance as far as possible in the run while preserving hit points',

    [int]$Width = 1280,
    [int]$Height = 800,
    [switch]$Windowed
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$deployment = 'C:/AIAscension/acceptance/seeded79-20260916'
$hostDir = Join-Path $deployment 'host'
$peers = Join-Path $deployment 'addon/peers'
$baselineProfile = Join-Path $deployment 'profile'
$runs = Join-Path $root 'runs'

$bridge = Join-Path $root 'sts2-jev-bridge.exe'
$transport = Join-Path $root 'systemone_transport.cmd'
$harness = Join-Path $root 'sts2-harness-runtime.exe'
if (-not (Test-Path $harness)) { $harness = Join-Path $peers 'sts2-harness-runtime.exe' }
$mcp = Join-Path $peers 'sts2-mcp-server.exe'
$gateway = Join-Path $peers 'sts2-gateway-runtime.exe'
$game = Join-Path $hostDir 'SlayTheSpire2.exe'
$keyFile = Join-Path $root 'key.txt'

# The mod opens its runtime session on the port it is told to use. Both are probed because a port
# is only the mod's if the game process owns it, and checking both costs nothing.
$modPortCandidates = @(15626, 15627)

# An episode that fails in seconds and is restarted at once is a crash loop, not a loop.
$maxConsecutiveFailures = 3

function Write-Banner {
    param([string]$Text)
    Write-Host ''
    Write-Host ('=' * 78) -ForegroundColor Cyan
    Write-Host "  $Text" -ForegroundColor Cyan
    Write-Host ('=' * 78) -ForegroundColor Cyan
}

function New-Credential {
    <#
      The launcher and the gateway require [A-Za-z0-9_-]{43,256}. This yields 64 characters from the
      operating system CSPRNG. RandomNumberGenerator::Fill is .NET Core only, so the provider is
      created explicitly for Windows PowerShell.
    #>
    $bytes = New-Object byte[] 48
    $rng = New-Object System.Security.Cryptography.RNGCryptoServiceProvider
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    [Convert]::ToBase64String($bytes).Replace('+', '-').Replace('/', '_').TrimEnd('=')
}

function Stop-Stale {
    <#
      taskkill writes to stderr when nothing matches, which is an error under this script's
      preference, so each call is contained. Nothing here is fatal: a process that is already gone
      is the outcome being asked for.
    #>
    foreach ($name in @('SlayTheSpire2.exe', 'sts2-harness-runtime.exe', 'sts2-gateway-runtime.exe',
            'sts2-mcp-server.exe', 'sts2-jev-bridge.exe')) {
        try { & taskkill.exe /F /IM $name 2>$null | Out-Null } catch { }
    }
    Start-Sleep -Seconds 2
}

function Stop-Tree {
    param([System.Diagnostics.Process[]]$Processes)
    foreach ($proc in $Processes) {
        if ($null -eq $proc) { continue }
        try { if (-not $proc.HasExited) { $proc.Kill() } } catch { }
    }
}

function Initialize-IsolatedUserDir {
    <#
      The mod refuses to initialise unless the game's user directory is isolated and already holds a
      profile baseline it considers fresh. Its own diagnostics name the cases: "isolated user
      directory is unavailable", "isolated user directory has no profile files",
      "profile_baseline_not_fresh", "profile_baseline_unavailable". Linux satisfies this by copying a
      pristine baseline into the game's user directory before every episode; this is the same thing.

      The directory also has to answer the questions the game would otherwise stop to ask -- the
      mods warning, the early-access notice, the intro logo -- because it is thrown away after each
      episode and so has never been asked them.
    #>
    param([string]$UserDirName)

    $target = Join-Path $env:APPDATA $UserDirName
    New-Item -ItemType Directory -Force -Path $target | Out-Null

    if (Test-Path $baselineProfile) {
        Copy-Item (Join-Path $baselineProfile '*') -Destination $target -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host '  copied the profile baseline into the episode directory' -ForegroundColor Gray
    } else {
        Write-Host "  no profile baseline at $baselineProfile; the mod will refuse to initialise" -ForegroundColor Yellow
    }

    # The Steam account directory is named after the signed-in account, so it is learned from a
    # directory an earlier episode created rather than hard-coded.
    $account = Get-ChildItem $env:APPDATA -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'AIAscensionJevLoop-*' -and $_.Name -ne $UserDirName } |
        Sort-Object LastWriteTime -Descending |
        ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'steam') -Directory -ErrorAction SilentlyContinue } |
        Select-Object -First 1
    if (-not $account) {
        Write-Host '  no earlier profile to learn the Steam account from; the game will ask once' -ForegroundColor Yellow
        return
    }

    $accountDir = Join-Path (Join-Path $target 'steam') $account.Name
    New-Item -ItemType Directory -Force -Path $accountDir | Out-Null
    $settingsPath = Join-Path $accountDir 'settings.save'
    try {
        $settings = if (Test-Path $settingsPath) {
            Get-Content $settingsPath -Raw | ConvertFrom-Json
        } else {
            [pscustomobject]@{}
        }
        $settings | Add-Member -NotePropertyName 'seen_ea_disclaimer' -NotePropertyValue $true -Force
        $settings | Add-Member -NotePropertyName 'skip_intro_logo' -NotePropertyValue $true -Force
        # The game writes mod_settings as null and then skips the mod, logging that the user has not
        # seen its mods warning. The configured block is what satisfies that.
        $mods = [pscustomobject]@{
            mods_enabled = $true
            mod_list     = @([pscustomobject]@{
                    id         = 'AIAscensionSTS2GameMod'
                    is_enabled = $true
                    source     = 'mods_directory'
                })
        }
        $settings | Add-Member -NotePropertyName 'mod_settings' -NotePropertyValue $mods -Force
        $settings | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $settingsPath -Encoding UTF8
        Write-Host "  answered the game's one-time prompts for $($account.Name)" -ForegroundColor Gray
    } catch {
        Write-Host "  could not seed the profile: $($_.Exception.Message)" -ForegroundColor Yellow
    }
}

function Wait-ForModRuntime {
    <#
      Waits for the mod's runtime session, and requires the game process to own the port.

      An earlier version accepted any listener on a candidate port and matched 15627, which on this
      machine is held by svchost: the harness was then pointed at a Windows service and every episode
      failed with observe_failed. A listener is only the mod's if the game opened it.
    #>
    param([System.Diagnostics.Process]$GameProcess, [int]$Seconds)

    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        foreach ($candidate in $modPortCandidates) {
            $listener = Get-NetTCPConnection -State Listen -LocalPort $candidate -ErrorAction SilentlyContinue |
                Where-Object { $_.OwningProcess -eq $GameProcess.Id } |
                Select-Object -First 1
            if ($listener) { return $candidate }
        }
        if ($GameProcess.HasExited) {
            throw 'the mod runtime session never opened: the game exited. See game.log in the run directory.'
        }
        Start-Sleep -Seconds 3
    }
    throw "the mod runtime session never opened within $Seconds seconds. See game.log in the run directory."
}

foreach ($required in @($bridge, $transport, $harness, $mcp, $gateway, $game, $keyFile)) {
    if (-not (Test-Path $required)) { throw "missing required component: $required" }
}
New-Item -ItemType Directory -Force -Path $runs | Out-Null

$apiKey = (Get-Content $keyFile -Raw).Trim()
$digest = (Get-FileHash $bridge -Algorithm SHA256).Hash.ToLowerInvariant()

Write-Banner "Jev plays Slay the Spire 2   |   gate $GatePercent%   |   $Width x $Height"
Write-Host "  bridge digest $digest"
Write-Host "  deployment    $deployment"

$episode = 0
$consecutiveFailures = 0

while ($true) {
    $episode++
    if ($Episodes -gt 0 -and $episode -gt $Episodes) { break }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $runDir = Join-Path $runs "episode-$stamp"
    $stateDir = Join-Path $runDir 'state'
    $gatewayDir = Join-Path $runDir 'gateway'
    New-Item -ItemType Directory -Force -Path $stateDir, $gatewayDir | Out-Null

    Write-Banner "episode $episode   |   $stamp"

    @{
        schema                  = 'ascension.jev-loop-authorization.v1'
        recorded_utc            = (Get-Date).ToUniversalTime().ToString('o')
        provider_calls          = 'authorized'
        authorized_by           = 'repository owner'
        provider                = 'typesafe'
        model                   = 'jev-latest'
        confidence_gate_percent = $GatePercent
    } | ConvertTo-Json | Set-Content (Join-Path $runDir 'authorization.json') -Encoding UTF8

    Stop-Stale

    # A disposable user directory per episode, which is what the mod requires, seeded before the
    # game reads it.
    $userDir = "AIAscensionJevLoop-$stamp"
    @"
[application]
config/use_custom_user_dir=true
config/custom_user_dir_name="$userDir"
"@ | Set-Content (Join-Path $hostDir 'override.cfg') -Encoding UTF8

    $runtimeToken = New-Credential
    $gatewayToken = New-Credential

    $gameProc = $null
    $gatewayProc = $null
    $harnessProc = $null
    try {
        Initialize-IsolatedUserDir -UserDirName $userDir

        # --- the game, with the mod's runtime session listening on loopback -------------------
        $env:STS2_RUNTIME_SESSION = '1'
        $env:STS2_RUNTIME_BIND_ADDRESS = '127.0.0.1'
        $env:STS2_RUNTIME_PORT = "$($modPortCandidates[0])"
        $env:STS2_RUNTIME_TOKEN = $runtimeToken

        $gameArgs = @(
            '--max-fps', '60',
            '--resolution', "${Width}x${Height}",
            '--disable-vsync',
            '--rendering-driver', 'd3d12',
            '--audio-driver', 'Dummy',
            '--log-file', (Join-Path $runDir 'game.log')
        )
        if (-not $Windowed) { $gameArgs += '--fullscreen' }

        Write-Host '  starting the game...' -ForegroundColor Gray
        $gameProc = Start-Process $game -WorkingDirectory $hostDir -ArgumentList $gameArgs -PassThru
        $gameProc.Id | Set-Content (Join-Path $runDir 'game.pid')

        $modPort = Wait-ForModRuntime -GameProcess $gameProc -Seconds $GameReadySeconds
        Write-Host "  the mod runtime session is listening on $modPort" -ForegroundColor Gray

        # --- the gateway, holding both credentials --------------------------------------------
        # A gateway token carries no authority unless its scope says so, or allocation is answered
        # with HTTP 401. The gateway also serves exactly one instance and defaults to "instance-1",
        # so it is told the same per-episode identity the harness allocates with, or the allocation
        # is answered with HTTP 409 instead.
        $env:STS2_GATEWAY_TOKEN = $gatewayToken
        $env:STS2_GATEWAY_TOKEN_SCOPE = 'read,mutate,control'
        $env:STS2_MOD_TOKEN = $runtimeToken
        $env:STS2_MOD_ADDR = "127.0.0.1:$modPort"
        $env:STS2_CALLER_ID = 'harness'
        $env:STS2_INSTANCE_ID = "instance-jev-$stamp"
        $env:STS2_SESSION_ID = "session-jev-$stamp"
        $env:STS2_MCP_SESSION_ID = "mcp-session-jev-$stamp"
        $env:STS2_LEASE_ID = "lease-jev-$stamp"
        $env:STS2_LEASE_EPOCH = '1'
        $env:STS2_DEPLOYMENT_ID = 'deployment-jev'

        Write-Host '  starting the gateway...' -ForegroundColor Gray
        $gatewayProc = Start-Process $gateway -WorkingDirectory $gatewayDir -PassThru `
            -RedirectStandardOutput (Join-Path $runDir 'gateway.out.log') `
            -RedirectStandardError (Join-Path $runDir 'gateway.err.log')
        Start-Sleep -Seconds 5

        # --- the harness, driving the episode through Jev -------------------------------------
        $env:STS2_RUNTIME_PROFILE = 'runtime-v3-gameplay'
        $env:STS2_MCP_BINARY = $mcp
        $env:STS2_OBJECTIVE = $Objective
        $env:STS2_PROVIDER_KIND = 'typesafe-jev'
        $env:STS2_EXO_ADMISSION = 'legacy'
        # Campaign mode, not the combat demo. The demo only acts once the host is already in combat
        # and never leaves a menu, so against a freshly launched game it polls an unchanging main
        # menu and Jev is asked nothing.
        $env:STS2_CAMPAIGN_EPISODE = 'true'
        # The runner waits on this barrier whenever an observation is not yet actionable, and the
        # default of 8 polls x 1s expires while the game is still loading.
        $env:STS2_BARRIER_MAX_POLLS = '40'
        $env:STS2_BARRIER_WAIT_MILLIS = '3000'
        $env:STS2_EXO_BRIDGE_BINARY = $bridge
        $env:STS2_EXO_REVISION = $digest
        $env:STS2_EXO_BRIDGE_ARGS_JSON =
            "[""--model"",""jev-latest"",""--transport"",""$($transport -replace '\\', '\\')"",""--gate"",""$GatePercent""]"
        # Written literally: ConvertTo-Json unwraps a single-element array into a bare string, which
        # the runtime rejects as not being a JSON array.
        $env:STS2_EXO_INHERITED_ENV_JSON = '["TYPESAFE_API_KEY","JEV_CONTEXT_LOG"]'
        $env:TYPESAFE_API_KEY = $apiKey
        # Every exchange with the provider is recorded here: the state, the instructions, every
        # option, and what came back. Read it with jev-context.py.
        $env:JEV_CONTEXT_LOG = Join-Path $runDir 'jev-context.jsonl'
        $env:STS2_RUN_ID = "run-jev-$stamp"
        $env:STS2_EPISODE_ID = "episode-jev-$stamp"
        $env:STS2_TRAJECTORY_ID = "trajectory-jev-$stamp"
        $env:STS2_TRACE_ID = "trace-jev-$stamp"
        $env:STS2_ARTIFACT_ID = "artifact-jev-$stamp"

        Write-Host '  handing the run to Jev...' -ForegroundColor Green
        $harnessProc = Start-Process $harness -WorkingDirectory $stateDir -PassThru `
            -RedirectStandardOutput (Join-Path $runDir 'harness.out.log') `
            -RedirectStandardError (Join-Path $runDir 'harness.err.log')

        if (-not $harnessProc.WaitForExit($EpisodeTimeoutMinutes * 60 * 1000)) {
            Write-Host "  episode passed its $EpisodeTimeoutMinutes minute bound" -ForegroundColor Yellow
            'timeout' | Set-Content (Join-Path $runDir 'outcome.txt')
            $consecutiveFailures++
        } else {
            $code = $harnessProc.ExitCode
            "exit $code" | Set-Content (Join-Path $runDir 'outcome.txt')
            Write-Host "  episode finished, harness exit $code" -ForegroundColor Gray
            if ($code -eq 0) { $consecutiveFailures = 0 } else { $consecutiveFailures++ }
        }
    } catch {
        $_.Exception.Message | Set-Content (Join-Path $runDir 'error.txt')
        Write-Host "  episode failed: $($_.Exception.Message)" -ForegroundColor Red
        $consecutiveFailures++
    } finally {
        Stop-Tree @($harnessProc, $gatewayProc, $gameProc)
        Stop-Stale
        foreach ($name in @('STS2_RUNTIME_TOKEN', 'STS2_MOD_TOKEN', 'STS2_GATEWAY_TOKEN',
                'TYPESAFE_API_KEY', 'JEV_CONTEXT_LOG')) {
            Remove-Item "Env:\$name" -ErrorAction SilentlyContinue
        }
        Write-Host "  record: $runDir" -ForegroundColor DarkGray
    }

    if ($consecutiveFailures -ge $maxConsecutiveFailures) {
        Write-Host ''
        Write-Host "  $consecutiveFailures episodes failed in a row; stopping rather than restarting again." -ForegroundColor Yellow
        Write-Host "  The last one is recorded in $runDir - read harness.err.log and game.log there." -ForegroundColor Yellow
        break
    }
}

Write-Banner 'loop finished'
