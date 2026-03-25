# Pruner installer for Windows
#
# Interactive:
#   irm https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.ps1 | iex
#
# Non-interactive (flags skip the prompts):
#   $env:PRUNER_HOOK = "1"; $env:PRUNER_GLOBAL = "1"; irm ... | iex
#
# Environment variables for non-interactive use:
#   PRUNER_VERSION      Specific version to install (default: latest)
#   PRUNER_INSTALL_DIR  Install directory (default: ~/.local/bin)
#   PRUNER_HOOK         Set to "1" to install Claude Code hook
#   PRUNER_GLOBAL       Set to "1" to install globally (~/.claude/)
#   PRUNER_COPILOT_SKILL  Set to "1" for Copilot CLI skill
#   PRUNER_COPILOT_GLOBAL Set to "1" for global Copilot skill
#   PRUNER_NO_INTERACTIVE Set to "1" to skip prompts

$ErrorActionPreference = "Stop"

$Repo = "heikki-laitala/pruner"
$Version = if ($env:PRUNER_VERSION) { $env:PRUNER_VERSION } else { "latest" }
$InstallDir = if ($env:PRUNER_INSTALL_DIR) { $env:PRUNER_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }
$Hook = $env:PRUNER_HOOK -eq "1"
$Global = $env:PRUNER_GLOBAL -eq "1"
$CopilotSkill = $env:PRUNER_COPILOT_SKILL -eq "1"
$CopilotGlobal = $env:PRUNER_COPILOT_GLOBAL -eq "1"
$NoInteractive = $env:PRUNER_NO_INTERACTIVE -eq "1"
$HasSetupFlags = $Hook -or $Global -or $CopilotSkill -or $CopilotGlobal

# Resolve version
if (-not $Version -or $Version -eq "latest") {
    Write-Host "Fetching latest release..."
    $Release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $Release.tag_name
} else {
    Write-Host "Using version $Version"
}

$BinaryName = "pruner-windows-x86_64"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$BinaryName.zip"

Write-Host "Installing pruner $Version (windows/x86_64)..."
Write-Host "Downloading $DownloadUrl..."

# Download to temp
$TempDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "pruner-install-$(Get-Random)")
$ZipPath = Join-Path $TempDir "$BinaryName.zip"

try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
} catch {
    Write-Error "Download failed. Check https://github.com/$Repo/releases"
    Remove-Item -Recurse -Force $TempDir
    exit 1
}

# Extract
Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force

# Install
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$ExePath = Join-Path $InstallDir "pruner.exe"
Copy-Item (Join-Path $TempDir "$BinaryName.exe") $ExePath -Force
Write-Host "Installed binary -> $ExePath"

# Cleanup
Remove-Item -Recurse -Force $TempDir

# Check PATH
if ($env:PATH -notlike "*$InstallDir*") {
    Write-Host ""
    Write-Warning "$InstallDir is not in your PATH."
    Write-Host "Add it with:"
    Write-Host ""
    Write-Host "  `$env:PATH = `"$InstallDir;`$env:PATH`""
    Write-Host ""
    Write-Host "Or permanently via System Properties > Environment Variables."
    Write-Host ""
}

# Verify
try {
    & $ExePath --version
} catch {
    Write-Host "pruner installed but not in PATH yet."
}

# -- Interactive setup (when no flags provided) --

if (-not $HasSetupFlags -and -not $NoInteractive) {
    Write-Host ""
    Write-Host "How do you want to set up pruner?"
    Write-Host ""
    Write-Host "  1) Claude Code  (global - works in every repo)"
    Write-Host "  2) Copilot CLI  (global - works in every repo)"
    Write-Host "  3) Both Claude Code + Copilot CLI (global)"
    Write-Host "  4) Skip - I'll set up later with 'pruner init'"
    Write-Host ""
    $Choice = Read-Host "Choice [1]"
    if ([string]::IsNullOrWhiteSpace($Choice)) { $Choice = "1" }

    switch ($Choice) {
        "1" {
            $Global = $true
            Write-Host ""
            Write-Host "Claude Code mode:"
            Write-Host "  1) Hook - context injected automatically (best performance)"
            Write-Host "  2) Skill - Claude calls pruner as a tool"
            Write-Host ""
            $Mode = Read-Host "Choice [1]"
            if ([string]::IsNullOrWhiteSpace($Mode)) { $Mode = "1" }
            if ($Mode -eq "1") { $Hook = $true }
        }
        "2" {
            $CopilotGlobal = $true
            $CopilotSkill = $true
        }
        "3" {
            $Global = $true
            $CopilotGlobal = $true
            $CopilotSkill = $true
            Write-Host ""
            Write-Host "Claude Code mode:"
            Write-Host "  1) Hook - context injected automatically (best performance)"
            Write-Host "  2) Skill - Claude calls pruner as a tool"
            Write-Host ""
            $Mode = Read-Host "Choice [1]"
            if ([string]::IsNullOrWhiteSpace($Mode)) { $Mode = "1" }
            if ($Mode -eq "1") { $Hook = $true }
        }
        "4" {
            Write-Host ""
            Write-Host "To set up later:"
            Write-Host "  pruner init --global --hook          # Claude Code (recommended)"
            Write-Host "  pruner init --copilot-skill --copilot-global  # Copilot CLI"
            Write-Host "  pruner init C:\path\to\project --hook  # per-project"
            Write-Host ""
            Write-Host "Done."
            exit 0
        }
        default {
            Write-Host "Invalid choice. Skipping setup."
            Write-Host ""
            Write-Host "Done."
            exit 0
        }
    }
}

# -- Run pruner init --

Write-Host ""
$InitArgs = @()
if ($Hook) { $InitArgs += "--hook" }
if ($Global) { $InitArgs += "--global" }
if ($CopilotSkill) { $InitArgs += "--copilot-skill" }
if ($CopilotGlobal) { $InitArgs += "--copilot-global" }

if ($Global -or $CopilotGlobal) {
    Write-Host "Setting up global integration..."
    & $ExePath init @InitArgs
    Write-Host ""
    Write-Host "Pruner is ready. It will auto-index each repo on first use."
    Write-Host "Add .pruner/ to your .gitignore (global install doesn't modify it):"
    Write-Host ""
    Write-Host "  echo '.pruner/' >> .gitignore"
} elseif ($HasSetupFlags) {
    Write-Host "To set up pruner in a project:"
    Write-Host ""
    if ($Hook) {
        Write-Host "  pruner init C:\path\to\project --hook   # Claude Code (best performance)"
    } else {
        Write-Host "  pruner init C:\path\to\project          # skill mode (works everywhere)"
        Write-Host "  pruner init C:\path\to\project --hook   # Claude Code (best performance)"
    }
    if ($CopilotSkill) {
        Write-Host "  pruner init C:\path\to\project --copilot-skill   # Copilot CLI skill files"
    }
}

Write-Host ""
Write-Host "Done."
