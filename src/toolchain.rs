use std::{
    path::{Path, PathBuf},
    process::Command,
    thread::available_parallelism,
};

use anyhow::{bail, Context};

use crate::{
    config::Config,
    utils::{ensure_binary, CommandExt},
};

const LIBC_REPO: &str = "https://github.com/john-sharratt/wasix-libc.git";

/// Custom rust repository.
const RUST_REPO: &str = "https://github.com/theduke/rust.git";
/// Branch to use in the custom Rust repo.
const RUST_BRANCH: &str = "wasix5";

/// Download url for LLVM + clang.
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

/// Build the wasix toolchain.
///
/// Returns the toolchain directory path.
pub fn build_toolchain(
    options: BuildToochainOptions,
) -> Result<Option<RustBuildOutput>, anyhow::Error> {
    eprintln!("Building the wasix toolchain...");

    if ensure_binary("apt-get", &["--version"]).is_ok() {
        setup_apt()?;
    }

    if options.build_libc {
        build_libc(&options.root, None, options.update_repos)?;
    } else {
        let dir = options.root.join("wasix-libc");
        let dir32 = dir.join("sysroot32");
        let dir64 = dir.join("sysroot64");
        if !(dir32.is_dir() && dir64.is_dir()) {
            bail!(
                "Tried to skip libc build, but {} or {} were not found",
                dir32.display(),
                dir64.display()
            )
        }
        eprintln!("Skipping libc build!");
    }

    if options.build_rust {
        let out = build_rust(
            &options.root,
            None,
            options.rust_host_triple.as_deref(),
            options.update_repos,
        )?;
        Ok(Some(out))
    } else {
        eprintln!("Skipping rustc build!");
        Ok(None)
    }
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
            .args(["clone", source])
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
            .args(["submodule", "update", "--init", "--recursive"])
            .current_dir(path)
            .run_verbose()?;
    }

    eprintln!("Git repo ready at {}", path.display());

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn build_libc(
    build_root: &Path,
    git_tag: Option<String>,
    update_repo: bool,
) -> Result<(), anyhow::Error> {
    anyhow::bail!("libc builds are only supported on Linux");
}

/// Build the wasix-libc sysroot.
// Currently only works on Linux.
// Mac OS support is easy to add.
#[cfg(target_os = "linux")]
fn build_libc(
    build_root: &Path,
    git_tag: Option<String>,
    update_repo: bool,
) -> Result<(), anyhow::Error> {
    eprintln!("Building wasix-libc...");

    ensure_binary("git", &["--version"])?;

    let git_tag = git_tag.as_deref().unwrap_or("main");

    std::fs::create_dir_all(build_root)
        .with_context(|| format!("Could not create directory: {}", build_root.display()))?;
    let build_dir = build_root.join("wasix-libc");

    if update_repo {
        prepare_git_repo(LIBC_REPO, git_tag, &build_dir, true)?;
    }

    eprintln!("Ensuring LLVM...");
    let llvm_dir = build_root.join("llvm-15");
    if !llvm_dir.join("bin").join("clang").is_file() {
        eprintln!("Downloading LLVM...");
        std::fs::create_dir_all(&llvm_dir)?;

        let archive_path = build_dir.join("llvm.tar.xz");

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

    // Now run the build.

    // TODO: Should we run make clean? (prevents caching...)
    // Command::new("make")
    //     .arg("clean")
    //     .current_dir(&build_dir)
    //     .run_verbose()?;

    eprintln!("Building wasm32...");
    let dir32 = build_dir.join("sysroot32");

    eprintln!("Generating headers...");
    Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            "tools/wasix-headers/Cargo.toml",
            "generate-libc",
        ])
        .current_dir(&build_dir)
        .run_verbose()?;
    Command::new("make")
        .arg(format!(
            "-j{}",
            available_parallelism().map(|x| x.get()).unwrap_or(1)
        ))
        .current_dir(&build_dir)
        .env("TARGET_ARCH", "wasm32")
        .env("TARGET_OS", "wasix")
        .env("CC", llvm_dir.join("bin").join("clang"))
        .env("NM", llvm_dir.join("bin").join("llvm-nm"))
        .env("AR", llvm_dir.join("bin").join("llvm-ar"))
        .run_verbose()?;
    std::fs::remove_file(build_dir.join("sysroot/lib/wasm32-wasi/libc-printscan-long-double.a"))
        .ok();
    if dir32.is_dir() {
        std::fs::remove_dir_all(&dir32)?;
    }
    std::fs::rename(build_dir.join("sysroot"), &dir32)?;

    eprintln!("Building wasm64...");
    let dir64 = build_dir.join("sysroot64");

    eprintln!("Generating headers...");
    Command::new("cargo")
        .args([
            "run",
            "--manifest-path",
            "tools/wasix-headers/Cargo.toml",
            "generate-libc",
            "--64bit",
        ])
        .current_dir(&build_dir)
        .run_verbose()?;
    Command::new("make")
        .current_dir(&build_dir)
        .env("TARGET_ARCH", "wasm64")
        .env("TARGET_OS", "wasix")
        .env("CC", llvm_dir.join("bin").join("clang"))
        .env("NM", llvm_dir.join("bin").join("llvm-nm"))
        .env("AR", llvm_dir.join("bin").join("llvm-ar"))
        .run_verbose()?;
    std::fs::remove_file(build_dir.join("sysroot/lib/wasm64-wasi/libc-printscan-long-double.a"))
        .ok();
    if dir64.is_dir() {
        std::fs::remove_dir_all(&dir64)?;
    }
    std::fs::rename(build_dir.join("sysroot"), &dir64)?;

    eprintln!(
        "wasix-libc build complete!\n{}\n{}",
        dir32.display(),
        dir64.display(),
    );

    Ok(())
}

/// Info for a successful rust toolchain build.
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
    let git_tag = tag.unwrap_or(RUST_BRANCH);

    if update_repo {
        prepare_git_repo(RUST_REPO, git_tag, &rust_dir, true)?;
    }

    let config = r#"
changelog-seen = 2

# NOTE: can't enable because using the cached llvm prevents building lld,
# which is required for the toolchain to work.
#[llvm]
#download-ci-llvm = true

[build]
target = ["wasm32-wasmer-wasi", "wasm64-wasmer-wasi"]
extended = true
tools = [ "clippy", "rustfmt" ]
configure-args = []

[rust]
lld = true
llvm-tools = true

[target.wasm32-wasmer-wasi]
wasi-root = "../wasix-libc/sysroot32"

[target.wasm64-wasmer-wasi]
wasi-root = "../wasix-libc/sysroot64"
"#;

    std::fs::write(rust_dir.join("config.toml"), config)?;

    // Stage 1.
    let mut cmd = Command::new("python3");
    cmd.args(["x.py", "build"]);
    if let Some(triple) = host_triple {
        cmd.args(["--host", triple]);
    }
    cmd.current_dir(&rust_dir).run_verbose()?;

    // Stage 2.
    let mut cmd = Command::new("python3");
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

    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    return Some("x86_64-apple-darwin");

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
fn download_toolchain(target: &str, toolchain_dir: &Path) -> Result<PathBuf, anyhow::Error> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("cargo-wasix")
        .build()?;

    let repo = RUST_REPO
        .trim_start_matches("https://github.com/")
        .trim_end_matches(".git");
    let release_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: GithubReleaseData = client
        .get(&release_url)
        .send()?
        .error_for_status()
        .context("Could not download release info")?
        .json()
        .context("Could not deserialize release info")?;

    // Try to find the asset for the wanted target triple.
    let asset_name = format!("rust-toolchain-{target}.tar.gz");
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .with_context(|| {
            format!(
                "Release {} does not have a prebuilt toolchain for host {}",
                release.tag_name, target
            )
        })?;

    // Download.
    eprintln!(
        "Downloading release from url '{}'...",
        &asset.browser_download_url
    );
    let res = client
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?;

    eprintln!("Extracting...");
    let decoder = flate2::read::GzDecoder::new(res);
    let mut archive = tar::Archive::new(decoder);

    let out_dir = toolchain_dir.join(format!("{target}_{}", release.tag_name));
    archive.unpack(&out_dir)?;

    // Ensure permissions.
    #[cfg(target_family = "unix")]
    {
        use std::os::unix::fs::PermissionsExt;

        let iter1 = std::fs::read_dir(out_dir.join("bin"))?;
        let iter2 = std::fs::read_dir(out_dir.join(format!("lib/rustlib/{target}/bin")))?;

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

    eprintln!("Downloaded toolchain {} to {}", target, out_dir.display());

    Ok(out_dir)
}

/// Link the "wasix" toolchain to a local directory via rustup.
fn rustup_link_wasix_toolchain(dir: &Path) -> Result<(), anyhow::Error> {
    eprintln!("Activating toolchain...");
    Command::new("rustup")
        .args(["toolchain", "link", "wasix"])
        .arg(dir)
        .run_verbose()
        .context("Could not link toolchain: rustup not installed?")?;

    eprintln!("wasix toolchain was linked and is now available!");

    Ok(())
}

/// Tries to download a pre-built toolchain if possible, and builds the
/// toolchain locally otherwise.
fn install_toolchain(toolchain_dir: &Path, build_dir: &Path) -> Result<(), anyhow::Error> {
    if let Some(target) = guess_host_target() {
        match download_toolchain(target, toolchain_dir) {
            Ok(path) => {
                rustup_link_wasix_toolchain(&path)?;
                return Ok(());
            }
            Err(err) => {
                eprintln!("Could not download pre-built toolchain: {err:?}");
            }
        }
    }

    eprintln!("Could not install pre-built toolchain!");
    eprintln!("Building local toolchain...");
    eprintln!("WARNING: this could take a long time and use a lot of disk space!");

    let rust = build_toolchain(BuildToochainOptions {
        root: build_dir.to_owned(),
        build_libc: true,
        build_rust: true,
        rust_host_triple: None,
        update_repos: true,
    })?
    .unwrap();
    rustup_link_wasix_toolchain(&rust.toolchain_dir)?;

    Ok(())
}

/// Makes sure that the wasix toolchain is available.
///
/// Tries to download a pre-built toolchain if possible, and builds the toolchain
/// locally otherwise.
///
/// Also checks that the toolchain is correctly installed.
pub fn ensure_toolchain(_config: &Config, is64bit: bool) -> Result<(), anyhow::Error> {
    // rustup is not itself synchronized across processes so at least attempt to
    // synchronize our own calls. This may not work and if it doesn't we tried,
    // this is largely opportunistic anyway.
    let _lock = crate::utils::flock(&Config::data_dir()?.join("rustup-lock"));

    // First check if the toolchain is present
    let toolchains = Command::new("rustup")
        .arg("toolchain")
        .arg("list")
        .capture_stdout()
        .ok();

    let has_wasix_toolchain = if let Some(toolchains) = toolchains {
        toolchains.lines().any(|a| a == "wasix")
    } else {
        false
    };

    // Install the toolchain if its not there
    if !has_wasix_toolchain {
        install_toolchain(
            &Config::toolchain_dir()?,
            &Config::cache_dir()?.join("build"),
        )?;
    }

    // Ok we need to actually check since this is perhaps the first time we've
    // ever checked. Let's ask rustc what its sysroot is and see if it has a
    // wasm64-wasi folder.
    let push_toolchain = std::env::var("RUSTUP_TOOLCHAIN").unwrap_or_default();
    std::env::set_var("RUSTUP_TOOLCHAIN", "wasix");
    let sysroot = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .capture_stdout()
        .ok();
    if let Some(sysroot) = sysroot {
        let sysroot = Path::new(sysroot.trim());
        let lib_name = if is64bit {
            "lib/rustlib/wasm64-wasmer-wasi"
        } else {
            "lib/rustlib/wasm32-wasmer-wasi"
        };
        if sysroot.join(lib_name).exists() {
            std::env::set_var("RUSTUP_TOOLCHAIN", push_toolchain);
            return Ok(());
        }
    }
    std::env::set_var("RUSTUP_TOOLCHAIN", push_toolchain);

    bail!(
        "failed to find the `wasm64-wasmer-wasi` target installed, and rustup \
        is also not detected, you'll need to be sure to install the \
        `wasm64-wasi` target before using this command"
    );
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
        let dir = download_toolchain("x86_64-unknown-linux-gnu", &tmp_dir).unwrap();

        assert!(dir.join("bin").join("rustc").is_file());

        std::fs::remove_dir_all(&tmp_dir).ok();
    }
}
