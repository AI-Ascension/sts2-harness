# Reads one System One request body on stdin, posts it, writes the response body on stdout.
#
# This is the operator-owned transport the bridge spawns. It performs exactly one exchange and
# knows nothing about the action catalog or the decision. The credential comes from the
# environment and is never logged, echoed, or written to stdout. It uses the platform TLS stack,
# so it adds no dependency to the harness workspace.
#
# It also records the exchange when JEV_CONTEXT_LOG names a file: the whole context Jev was given,
# state, instructions and every option, with what came back and how much of the budget it used.
# Recording is best effort and wrapped, so a failure to record can never fail an exchange.
#
# A repeated state is stored once. The same state is re-sent while the model abstains, so storing
# every copy would grow the file without adding anything; later records carry the digest alone,
# which also makes the repetition visible rather than hiding it in duplicate text.

$ErrorActionPreference = 'Stop'

# The builder's own conservative ceiling for the state plus the longest question, kept in step with
# MAX_STATE_AND_QUESTION_BYTES in crates/harness/src/context_control/systemone_request.rs.
$script:BudgetBytes = 64 * 1024

# The record is a JSON Lines stream, and PowerShell's [Text.Encoding]::UTF8 emits a byte-order mark
# when it creates a file, which puts three bytes in front of the first record and makes that line
# unparseable for a reader that does not expect them. UTF8Encoding($false) writes the same bytes as
# UTF-8 without the mark, so the file starts with `{` the way every reader of JSON Lines expects.
# The GetBytes calls elsewhere in this script are unaffected: they never write a preamble.
$script:MarkFreeUtf8 = New-Object System.Text.UTF8Encoding($false)

function Get-Sha256Hex([string]$text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))
        return -join ($bytes | ForEach-Object { $_.ToString('x2') })
    } finally {
        $sha.Dispose()
    }
}

function Write-ContextRecord([string]$requestBody, [string]$responseBody, $status) {
    $path = $env:JEV_CONTEXT_LOG
    if ([string]::IsNullOrWhiteSpace($path)) { return }
    try {
        $request = $requestBody | ConvertFrom-Json
        $state = [string]$request.state
        $stateSha = Get-Sha256Hex $state
        $longest = 0
        $options = @()
        $instructions = $null
        $questionNames = @()
        if ($request.questions) {
            foreach ($q in $request.questions.PSObject.Properties) {
                $questionNames += $q.Name
                $encoded = ($q.Value | ConvertTo-Json -Depth 20 -Compress)
                if ($encoded.Length -gt $longest) { $longest = $encoded.Length }
            }
            $action = $request.questions.action
            if ($action) {
                $instructions = [string]$action.instructions
                if ($action.criteria) {
                    $options = @($action.criteria.PSObject.Properties | ForEach-Object { $_.Name })
                }
            }
        }
        $used = $state.Length + $longest

        # Full state the first time this exact state is seen; by digest afterwards.
        $seen = $false
        if (Test-Path -LiteralPath $path) {
            foreach ($line in [IO.File]::ReadLines($path)) {
                if ($line -and $line.Contains($stateSha)) { $seen = $true; break }
            }
        }

        $entry = [ordered]@{
            recorded_utc           = (Get-Date).ToUniversalTime().ToString('o')
            model                  = [string]$request.model
            state_sha256           = $stateSha
            state_bytes            = $state.Length
            question_names         = @($questionNames | Sort-Object)
            instructions           = $instructions
            # The criteria map, not just its keys: the description is what the model reads to
            # tell one option from another.
            criteria               = $criteria
            options                = $options
            option_count           = $options.Count
            longest_question_bytes = $longest
            budget                 = [ordered]@{
                ceiling_bytes  = $script:BudgetBytes
                used_bytes     = $used
                headroom_bytes = $script:BudgetBytes - $used
                used_percent   = [math]::Round(100.0 * $used / $script:BudgetBytes, 2)
            }
            http_status            = $status
        }
        if ($seen) { $entry['state_repeat_of'] = $stateSha } else { $entry['state'] = $state }
        if (-not [string]::IsNullOrWhiteSpace($responseBody)) {
            try { $entry['response'] = ($responseBody | ConvertFrom-Json) }
            catch { $entry['response_unparsed'] = $responseBody.Substring(0, [Math]::Min(4000, $responseBody.Length)) }
        }
        $line = ($entry | ConvertTo-Json -Depth 30 -Compress)
        [IO.File]::AppendAllText($path, $line + "`n", $script:MarkFreeUtf8)
    } catch {
        # Recording must never fail the exchange.
    }
}

$key = $env:TYPESAFE_API_KEY
if ([string]::IsNullOrWhiteSpace($key)) {
    [Console]::Error.WriteLine('TYPESAFE_API_KEY is not set')
    exit 2
}

$body = [Console]::In.ReadToEnd()

try {
    $response = Invoke-WebRequest -Uri 'https://api.typesafe.ai/v1/systemone' `
        -Method Post `
        -Body ([Text.Encoding]::UTF8.GetBytes($body)) `
        -ContentType 'application/json' `
        -Headers @{ Authorization = "Bearer $key" } `
        -UseBasicParsing `
        -TimeoutSec 60
    Write-ContextRecord $body $response.Content ([int]$response.StatusCode)
    [Console]::Out.Write($response.Content)
    exit 0
} catch {
    # Status and message go to stderr so a failure is diagnosable without polluting stdout,
    # which the bridge parses. The credential is not part of either.
    $status = $null
    $detail = $null
    if ($_.Exception.Response) {
        $status = [int]$_.Exception.Response.StatusCode
        try {
            $reader = [IO.StreamReader]::new($_.Exception.Response.GetResponseStream())
            $detail = $reader.ReadToEnd()
            $reader.Dispose()
        } catch { $detail = $null }
    }
    Write-ContextRecord $body $detail $status
    if ($status) { [Console]::Error.WriteLine("provider returned HTTP $status") }
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
