//! Implements functionality for downloading/installing the
//! wasix toolchain (mainly RUSTC).

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};
use reqwest::header::HeaderMap;

use crate::{config::Config, utils::CommandExt};

/// Custom rust repository.
const RUST_REPO: &str = "https://github.com/wasix-org/rust.git";

const RUSTUP_TOOLCHAIN_NAME: &str = "wasix";

/// Try to get the host target triple.
///
/// Only checks for targets that have pre-built toolchains.
#[allow(unreachable_code)]
fn guess_host_target() -> Option<&'static str> {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    return Some("x86_64-unknown-linux-gnu");

    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    return Some("x86_64-apple-darwin");

    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    return Some("aarch64-apple-darwin");

    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    return Some("x86_64-pc-windows-msvc");

    None
}

/// Release returned by Github API.
#[derive(serde::Deserialize)]
struct GithubReleaseData {
    assets: Vec<GithubAsset>,
    tag_name: String,
}

/// Release asset returned by Github API.
#[derive(serde::Deserialize)]
struct GithubAsset {
    browser_download_url: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainSpec {
    Latest,
    Version(String),
}

impl Display for ToolchainSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolchainSpec::Latest => write!(f, "latest"),
            ToolchainSpec::Version(v) => write!(f, "{v}"),
        }
    }
}

impl From<String> for ToolchainSpec {
    fn from(value: String) -> Self {
        if value == "latest" {
            ToolchainSpec::Latest
        } else {
            ToolchainSpec::Version(value)
        }
    }
}

impl ToolchainSpec {
    pub fn is_latest(&self) -> bool {
        *self == ToolchainSpec::Latest
    }
}

/// Download a pre-built toolchain from Github releases.
fn download_toolchain(
    target: &str,
    toolchains_root_dir: &Path,
    toolchain_spec: ToolchainSpec,
) -> Result<PathBuf, anyhow::Error> {
    let mut headers = HeaderMap::new();

    // Use api token if specified via env var.
    // Prevents 403 errors when IP is throttled by Github API.
    let gh_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty());

    if let Some(token) = gh_token {
        headers.insert("authorization", format!("Bearer {token}").parse()?);
    }

    let client = reqwest::blocking::Client::builder()
        .default_headers(headers)
        .user_agent("cargo-wasix")
        .build()?;

    let repo = RUST_REPO
        .trim_start_matches("https://github.com/")
        .trim_end_matches(".git");

    let postfix = if toolchain_spec.is_latest() {
        format!("{toolchain_spec}")
    } else {
        format!("tags/{toolchain_spec}")
    };

    let release_url = format!("https://api.github.com/repos/{repo}/releases/{postfix}");

    eprintln!("Finding {toolchain_spec} release... ({release_url})...");

    let release: GithubReleaseData = client
        .get(&release_url)
        .send()?
        .error_for_status()
        .context("Could not download release info")?
        .json()
        .context("Could not deserialize release info")?;

    // Try to find the asset for the wanted target triple.
    let rust_asset_name = format!("rust-toolchain-{target}.tar.gz");
    let rust_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == rust_asset_name)
        .with_context(|| {
            format!(
                "Release {} does not have a prebuilt toolchain for host {}",
                release.tag_name, target
            )
        })?;

    let toolchain_dir = toolchains_root_dir.join(format!("{target}_{}", release.tag_name));
    if toolchain_dir.is_dir() {
        eprintln!(
            "Toolchain path {} already exists - deleting existing files!",
            toolchain_dir.display()
        );
        std::fs::remove_dir_all(&toolchain_dir)?;
    }

    // Download.
    eprintln!(
        "Downloading Rust toolchain from url '{}'...",
        &rust_asset.browser_download_url
    );
    let res = client
        .get(&rust_asset.browser_download_url)
        .send()?
        .error_for_status()?;

    let decoder = flate2::read::GzDecoder::new(res);
    let mut archive = tar::Archive::new(decoder);

    let rust_dir = toolchain_dir.join("rust");
    archive.unpack(&rust_dir)?;

    // Ensure permissions.
    #[cfg(target_family = "unix")]
    {
        use std::os::unix::fs::PermissionsExt;

        let iter1 = std::fs::read_dir(rust_dir.join("bin"))?;
        let iter2 = std::fs::read_dir(rust_dir.join(format!("lib/rustlib/{target}/bin")))?;

        // Make sure the binaries can be executed.
        for res in iter1.chain(iter2) {
            let entry = res?;
            if entry.file_type()?.is_file() {
                let mut perms = entry.metadata()?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(entry.path(), perms)?;
            }
        }
    }

    eprintln!("Downloaded toolchain {} to {}", target, rust_dir.display());

    Ok(toolchain_dir)
}

/// Tries to download a pre-built toolchain if possible, and builds the
/// toolchain locally otherwise.
///
/// Returns the path to the toolchain.
pub fn install_prebuilt_toolchain(
    toolchain_dir: &Path,
    toolchain_spec: ToolchainSpec,
) -> Result<RustupToolchain, anyhow::Error> {
    if let Some(target) = guess_host_target() {
        match download_toolchain(target, toolchain_dir, toolchain_spec) {
            Ok(path) => RustupToolchain::link(RUSTUP_TOOLCHAIN_NAME, &path.join("rust")),
            Err(err) => {
                eprintln!("Could not download pre-built toolchain: {err:?}");

                let root_cause = err.root_cause();
                let root_description = format!("{root_cause:?}");
                if root_description.contains("HTTP") && root_description.contains("api.github.com")
                {
                    eprintln!("\nHint: You can pass in a Github token via the GITHUB_TOKEN environment variable to avoid rate limits");
                }

                Err(err.context("Download of pre-built toolchain failed"))
            }
        }
    } else {
        Err(anyhow::anyhow!(
            "The WASIX toolchain is not available for download on this platform. Build it yourself with: 'cargo wasix build-toolchain'"
        ))
    }
}

#[derive(Clone, Debug)]
pub struct RustupToolchain {
    pub name: String,
    pub path: PathBuf,
}

impl RustupToolchain {
    /// Verify if the "wasix" toolchain is present in rustup.
    ///
    /// Returns the path to the toolchain.
    fn find_by_name(name: &str) -> Result<Option<Self>, anyhow::Error> {
        let out = Command::new("rustup")
            .args(["toolchain", "list", "--verbose"])
            .capture_stdout()?;
        let path_raw = out
            .lines()
            .find(|line| {
                let line = line.trim_start();
                line.starts_with(name)
                    && matches!(line.chars().nth(name.len()), Some(c) if c.is_whitespace())
            })
            .and_then(|line| line.strip_prefix(name))
            .map(|line| {
                let default_toolchain = "(default)";

                let line = line.trim();
                line.strip_prefix(default_toolchain)
                    .map(|line| line.trim())
                    .unwrap_or(line)
            });

        if let Some(path) = path_raw {
            Ok(Some(Self {
                name: name.to_string(),
                path: path.into(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Link the "wasix" toolchain to a local directory via rustup.
    fn link(name: &str, dir: &Path) -> Result<Self, anyhow::Error> {
        eprintln!(
            "Activating rustup toolchain {} at {}...",
            name,
            dir.display()
        );

        // Small sanity check.
        #[cfg(not(target_os = "windows"))]
        let rustc_exe = "rustc";
        #[cfg(target_os = "windows")]
        let rustc_exe = "rustc.exe";

        let rustc_path = dir.join("bin").join(rustc_exe);
        if !rustc_path.is_file() {
            bail!(
                "Invalid toolchain directory: rustc executable not found at {}",
                rustc_path.display()
            );
        }

        // If already present, unlink first.
        // This is required because otherwise rustup can get in a buggy state.
        if Self::find_by_name(name)?.is_some() {
            Command::new("rustup")
                .args(["toolchain", "remove", name])
                .run()
                .context("Could not remove wasix toolchain")?;
        }

        Command::new("rustup")
            .args(["toolchain", "link", name])
            .arg(dir)
            .run_verbose()
            .context("Could not link toolchain: rustup not installed?")?;

        eprintln!("rustup toolchain {name} was linked and is now available!");

        Ok(Self {
            name: name.to_string(),
            path: dir.into(),
        })
    }
}

/// Makes sure that the wasix toolchain is available.
///
/// Tries to download a pre-built toolchain if possible, and builds the toolchain
/// locally otherwise.
///
/// Also checks that the toolchain is correctly installed.
///
/// Returns the path to the toolchain.
pub fn ensure_toolchain(config: &Config) -> Result<RustupToolchain, anyhow::Error> {
    let _lock = Config::acquire_lock()?;

    let toolchain = if let Some(chain) = RustupToolchain::find_by_name(RUSTUP_TOOLCHAIN_NAME)? {
        chain
    } else if !config.is_offline {
        install_prebuilt_toolchain(&Config::toolchain_dir()?, ToolchainSpec::Latest)?
    } else {
        bail!(
            r#"
Could not detect wasix toolchain, and could not install because CARGO_WASIX_OFFLINE is set.
Run `cargo wasix build-toolchain if you want to build locally.
WARNING: building takes a long time!"#
        );
    };

    // Sanity check the toolchain.
    #[cfg(not(target_os = "windows"))]
    let rust_cmd = "rustc";
    #[cfg(target_os = "windows")]
    let rust_cmd = "rustc.exe";

    let rust_sysroot = Command::new(rust_cmd)
        .arg(format!("+{}", toolchain.name))
        .arg("--print")
        .arg("sysroot")
        .capture_stdout()
        .map(|out| PathBuf::from(out.trim()))
        .context("Could not execute rustc")?;
    assert_eq!(toolchain.path, rust_sysroot);

    let lib_name = "lib/rustlib/wasm32-wasmer-wasi";
    let lib_dir = rust_sysroot.join(lib_name);
    if !lib_dir.exists() {
        bail!(
            "Invalid wasix rustup toolchain {} at {}: {} does not exist",
            toolchain.name,
            toolchain.path.display(),
            lib_dir.display()
        );
    }
    Ok(toolchain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_toolchain() {
        let tmp_dir = std::env::temp_dir().join("cargo-wasix").join("download");
        if tmp_dir.is_dir() {
            std::fs::remove_dir_all(&tmp_dir).unwrap_or_default();
        }
        let root = download_toolchain("x86_64-unknown-linux-gnu", &tmp_dir, ToolchainSpec::Latest)
            .unwrap();
        let dir = root.join("rust");

        #[cfg(not(target_os = "windows"))]
        let exe_name = "rustc";
        #[cfg(target_os = "windows")]
        let exe_name = "rustc.exe";

        assert!(dir.join("bin").join(exe_name).is_file());
        std::fs::remove_dir_all(&tmp_dir).ok();
    }
}
