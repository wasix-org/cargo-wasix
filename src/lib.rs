use crate::cache::Cache;
use crate::config::Config;
use crate::utils::CommandExt;
use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::io::BufWriter;
use std::io::Write;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod cache;
mod config;
mod internal;
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
    Run,
    Test,
    Bench,
    Check,
    Fix,
}

fn rmain(config: &mut Config) -> Result<()> {
    config.load_cache()?;

    // skip the current executable and the `wasix` inserted by Cargo
    let mut is64bit = false;
    let mut args = env::args_os().skip(2);
    let subcommand = args.next().and_then(|s| s.into_string().ok());
    let subcommand = match subcommand.as_ref().map(|s| s.as_str()) {
        Some("build") => Subcommand::Build,
        Some("build64") => { is64bit = true; Subcommand::Build }
        Some("run") => Subcommand::Run,
        Some("run64") => { is64bit = true; Subcommand::Run }
        Some("test") => Subcommand::Test,
        Some("test64") => { is64bit = true; Subcommand::Test },
        Some("bench") => Subcommand::Bench,
        Some("bench64") => { is64bit = true; Subcommand::Bench },
        Some("check") => Subcommand::Check,
        Some("check64") => { is64bit = true; Subcommand::Check },
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
        Subcommand::Check => "check",
        Subcommand::Fix => "fix",
        Subcommand::Test => "test",
        Subcommand::Bench => "bench",
        Subcommand::Run => "run",
    });

    // TODO: figure out when these flags are already passed to `cargo` and skip
    // passing them ourselves.
    if is64bit {
        cargo.arg("--target").arg("wasm64-wasmer-wasi");
    } else {
        cargo.arg("--target").arg("wasm32-wasmer-wasi");
    }
    cargo.arg("--message-format").arg("json-render-diagnostics");
    for arg in args {
        if let Some(arg) = arg.to_str() {
            if arg.starts_with("--verbose") || arg.starts_with("-v") {
                config.set_verbose(true);
            }
        }

        cargo.arg(arg);
    }

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
    let (wasix_runner, using_default) = env::var("CARGO_TARGET_WASM32_WASIX_RUNNER")
        .map(|runner_override| (runner_override, false))
        .unwrap_or_else(|_| ("wasmer".to_string(), true));

    match subcommand {
        Subcommand::Run | Subcommand::Bench | Subcommand::Test => {
            if !using_default {
                // check if the override is either a valid path or command found on $PATH
                if !(Path::new(&wasix_runner).exists() || which::which(&wasix_runner).is_ok()) {
                    bail!(
                        "failed to find `{}` (specified by $CARGO_TARGET_WASM32_WASIX_RUNNER) \
                         on the filesytem or in $PATH, you'll want to fix the path or unset \
                         the $CARGO_TARGET_WASM32_WASIX_RUNNER environment variable before \
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
            cargo.env("CARGO_TARGET_WASM32_WASIX_RUNNER", env::current_exe()?);
        }

        Subcommand::Build | Subcommand::Check | Subcommand::Fix => {}
    }

    let update_check = internal::UpdateCheck::new(config);
    install_wasix_target(&config, is64bit)?;

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
    let build = execute_cargo(&mut cargo, &config)?;

    for run in build.runs.iter() {
        config.status("Running", &format!("`{}`", run.join(" ")));
        Command::new(&wasix_runner)
            .arg("--")
            .args(run.iter())
            .run()
            .map_err(|e| utils::hide_normal_process_exit(e, config))?;
    }

    update_check.print();
    Ok(())
}

pub const HELP: &'static str = include_str!("txt/help.txt");
pub const INSTALL: &'static str = include_str!("txt/install-wasix.sh");

fn print_help() -> ! {
    println!("{}", HELP);
    std::process::exit(0);
}

/// Installs the `wasm64-wasi` target into our global cache.
fn install_wasix_target(config: &Config, is64bit: bool) -> Result<()>
{
    // rustup is not itself synchronized across processes so at least attempt to
    // synchronize our own calls. This may not work and if it doesn't we tried,
    // this is largely opportunistic anyway.
    let _lock = utils::flock(&config.cache().root().join("rustup-lock"));

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
    if has_wasix_toolchain == false
    {
        // Read WASIX installation script and run it with SH
        let mut cmd = Command::new("bash")
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut outstdin = cmd.stdin.take().unwrap();
        let mut writer = BufWriter::new(&mut outstdin);
        writer.write_all(INSTALL.as_bytes())?;
        drop(writer);
        drop(outstdin);

        cmd.wait()?;
    }

    // Ok we need to actually check since this is perhaps the first time we've
    // ever checked. Let's ask rustc what its sysroot is and see if it has a
    // wasm64-wasi folder.
    let push_toolchain = env::var("RUSTUP_TOOLCHAIN").unwrap_or("".to_string());
    env::set_var("RUSTUP_TOOLCHAIN", "wasix");
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
            env::set_var("RUSTUP_TOOLCHAIN", push_toolchain);
            return Ok(());
        }
    }
    env::set_var("RUSTUP_TOOLCHAIN", push_toolchain);
    
    bail!(
        "failed to find the `wasm64-wasmer-wasi` target installed, and rustup \
        is also not detected, you'll need to be sure to install the \
        `wasm64-wasi` target before using this command"
    );
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
    utils::check_success(&cargo, &status, &[], &[])
        .map_err(|e| utils::hide_normal_process_exit(e, config))?;

    let mut build = CargoBuild::default();
    for line in json.lines() {
        if !line.starts_with("{") {
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
