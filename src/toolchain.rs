//! Implements functionality for downloading/installing or building the
//! wasix toolchain (mainly RUSTC).
//!
//! Mainly:
//! * Download/install pre-built toolchains.
//! * Build the whole toolchain

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};
use reqwest::header::HeaderMap;

use crate::{
    config::Config,
    utils::{ensure_binary, CommandExt},
};

/// Custom rust repository.
const RUST_REPO: &str = "https://github.com/wasix-org/rust.git";
/// Branch to use in the custom Rust repo.
const RUST_BRANCH: &str = "wasix";

const RUSTUP_TOOLCHAIN_NAME: &str = "wasix";

const LIBC_REPO: &str = "https://github.com/wasix-org/wasix-libc.git";

/// Download url for LLVM + clang (LINUX).
const LLVM_LINUX_SOURCE: &str = "https://github.com/llvm/llvm-project/releases/download/llvmorg-15.0.2/clang+llvm-15.0.2-x86_64-unknown-linux-gnu-rhel86.tar.xz";

/// Options for a toolchain build.
pub struct BuildToochainOptions {
    root: PathBuf,
    build_libc: bool,
    build_rust: bool,
    rust_host_triple: Option<String>,

    update_repos: bool,
}

impl BuildToochainOptions {
    pub fn from_env() -> Result<Self, anyhow::Error> {
        // Read components to build from env var.
        let (build_libc, build_rust) = match std::env::var("WASIX_COMPONENTS")
            .unwrap_or_default()
            .as_str()
        {
            "" | "all" => (true, true),
            "libc" => (true, false),
            "rust" => (false, true),
            other => {
                bail!("Invalid env var WASIX_COMPONENTS with value '{other}' - expected 'all' or 'libc'");
            }
        };

        let root = if let Ok(dir) = std::env::var("WASIX_BUILD_DIR") {
            PathBuf::from(dir)
        } else {
            #[allow(deprecated)]
            std::env::home_dir()
                .context("Could not determine home dir. set WASIX_BUILD_DIR env var!")?
                .join(".wasix")
        };

        let rust_host_triple = std::env::var("WASIX_RUST_HOST").ok();
        let update_repos = std::env::var("WASIX_NO_UPDATE_REPOS").is_err();

        Ok(Self {
            root,
            build_rust,
            build_libc,
            rust_host_triple,
            update_repos,
        })
    }
}

fn ensure_libc_dir_valid(dir: &Path) -> Result<(), anyhow::Error> {
    // Make sure sysroot dirs exist.
    let dir32 = dir.join("sysroot32");
    let archive32 = dir32.join("lib/wasm32-wasi/libc.a");

    let dir64 = dir.join("sysroot64");
    let archive64 = dir64.join("lib/wasm64-wasi/libc.a");

    if !dir32.is_dir() {
        bail!(
            "Invalid libc dir: directory does not exit: '{}'",
            dir32.display()
        );
    }
    if !archive32.is_file() {
        bail!(
            "Invalid libc dir: archive does not exit: '{}'",
            archive32.display()
        );
    }

    if !dir64.is_dir() {
        bail!(
            "Invalid libc dir: directory does not exit: '{}'",
            dir64.display()
        );
    }
    if !archive64.is_file() {
        bail!(
            "Invalid libc dir: archive does not exit: '{}'",
            archive64.display()
        );
    }

    Ok(())
}

/// Build the wasix toolchain.
///
/// Returns the toolchain directory path.
pub fn build_toolchain(
    options: BuildToochainOptions,
) -> Result<Option<RustBuildOutput>, anyhow::Error> {
    eprintln!("Building the wasix toolchain...");
    eprintln!("WARNING: this could take a long time and use a lot of disk space!");

    if ensure_binary("apt-get", &["--version"]).is_ok() {
        setup_apt()?;
    }

    let libc_dir = options.root.join("wasix-libc");
    if options.build_libc {
        build_libc(&options.root, None, options.update_repos)?;
        ensure_libc_dir_valid(&libc_dir).context("libc build failed")?;
    } else {
        eprintln!("Skipping libc build!");
        ensure_libc_dir_valid(&libc_dir)
            .context("libc build skipped, but specified path invalid")?;
    }

    if !options.build_rust {
        return Ok(None);
    }

    let out = build_rust(
        &options.root,
        None,
        options.rust_host_triple.as_deref(),
        options.update_repos,
    )?;

    RustupToolchain::link(RUSTUP_TOOLCHAIN_NAME, &out.toolchain_dir)?;

    Ok(Some(out))
}

/// Install basic required packages on Debian based systems.
fn setup_apt() -> Result<(), anyhow::Error> {
    let have_sudo = ensure_binary("sudo", &["--version"]).is_ok();

    let args = &[
        "install",
        "-y",
        // Packages.
        "curl",
        "xz-utils",
        "build-essential",
        "git",
        "python3",
    ];

    if have_sudo {
        Command::new("sudo")
            .arg("apt-get")
            .args(args)
            .run_verbose()?;
    } else {
        Command::new("apt-get").args(args).run_verbose()?;
    }

    Ok(())
}

/// Initialize a Git repo.
///
/// Clone if it doesn't exist yet, otherwise update the branch/tag.
fn prepare_git_repo(
    source: &str,
    tag: &str,
    path: &Path,
    all_submodules: bool,
) -> Result<(), anyhow::Error> {
    eprintln!("Preparing git repo {source} with tag/branch {tag}");
    ensure_binary("git", &["--version"])?;

    if !path.join(".git").is_dir() {
        Command::new("git")
            .args([
                "clone",
                // "--depth",
                // "1",
                // "--recurse-submodules",
                // "--shallow-submodules",
                source,
            ])
            .arg(path)
            .run_verbose()?;
    }
    Command::new("git")
        .args(["fetch", "origin", tag])
        .current_dir(path)
        .run_verbose()?;
    Command::new("git")
        .args(["reset", "--hard", tag])
        .current_dir(path)
        .run_verbose()?;

    if all_submodules {
        Command::new("git")
            .args(["submodule", "update", "--init", "--recursive", "--progress"]) // added progress as llvm takes a very long time
            .current_dir(path)
            .run_verbose()?;
    }

    eprintln!("Git repo ready at {}", path.display());

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn build_libc(
    _build_root: &Path,
    _git_tag: Option<String>,
    _update_repo: bool,
) -> Result<(), anyhow::Error> {
    anyhow::bail!("libc builds are only supported on Linux");
}

/// Build the wasix-libc sysroot.
// Currently only works on Linux.
#[cfg(target_os = "linux")]
fn build_libc(
    build_root: &Path,
    git_tag: Option<String>,
    update_repo: bool,
) -> Result<(), anyhow::Error> {
    use crate::utils::copy_path;

    eprintln!("Building wasix-libc...");

    ensure_binary("git", &["--version"])?;

    let git_tag = git_tag.as_deref().unwrap_or("main");

    std::fs::create_dir_all(build_root)
        .with_context(|| format!("Could not create directory: {}", build_root.display()))?;
    let libc_dir = build_root.join("wasix-libc");

    if update_repo {
        prepare_git_repo(LIBC_REPO, git_tag, &libc_dir, true)?;
    }

    eprintln!("Ensuring LLVM...");
    let llvm_dir = build_root.join("llvm-15");
    if !llvm_dir.join("bin").join("clang").is_file() {
        eprintln!("Downloading LLVM...");
        std::fs::create_dir_all(&llvm_dir)?;

        let archive_path = libc_dir.join("llvm.tar.xz");

        Command::new("curl")
            .args(["-L", "-o"])
            .arg(&archive_path)
            .arg(LLVM_LINUX_SOURCE)
            .run_verbose()?;

        eprintln!("Extracting LLVM...");
        Command::new("tar")
            .args(["xJf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&llvm_dir)
            .args(["--strip-components", "1"])
            .run_verbose()?;

        std::fs::remove_file(&archive_path).ok();

        eprintln!("Downloaded LLVM to {}", llvm_dir.display());
    }
    // Sanity check for clang.
    Command::new(llvm_dir.join("bin").join("clang"))
        .arg("--version")
        .run_verbose()?;

    let original_path_env = std::env::var("PATH")?;
    let path = format!(
        "{}:{}",
        llvm_dir.join("bin").to_str().unwrap(),
        original_path_env
    );

    // Now run the build.

    // TODO: Should we run make clean? (prevents caching...)
    // Command::new("make")
    //     .arg("clean")
    //     .current_dir(&build_dir)
    //     .run_verbose()?;

    let tmp_dir = std::env::temp_dir();

    let dir32 = libc_dir.join("sysroot32");
    let dir32_tmp = tmp_dir.join("sysroot32");
    let dir64 = libc_dir.join("sysroot64");
    let dir64_tmp = tmp_dir.join("sysroot64");

    if dir32_tmp.is_dir() {
        std::fs::remove_dir_all(&dir32_tmp)?;
    }
    if dir64_tmp.is_dir() {
        std::fs::remove_dir_all(&dir64_tmp)?;
    }

    eprintln!("Building wasm32...");
    Command::new("make")
        .arg("clean")
        .current_dir(&libc_dir)
        .run_verbose()?;
    Command::new("bash")
        .arg("./build32.sh")
        .current_dir(&libc_dir)
        .env("PATH", &path)
        .run_verbose()
        .context("could not build sysroot32")?;

    copy_path(&dir32, &dir32_tmp, false, true)?;

    eprintln!("Building wasm64...");
    Command::new("make")
        .arg("clean")
        .current_dir(&libc_dir)
        .run_verbose()?;
    Command::new("bash")
        .arg("./build64.sh")
        .current_dir(&libc_dir)
        .env("PATH", &path)
        .run_verbose()
        .context("could not build sysroot64")?;
    // copy_path(&dir64, &dir64_tmp, false, true)?;

    // Command::new("make")
    //     .arg("clean")
    //     .current_dir(&libc_dir)
    //     .run_verbose()?;

    if dir32.is_dir() {
        std::fs::remove_dir_all(&dir32)?;
    }

    std::fs::rename(&dir32_tmp, &dir32).context("could not copy temp dir")?;
    // std::fs::rename(&dir64_tmp, &dir64)?;

    eprintln!(
        "wasix-libc build complete!\n{}\n{}",
        dir32.display(),
        dir64.display(),
    );

    Ok(())
}

/// Output info of a successful rust toolchain build.
pub struct RustBuildOutput {
    pub target: String,
    pub toolchain_dir: PathBuf,
}

/// Build the Rust toolchain for wasm{32,64}-wasmer-wasi
fn build_rust(
    build_root: &Path,
    tag: Option<&str>,
    host_triple: Option<&str>,
    update_repo: bool,
) -> Result<RustBuildOutput, anyhow::Error> {
    let rust_dir = build_root.join("wasix-rust");
    let libc_dir = build_root.join("wasix-libc");
    let git_tag = tag.unwrap_or(RUST_BRANCH);

    ensure_libc_dir_valid(&libc_dir)?;

    if update_repo {
        prepare_git_repo(RUST_REPO, git_tag, &rust_dir, true)?;
    }

    let sysroot32 = libc_dir.join("sysroot32");
    let sysroot64 = libc_dir.join("sysroot64");
    if !sysroot32.is_dir() || !sysroot64.is_dir() {
        anyhow::bail!(
            "Could not find wasix-libc sysroots at {} and {}",
            sysroot32.display(),
            sysroot64.display()
        );
    }

    let config_tpl = r#"
changelog-seen = 2

# NOTE: can't enable because using the cached llvm prevents building rust-lld,
# which is required for the toolchain to work.
[llvm]
download-ci-llvm = false

[build]
target = ["wasm32-wasmer-wasi", "wasm64-wasmer-wasi"]
extended = true
tools = [ "clippy", "rustfmt" ]
configure-args = []

[rust]
lld = true
llvm-tools = true

[target.wasm32-wasmer-wasi]
wasi-root = "{sysroot32}"

[target.wasm64-wasmer-wasi]
wasi-root = "{sysroot64}"
"#;

    // Note: need to replace \ with \\ for Windows paths.
    let config = config_tpl
        .replace(
            "{sysroot32}",
            &sysroot32.to_str().unwrap().replace('\\', "\\\\"),
        )
        .replace(
            "{sysroot64}",
            &sysroot64.to_str().unwrap().replace('\\', "\\\\"),
        );

    std::fs::write(rust_dir.join("config.toml"), config)?;

    // Stage 1.

    let has_python3 = Command::new("python3").arg("--version").run().is_ok();
    let python_cmd = if has_python3 { "python3" } else { "python" };

    let mut cmd = Command::new(python_cmd);
    // Added because x.py checks for GITHUB_ACTIONS env var and does some weird
    // things that break the build.
    cmd.env("GITHUB_ACTIONS", "false");
    cmd.args(["x.py", "build"]);
    if let Some(triple) = host_triple {
        cmd.args(["--host", triple]);
    }
    cmd.current_dir(&rust_dir).run_verbose()?;

    // Stage 2.
    let mut cmd = Command::new("python3");
    // Added because x.py checks for GITHUB_ACTIONS env var and does some weird
    // things that break the build.
    cmd.env("GITHUB_ACTIONS", "false");
    cmd.arg(rust_dir.join("x.py"))
        .args(["build", "--stage", "2"]);
    if let Some(triple) = host_triple {
        cmd.args(["--host", triple]);
    }
    cmd.current_dir(&rust_dir).run_verbose()?;

    eprintln!("Rust build complete!");

    if let Some(triple) = host_triple {
        let dir = rust_dir.join("build").join(triple).join("stage2");
        Ok(RustBuildOutput {
            target: triple.to_string(),
            toolchain_dir: dir,
        })
    } else {
        // Find target.
        // TODO: properly detect host triple from output?
        // Currently could return the wrong result if multiple hosts were built.
        for res in std::fs::read_dir(rust_dir.join("build"))? {
            let entry = res?;
            let toolchain_dir = entry.path().join("stage2");
            if toolchain_dir.is_dir() {
                let target = entry.file_name().to_string_lossy().to_string();
                return Ok(RustBuildOutput {
                    target,
                    toolchain_dir,
                });
            }
        }

        bail!("Could not find build directory")
    }
}

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

/// Download a pre-built toolchain from Github releases.
fn download_toolchain(target: &str, toolchains_root_dir: &Path) -> Result<PathBuf, anyhow::Error> {
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
    let release_url = format!("https://api.github.com/repos/{repo}/releases/latest");

    eprintln!("Finding latest release... ({release_url})...");

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

    // Find sysroot asset.
    let sysroot_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "wasix-libc.tar.gz")
        .with_context(|| {
            format!(
                "Release {} does not have the sysroot asset",
                release.tag_name,
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

    // Download and extract sysroot.
    eprintln!(
        "Downloading sysroot from url '{}'...",
        &sysroot_asset.browser_download_url
    );
    let res = client
        .get(&sysroot_asset.browser_download_url)
        .send()?
        .error_for_status()?;

    eprintln!("Extracting...");
    let decoder = flate2::read::GzDecoder::new(res);
    let mut archive = tar::Archive::new(decoder);

    let out_dir = toolchain_dir.join("sysroot");
    archive.unpack(&out_dir)?;

    // The archive contains a redundant additional directory. Strip it.
    let wrapper = out_dir.join("wasix-libc");
    if wrapper.is_dir() {
        std::fs::rename(wrapper.join("sysroot32"), out_dir.join("sysroot32"))
            .context("Invalid/missing libc sysroot directory")?;
        std::fs::rename(wrapper.join("sysroot64"), out_dir.join("sysroot64"))
            .context("Invalid/missing libc sysroot directory")?;

        std::fs::remove_dir_all(wrapper).context("Could not delete intermediate directory")?;
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

    eprintln!("Extracting...");
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
pub fn install_prebuilt_toolchain(toolchain_dir: &Path) -> Result<RustupToolchain, anyhow::Error> {
    if let Some(target) = guess_host_target() {
        match download_toolchain(target, toolchain_dir) {
            Ok(path) => RustupToolchain::link(RUSTUP_TOOLCHAIN_NAME, &path.join("rust")),
            Err(err) => {
                eprintln!("Could not download pre-built toolchain: {err:?}");
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
            .find(|line| line.trim().starts_with(name))
            .and_then(|line| line.strip_prefix(name))
            .map(|line| line.trim());

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

    pub fn sysroot_dir(&self, is64bit: bool) -> Option<PathBuf> {
        let size = if is64bit { 64 } else { 32 };
        let path = self.path.parent()?.join(format!("sysroot{size}"));
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
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
pub fn ensure_toolchain(config: &Config, is64bit: bool) -> Result<RustupToolchain, anyhow::Error> {
    let _lock = Config::acquire_lock()?;

    let toolchain = if let Some(chain) = RustupToolchain::find_by_name(RUSTUP_TOOLCHAIN_NAME)? {
        chain
    } else if !config.is_offline {
        install_prebuilt_toolchain(&Config::toolchain_dir()?)?
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

    let lib_name = if is64bit {
        "lib/rustlib/wasm64-wasmer-wasi"
    } else {
        "lib/rustlib/wasm32-wasmer-wasi"
    };
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
        let root = download_toolchain("x86_64-unknown-linux-gnu", &tmp_dir).unwrap();
        let dir = root.join("rust");

        #[cfg(not(target_os = "windows"))]
        let exe_name = "rustc";
        #[cfg(target_os = "windows")]
        let exe_name = "rustc.exe";

        assert!(dir.join("bin").join(exe_name).is_file());
        std::fs::remove_dir_all(&tmp_dir).ok();
    }
}
