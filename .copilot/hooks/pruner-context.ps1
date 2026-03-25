# Copilot userPromptSubmitted hook: runs pruner context and stores output in .pruner/copilot-context.md

$ErrorActionPreference = "Stop"
$inputJson = [Console]::In.ReadToEnd() | ConvertFrom-Json

$prompt = $inputJson.prompt
if ([string]::IsNullOrWhiteSpace($prompt)) {
    exit 0
}

$root = $inputJson.cwd
if ([string]::IsNullOrWhiteSpace($root) -or -not (Test-Path $root)) {
    $root = "."
}

# Only run if this looks like a code repo (has .git or .pruner already).
# Avoids creating .pruner/ in random directories like ~ or ~/Downloads.
if (-not (Test-Path (Join-Path $root ".git")) -and -not (Test-Path (Join-Path $root ".pruner"))) {
    exit 0
}

$pruner = (Get-Command pruner -ErrorAction SilentlyContinue).Source
if ([string]::IsNullOrWhiteSpace($pruner)) {
    $candidates = @(
        (Join-Path $env:USERPROFILE ".local\bin\pruner.exe"),
        (Join-Path $env:USERPROFILE ".cargo\bin\pruner.exe"),
        (Join-Path $root "target\release\pruner.exe")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            $pruner = $candidate
            break
        }
    }
}
if ([string]::IsNullOrWhiteSpace($pruner) -or -not (Test-Path $pruner)) {
    exit 0
}

$prunerDir = Join-Path $root ".pruner"
if (-not (Test-Path $prunerDir)) {
    New-Item -ItemType Directory -Path $prunerDir | Out-Null
}

try {
    $output = & $pruner context $root $prompt 2>$null
} catch {
    exit 0
}

if ([string]::IsNullOrWhiteSpace($output)) {
    exit 0
}

$outFile = Join-Path $prunerDir "copilot-context.md"
$content = @(
    "# Pruner context (pre-computed codebase analysis)"
    ""
    $output
    ""
    "Use this context to work directly. Only read source files if a snippet is truncated."
    "Do not re-explore with grep/glob for the same keywords."
) -join "`n"

Set-Content -Path $outFile -Value $content -NoNewline
exit 0
