use crate::cache::Cache;
use crate::config::Config;
use crate::utils::CommandExt;
use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tool_path::ToolPath;

mod cache;
mod config;
mod internal;
mod tool_path;
mod toolchain;
mod utils;

pub fn main() {
    // See comments in `rmain` around `*_RUNNER` for why this exists here.
    if env::var("__CARGO_WASIX_RUNNER_SHIM").is_ok() {
        let args = env::args().skip(1).collect();
        println!(
            "{}",
            serde_json::to_string(&CargoMessage::RunWithArgs { args }).unwrap(),
        );
        return;
    }

    let mut config = Config::new();
    match rmain(&mut config) {
        Ok(()) => {}
        Err(e) => {
            config.print_error(&e);
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
enum Subcommand {
    Build,
    BuildToolchain,
    Run,
    Test,
    Bench,
    Check,
    Tree,
    Fix,
}

fn rmain(config: &mut Config) -> Result<()> {
    config.load_cache()?;

    // skip the current executable and the `wasix` inserted by Cargo
    let mut is64bit = false;
    let mut no_message_format = false;
    let mut args = env::args_os().skip(2);
    let subcommand = args.next().and_then(|s| s.into_string().ok());
    let subcommand = match subcommand.as_deref() {
        Some("build") => Subcommand::Build,
        Some("build64") => {
            is64bit = true;
            Subcommand::Build
        }
        Some("build-toolchain") => Subcommand::BuildToolchain,
        Some("run") => Subcommand::Run,
        Some("run64") => {
            is64bit = true;
            Subcommand::Run
        }
        Some("test") => Subcommand::Test,
        Some("test64") => {
            is64bit = true;
            Subcommand::Test
        }
        Some("bench") => Subcommand::Bench,
        Some("bench64") => {
            is64bit = true;
            Subcommand::Bench
        }
        Some("check") => Subcommand::Check,
        Some("check64") => {
            is64bit = true;
            Subcommand::Check
        }
        Some("tree") => {
            no_message_format = true;
            Subcommand::Tree
        }
        Some("tree64") => {
            is64bit = true;
            no_message_format = true;
            Subcommand::Tree
        }
        Some("fix") => Subcommand::Fix,
        Some("self") => return internal::main(&args.collect::<Vec<_>>(), config),
        Some("version") | Some("-V") | Some("--version") => {
            let git_info = match option_env!("GIT_INFO") {
                Some(s) => format!(" ({})", s),
                None => String::new(),
            };
            println!("cargo-wasix {}{}", env!("CARGO_PKG_VERSION"), git_info);
            std::process::exit(0);
        }
        _ => print_help(),
    };

    let mut cargo = Command::new("cargo");
    cargo.arg("+wasix");
    cargo.arg(match subcommand {
        Subcommand::Build => "build",
        Subcommand::BuildToolchain => "build-toolchain",
        Subcommand::Check => "check",
        Subcommand::Fix => "fix",
        Subcommand::Test => "test",
        Subcommand::Tree => "tree",
        Subcommand::Bench => "bench",
        Subcommand::Run => "run",
    });

    // TODO: figure out when these flags are already passed to `cargo` and skip
    // passing them ourselves.
    let target = if is64bit {
        "wasm64-wasmer-wasi"
    } else {
        "wasm32-wasmer-wasi"
    };
    cargo.arg("--target").arg(target);
    if !no_message_format {
        cargo.arg("--message-format").arg("json-render-diagnostics");
    }
    for arg in args {
        if let Some(arg) = arg.to_str() {
            if arg.starts_with("--verbose") || arg.starts_with("-v") {
                config.set_verbose(true);
            }
        }

        cargo.arg(arg);
    }

    let runner_env_var = format!(
        "CARGO_TARGET_{}_RUNNER",
        target.to_uppercase().replace('-', "_")
    );

    // If Cargo actually executes a wasm file, we don't want it to. We need to
    // postprocess wasm files (wasm-opt, wasm-bindgen, etc). As a result we will
    // actually postprocess wasm files after the build. To work around this we
    // could pass `--no-run` for `test`/`bench`, but there's unfortunately no
    // equivalent for `run`. Additionally we want to learn what arguments Cargo
    // parsed to pass to each wasm file.
    //
    // To solve this all we do a bit of a switcharoo. We say that *we* are the
    // runner, and our binary is configured to simply print a json message at
    // the beginning. We'll slurp up these json messages and then actually
    // execute everything at the end.
    //
    // Also note that we check here before we actually build that a runtime is
    // present. We first check the CARGO_TARGET_WASM32_WASIX_RUNNER environement
    // variable for a user-supplied runtime (path or executable) and use the
    // default, namely `wasmer`, if it is not set.
    let (wasix_runner, using_default) = env::var(&runner_env_var)
        .map(|runner_override| (runner_override, false))
        .unwrap_or_else(|_| ("wasmer".to_string(), true));

    match subcommand {
        Subcommand::BuildToolchain => {
            let opts = toolchain::BuildToochainOptions::from_env()?;
            toolchain::build_toolchain(opts)?;
            return Ok(());
        }
        Subcommand::Run | Subcommand::Bench | Subcommand::Test => {
            if !using_default {
                // check if the override is either a valid path or command found on $PATH
                if !(Path::new(&wasix_runner).exists() || which::which(&wasix_runner).is_ok()) {
                    bail!(
                        "failed to find `{}` (specified by ${runner_env_var}) \
                         on the filesytem or in $PATH, you'll want to fix the path or unset \
                         the ${runner_env_var} environment variable before \
                         running this command\n",
                        &wasix_runner
                    );
                }
            } else if which::which(&wasix_runner).is_err() {
                let mut msg = format!(
                    "failed to find `{}` in $PATH, you'll want to \
                     install `{}` before running this command\n",
                    wasix_runner, wasix_runner
                );
                // Because we know what runtime is being used here, we can print
                // out installation information.
                msg.push_str("you can also install through a shell:\n\n");
                msg.push_str("\tcurl https://wasmer.io/install.sh -sSf | bash\n");
                bail!("{}", msg);
            }
            cargo.env("__CARGO_WASIX_RUNNER_SHIM", "1");
            cargo.env(runner_env_var, env::current_exe()?);
        }

        Subcommand::Build | Subcommand::Check | Subcommand::Tree | Subcommand::Fix => {}
    }

    let update_check = internal::UpdateCheck::new(config);
    toolchain::ensure_toolchain(config, is64bit)?;

    // Set the SYSROOT
    if env::var("WASI_SDK_DIR").is_err() {
        if is64bit {
            env::set_var("WASI_SDK_DIR", "/opt/wasix-libc/sysroot64/");
        } else {
            env::set_var("WASI_SDK_DIR", "/opt/wasix-libc/sysroot32/");
        }
    }
    if let Ok(dir) = env::var("WASI_SDK_DIR") {
        config.verbose(|| config.status("WASI_SDK_DIR={}", &dir));
    }

    // Set some flags for RUST
    env::set_var("RUSTFLAGS", "-C target-feature=+atomics");

    // Run the cargo commands
    let build = execute_cargo(&mut cargo, config)?;

    for (wasm, profile, fresh) in build.wasms.iter() {
        // Cargo will always overwrite our `wasm` above with its own internal
        // cache. It's internal cache largely uses hard links.
        //
        // If `fresh` is *false*, then Cargo just built `wasm` and we need to
        // process it. If `fresh` is *true*, then we may have previously
        // processed it. If our previous processing was successful the output
        // was placed at `*.wasi.wasm`, so we use that to overwrite the
        // `*.wasm` file. In the process we also create a `*.rustc.wasm` for
        // debugging.
        //
        // Note that we remove files before renaming and such to ensure that
        // we're not accidentally updating the wrong hard link and such.
        let temporary_rustc = wasm.with_extension("rustc.wasm");
        let temporary_wasi = wasm.with_extension("wasi.wasm");

        drop(fs::remove_file(&temporary_rustc));
        fs::rename(wasm, &temporary_rustc)?;
        if !*fresh || !temporary_wasi.exists() {
            let result = process_wasm(&temporary_wasi, &temporary_rustc, profile, &build, config);
            result.with_context(|| {
                format!("failed to process wasm at `{}`", temporary_rustc.display())
            })?;
        }
        drop(fs::remove_file(wasm));
        fs::hard_link(&temporary_wasi, wasm)
            .or_else(|_| fs::copy(&temporary_wasi, wasm).map(|_| ()))?;
    }

    for run in build.runs.iter() {
        config.status("Running", &format!("`{}`", run.join(" ")));
        let mut cmd = Command::new(&wasix_runner);

        if wasix_runner == "wasmer" {
            cmd.arg("--enable-threads");
        }

        cmd.arg("--")
            .args(run.iter())
            .run()
            .map_err(|e| utils::hide_normal_process_exit(e, config))?;
    }

    update_check.print();
    Ok(())
}

pub const HELP: &str = include_str!("txt/help.txt");

fn print_help() -> ! {
    println!("{}", HELP);
    std::process::exit(0);
}

#[derive(Default, Debug)]
struct CargoBuild {
    // The version of `wasm-bindgen` used in this build, if any.
    wasm_bindgen: Option<String>,
    // The `*.wasm` artifacts we found during this build, in addition to the
    // profile that they were built with and whether or not it was `fresh`
    // during this build.
    wasms: Vec<(PathBuf, Profile, bool)>,
    // executed commands as part of the cargo build
    runs: Vec<Vec<String>>,
    // Configuration we found in the `Cargo.toml` workspace manifest for these
    // builds.
    manifest_config: ManifestConfig,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
struct Profile {
    opt_level: String,
    debuginfo: Option<u32>,
    test: bool,
}

#[derive(serde::Deserialize, Debug, Default)]
#[serde(rename_all = "kebab-case")]
struct ManifestConfig {
    wasm_opt: Option<bool>,
    wasm_name_section: Option<bool>,
    wasm_producers_section: Option<bool>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum CargoMessage {
    CompilerArtifact {
        filenames: Vec<String>,
        package_id: String,
        profile: Profile,
        fresh: bool,
    },
    BuildScriptExecuted,
    RunWithArgs {
        args: Vec<String>,
    },
    BuildFinished,
}

impl CargoBuild {
    fn enable_name_section(&self, profile: &Profile) -> bool {
        profile.debuginfo.is_some() || self.manifest_config.wasm_name_section.unwrap_or(true)
    }

    fn enable_producers_section(&self, profile: &Profile) -> bool {
        profile.debuginfo.is_some() || self.manifest_config.wasm_producers_section.unwrap_or(true)
    }
}

/// Process a wasm file that doesn't use `wasm-bindgen`, using `walrus` instead.
///
/// This will load up the module and do things like:
///
/// * Unconditionally demangle all Rust function names.
/// * Use `profile` to optionally drop debug information
fn process_wasm(
    wasm: &Path,
    temp: &Path,
    profile: &Profile,
    build: &CargoBuild,
    config: &Config,
) -> Result<()> {
    config.verbose(|| {
        config.status("Processing", &temp.display().to_string());
    });

    let mut module = walrus::ModuleConfig::new()
        // If the `debuginfo` is configured then we leave in the debuginfo
        // sections.
        .generate_dwarf(profile.debuginfo.is_some())
        .generate_name_section(build.enable_name_section(profile))
        .generate_producers_section(build.enable_producers_section(profile))
        .strict_validate(false)
        .parse_file(temp)?;

    // Demangle everything so it's got a more readable name since there's
    // no real need to mangle the symbols in wasm.
    for func in module.funcs.iter_mut() {
        if let Some(name) = &mut func.name {
            if let Ok(sym) = rustc_demangle::try_demangle(name) {
                *name = sym.to_string();
            }
        }
    }

    run_wasm_opt(wasm, &module.emit_wasm(), profile, build, config)?;
    Ok(())
}

fn run_wasm_opt(
    wasm: &Path,
    bytes: &[u8],
    profile: &Profile,
    build: &CargoBuild,
    config: &Config,
) -> Result<()> {
    // If debuginfo is enabled, automatically disable `wasm-opt`. It will mess
    // up dwarf debug information currently, so we can't run it.
    //
    // Additionally if no optimizations are enabled, no need to run `wasm-opt`,
    // we're not optimizing.
    if profile.debuginfo.is_some() || profile.opt_level == "0" {
        fs::write(wasm, bytes)?;
        return Ok(());
    }

    // Allow explicitly disabling wasm-opt via `Cargo.toml`.
    if build.manifest_config.wasm_opt == Some(false) {
        fs::write(wasm, bytes)?;
        return Ok(());
    }

    config.status("Optimizing", "with wasm-opt");
    let tempdir = tempfile::TempDir::new_in(wasm.parent().unwrap())
        .context("failed to create temporary directory")?;
    let wasm_opt = config.get_wasm_opt();

    let input = tempdir.path().join("input.wasm");
    fs::write(&input, bytes)?;
    let mut cmd = Command::new(wasm_opt.bin_path());
    cmd.arg(&input);
    cmd.arg(format!("-O{}", profile.opt_level));
    cmd.arg("-o").arg(wasm);
    cmd.arg("--strip-producers");
    cmd.arg("--asyncify");

    if build.enable_name_section(profile) {
        cmd.arg("--debuginfo");
    } else {
        cmd.arg("--strip-debug");
    }

    run_or_download(
        wasm_opt.bin_path(),
        wasm_opt.is_overridden(),
        &mut cmd,
        config,
        || install_wasm_opt(&wasm_opt, config),
    )
    .context("`wasm-opt` failed to execute")?;
    Ok(())
}

/// Executes the `cargo` command, reading all of the JSON that pops out and
/// parsing that into a `CargoBuild`.
fn execute_cargo(cargo: &mut Command, config: &Config) -> Result<CargoBuild> {
    config.verbose(|| config.status("Running", &format!("{:?}", cargo)));
    let mut process = cargo
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to spawn `cargo`")?;
    let mut json = String::new();
    process
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut json)
        .context("failed to read cargo stdout into a json string")?;
    let status = process.wait().context("failed to wait on `cargo`")?;
    utils::check_success(cargo, &status, &[], &[])
        .map_err(|e| utils::hide_normal_process_exit(e, config))?;

    let mut build = CargoBuild::default();
    for line in json.lines() {
        if !line.starts_with('{') {
            println!("{}", line);
            continue;
        }
        match serde_json::from_str(line) {
            Ok(CargoMessage::CompilerArtifact {
                filenames,
                profile,
                package_id,
                fresh,
            }) => {
                let mut parts = package_id.split_whitespace();
                if parts.next() == Some("wasm-bindgen") {
                    if let Some(version) = parts.next() {
                        build.wasm_bindgen = Some(version.to_string());
                    }
                }
                for file in filenames {
                    let file = PathBuf::from(file);
                    if file.extension().and_then(|s| s.to_str()) == Some("wasm") {
                        build.wasms.push((file, profile.clone(), fresh));
                    }
                }
            }
            Ok(CargoMessage::RunWithArgs { args }) => build.runs.push(args),
            Ok(CargoMessage::BuildScriptExecuted) => {}
            Ok(CargoMessage::BuildFinished) => {}
            Err(e) => bail!("failed to parse {}: {}", line, e),
        }
    }

    #[derive(serde::Deserialize)]
    struct CargoMetadata {
        workspace_root: String,
    }

    #[derive(serde::Deserialize)]
    struct CargoManifest {
        package: Option<CargoPackage>,
    }

    #[derive(serde::Deserialize)]
    struct CargoPackage {
        metadata: Option<ManifestConfig>,
    }

    let metadata = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1")
        .capture_stdout()?;
    let metadata = serde_json::from_str::<CargoMetadata>(&metadata)
        .context("failed to deserialize `cargo metadata`")?;
    let manifest = Path::new(&metadata.workspace_root).join("Cargo.toml");
    let toml = fs::read_to_string(&manifest)
        .context(format!("failed to read manifest: {}", manifest.display()))?;
    let toml = toml::from_str::<CargoManifest>(&toml).context(format!(
        "failed to deserialize as TOML: {}",
        manifest.display()
    ))?;

    if let Some(meta) = toml.package.and_then(|p| p.metadata) {
        build.manifest_config = meta;
    }

    Ok(build)
}

/// Attempts to execute `cmd` which is executing `requested`.
///
/// If the execution fails because `requested` isn't found *and* `requested` is
/// the same as the `cache` path provided, then `download` is invoked to
/// download the tool and then we re-execute `cmd` after the download has
/// finished.
///
/// Additionally nice diagnostics and such are printed along the way.
fn run_or_download(
    requested: &Path,
    is_overridden: bool,
    cmd: &mut Command,
    config: &Config,
    download: impl FnOnce() -> Result<()>,
) -> Result<()> {
    // NB: this is explicitly set up so that, by default, we simply execute the
    // command and assume that it exists. That should ideally avoid a few extra
    // syscalls to detect "will things work?"
    config.verbose(|| {
        if requested.exists() {
            config.status("Running", &format!("{:?}", cmd));
        }
    });

    let err = match cmd.run() {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    let rerun_after_download = err.chain().any(|e| {
        // NotFound means we need to clearly download, PermissionDenied may mean
        // that we were racing a download and the file wasn't executable, so
        // fall through and wait for the download to finish to try again.
        if let Some(err) = e.downcast_ref::<io::Error>() {
            return err.kind() == io::ErrorKind::NotFound
                || err.kind() == io::ErrorKind::PermissionDenied;
        }
        false
    });

    // This may have failed for some reason other than `NotFound`, in which case
    // it's a legitimate error. Additionally `requested` may not actually be a
    // path that we download, in which case there's also nothing that we can do.
    if !rerun_after_download || is_overridden {
        return Err(err);
    }

    download()?;
    config.verbose(|| {
        config.status("Running", &format!("{:?}", cmd));
    });
    cmd.run()
}

fn install_wasm_opt(path: &ToolPath, config: &Config) -> Result<()> {
    let tag = "version_109";
    let binaryen_url = |target: &str| {
        let mut url = "https://github.com/WebAssembly/binaryen/releases/download/".to_string();
        url.push_str(tag);
        url.push_str("/binaryen-");
        url.push_str(tag);
        url.push('-');
        url.push_str(target);
        url.push_str(".tar.gz");
        url
    };

    let url = if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        binaryen_url("x86_64-linux")
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        binaryen_url("x86_64-macos")
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        binaryen_url("arm64-macos")
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        binaryen_url("x86_64-windows")
    } else {
        bail!(
            "no precompiled binaries of `wasm-opt` are available for this \
             platform, you'll want to set `$WASM_OPT` to a preinstalled \
             `wasm-opt` command or disable via `wasm-opt = false` in \
             your manifest"
        )
    };

    let (base_path, sub_paths) = path.cache_paths().unwrap();
    download(
        &url,
        &format!("precompiled wasm-opt {}", tag),
        base_path,
        sub_paths,
        config,
    )
}

fn download(
    url: &str,
    name: &str,
    parent: &Path,
    sub_paths: &Vec<PathBuf>,
    config: &Config,
) -> Result<()> {
    // Globally lock ourselves downloading things to coordinate with any other
    // instances of `cargo-wasi` doing a download. This is a bit coarse, but it
    // gets the job done. Additionally if someone else does the download for us
    // then we can simply return.
    let _flock = utils::flock(&config.cache().root().join("downloading"));
    if sub_paths
        .iter()
        .all(|sub_path| parent.join(sub_path).exists())
    {
        return Ok(());
    }

    // Ok, let's actually do the download
    config.status("Downloading", name);
    config.verbose(|| config.status("Get", url));

    let response = utils::get(url)?;
    (|| -> Result<()> {
        fs::create_dir_all(parent)
            .context(format!("failed to create directory `{}`", parent.display()))?;

        let decompressed = flate2::read::GzDecoder::new(response);
        let mut tar = tar::Archive::new(decompressed);
        for entry in tar.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            for sub_path in sub_paths {
                if path.ends_with(sub_path) {
                    let entry_path = parent.join(sub_path);
                    let dir = entry_path.parent().unwrap();
                    if !dir.exists() {
                        fs::create_dir_all(dir)
                            .context(format!("failed to create directory `{}`", dir.display()))?;
                    }
                    entry.unpack(entry_path)?;
                }
            }
        }

        if let Some(missing) = sub_paths
            .iter()
            .find(|sub_path| !parent.join(sub_path).exists())
        {
            bail!("failed to find {:?} in archive", missing);
        }
        Ok(())
    })()
    .context(format!("failed to extract tarball from {}", url))
}
