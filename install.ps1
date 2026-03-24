# Pruner installer for Windows
#
# Usage:
#   irm https://raw.githubusercontent.com/heikki-laitala/pruner/main/install.ps1 | iex
#
# Or with options:
#   $env:PRUNER_VERSION = "v0.2.0"; irm ... | iex
#   irm ... | iex -CopilotSkill
#   irm ... | iex -CopilotHook
#   irm ... | iex -CopilotGlobal

param(
    [string]$Version = $env:PRUNER_VERSION,
    [string]$InstallDir = "$env:USERPROFILE\.local\bin",
    [switch]$CopilotSkill,
    [switch]$CopilotHook,
    [switch]$CopilotGlobal
)

$ErrorActionPreference = "Stop"

$Repo = "heikki-laitala/pruner"

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

Write-Host ""
Write-Host "To set up pruner in a project:"
Write-Host ""
Write-Host "  pruner init C:\path\to\project"
Write-Host "  pruner init C:\path\to\project --hook"
Write-Host "  pruner init C:\path\to\project --copilot-skill"
Write-Host "  pruner init C:\path\to\project --copilot-hook"
Write-Host ""

if ($CopilotGlobal) {
    Write-Host "Setting up global Copilot integration..."
    & $ExePath init --copilot-skill --copilot-global
    Write-Host ""
} elseif ($CopilotSkill) {
    Write-Host "To install Copilot skill in a project, run:"
    Write-Host "  pruner init C:\path\to\project --copilot-skill"
    Write-Host ""
} elseif ($CopilotHook) {
    Write-Host "To install Copilot hook in a project, run:"
    Write-Host "  pruner init C:\path\to\project --copilot-hook"
    Write-Host ""
}

Write-Host "Done."
