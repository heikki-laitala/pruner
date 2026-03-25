//! Self-upgrade: download latest release from GitHub and replace the running binary.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const REPO: &str = "heikki-laitala/pruner";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Resolved platform target matching release asset names.
struct Platform {
    os: &'static str,
    arch: &'static str,
}

fn detect_platform() -> Result<Platform> {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        anyhow::bail!("Unsupported OS for self-upgrade");
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        anyhow::bail!("Unsupported architecture for self-upgrade");
    };

    Ok(Platform { os, arch })
}

/// Fetch the latest release tag from GitHub.
pub fn check_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body: serde_json::Value = ureq::get(&url)
        .header("User-Agent", &format!("pruner/{CURRENT_VERSION}"))
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("Failed to fetch latest release from GitHub")?
        .body_mut()
        .read_json()
        .context("Failed to parse GitHub API response")?;

    body["tag_name"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No tag_name in GitHub release response"))
}

/// Normalize version string: strip leading 'v' for comparison.
fn version_bare(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

/// Compare two semver-ish version strings. Returns true if `latest` is newer than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let cur = version_bare(current);
    let lat = version_bare(latest);
    // Simple lexicographic comparison works for semver with same segment count
    // But let's do proper numeric comparison
    let parse = |v: &str| -> Vec<u64> { v.split('.').filter_map(|s| s.parse().ok()).collect() };
    let c = parse(cur);
    let l = parse(lat);
    l > c
}

/// Download the release asset to a temp directory and return path to the extracted binary.
fn download_and_extract(version: &str, platform: &Platform) -> Result<PathBuf> {
    let asset_name = format!("pruner-{}-{}", platform.os, platform.arch);
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;

    let (url, is_zip) = if platform.os == "windows" {
        (
            format!("https://github.com/{REPO}/releases/download/{version}/{asset_name}.zip"),
            true,
        )
    } else {
        (
            format!("https://github.com/{REPO}/releases/download/{version}/{asset_name}.tar.gz"),
            false,
        )
    };

    eprintln!("Downloading {url}...");

    let mut reader = ureq::get(&url)
        .header("User-Agent", &format!("pruner/{CURRENT_VERSION}"))
        .call()
        .with_context(|| format!("Download failed. Check https://github.com/{REPO}/releases"))?
        .into_body()
        .into_reader();

    if is_zip {
        // Download to a temp file first (zip needs seek)
        let zip_path = temp_dir.path().join("download.zip");
        let mut file = std::fs::File::create(&zip_path)?;
        std::io::copy(&mut reader, &mut file)?;

        let file = std::fs::File::open(&zip_path)?;
        let mut archive = zip::ZipArchive::new(file)?;

        // Find the .exe in the archive
        let exe_name = format!("{asset_name}.exe");
        let mut entry = archive
            .by_name(&exe_name)
            .with_context(|| format!("{exe_name} not found in zip archive"))?;

        let out_path = temp_dir.path().join("pruner.exe");
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;

        // Leak the tempdir so it isn't deleted before we use the file
        let dir = temp_dir.keep();
        Ok(dir.join("pruner.exe"))
    } else {
        let decoder = flate2::read::GzDecoder::new(reader);
        let mut archive = tar::Archive::new(decoder);

        let out_path = temp_dir.path().join("pruner");
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            // The archive contains a single file named like "pruner-macos-aarch64"
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("pruner"))
            {
                let mut out_file = std::fs::File::create(&out_path)?;
                std::io::copy(&mut entry, &mut out_file)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755))?;
                }
                break;
            }
        }

        if !out_path.exists() {
            anyhow::bail!("Binary not found in tar archive");
        }

        let dir = temp_dir.keep();
        Ok(dir.join("pruner"))
    }
}

/// Detect which integrations are currently installed by probing filesystem.
struct DetectedIntegrations {
    global: bool,
    hook: bool,
    copilot_global: bool,
    copilot_skill: bool,
}

impl DetectedIntegrations {
    fn has_any(&self) -> bool {
        self.global || self.copilot_global
    }
}

fn detect_installed_integrations() -> DetectedIntegrations {
    let home = dirs::home_dir().unwrap_or_default();

    let claude_hook = home.join(".claude/hooks/pruner-context.sh").exists();
    let claude_skill = home.join(".claude/skills/pruner/SKILL.md").exists();
    let copilot_skill = home.join(".copilot/skills/pruner/SKILL.md").exists();

    DetectedIntegrations {
        global: claude_hook || claude_skill,
        hook: claude_hook,
        copilot_global: copilot_skill,
        copilot_skill,
    }
}

/// Re-run init with the new binary to update config files (hooks, skills, templates).
fn reinit_integrations(integrations: &DetectedIntegrations) -> Result<()> {
    let exe = std::env::current_exe().context("Cannot determine current executable path")?;

    let mut args = vec!["init".to_string()];
    if integrations.global {
        args.push("--global".to_string());
    }
    if integrations.hook {
        args.push("--hook".to_string());
    }
    if integrations.copilot_global {
        args.push("--copilot-global".to_string());
    }
    if integrations.copilot_skill && !integrations.copilot_global {
        args.push("--copilot-skill".to_string());
    }

    eprintln!(
        "Updating integrations (pruner init {})...",
        args[1..].join(" ")
    );
    let status = std::process::Command::new(&exe)
        .args(&args)
        .status()
        .context("Failed to run pruner init for config update")?;

    if !status.success() {
        anyhow::bail!("pruner init failed during upgrade (exit {})", status);
    }
    Ok(())
}

/// Clean up temp directory (best-effort).
fn cleanup_temp(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// Main upgrade entrypoint.
pub fn cmd_upgrade(check: bool, target_version: Option<&str>) -> Result<()> {
    let current = format!("v{CURRENT_VERSION}");
    eprintln!("Current version: {current}");

    // Resolve target version
    let latest = match target_version {
        Some(v) => {
            if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            }
        }
        None => {
            eprintln!("Checking for updates...");
            check_latest_version()?
        }
    };

    // Compare versions
    if target_version.is_none() && !is_newer(&current, &latest) {
        eprintln!("Already up to date ({current}).");
        return Ok(());
    }

    if check {
        if is_newer(&current, &latest) {
            println!("Update available: {current} -> {latest}");
            println!("Run `pruner upgrade` to install.");
        } else {
            println!("Already up to date ({current}).");
        }
        return Ok(());
    }

    eprintln!("Upgrading: {current} -> {latest}");

    // Download
    let platform = detect_platform()?;
    let new_binary = download_and_extract(&latest, &platform)?;
    let temp_dir = new_binary.parent().unwrap().to_path_buf();

    // Replace self
    eprintln!("Replacing binary...");
    self_replace::self_replace(&new_binary)
        .context("Failed to replace binary. Check file permissions.")?;

    // Clean up temp
    cleanup_temp(&temp_dir);

    // Verify
    let exe = std::env::current_exe()?;
    let output = std::process::Command::new(&exe)
        .arg("--version")
        .output()
        .context("Failed to verify new binary")?;
    let version_str = String::from_utf8_lossy(&output.stdout);
    eprintln!("Installed: {}", version_str.trim());

    // Detect and update integrations
    let integrations = detect_installed_integrations();
    if integrations.has_any() {
        reinit_integrations(&integrations)?;
    }

    eprintln!("Upgrade complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("v0.1.5", "v0.1.6"));
        assert!(is_newer("0.1.5", "0.1.6"));
        assert!(!is_newer("v0.1.6", "v0.1.6"));
        assert!(!is_newer("v0.2.0", "v0.1.9"));
        assert!(is_newer("v0.1.9", "v0.2.0"));
        assert!(is_newer("v0.9.0", "v1.0.0"));
    }

    #[test]
    fn test_version_bare() {
        assert_eq!(version_bare("v0.1.5"), "0.1.5");
        assert_eq!(version_bare("0.1.5"), "0.1.5");
    }

    #[test]
    fn test_detect_platform() {
        let p = detect_platform().unwrap();
        // Just verify it returns something valid
        assert!(!p.os.is_empty());
        assert!(!p.arch.is_empty());
    }
}
