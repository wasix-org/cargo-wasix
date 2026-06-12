use anyhow::{Context, Result, bail};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::utils::CommandExt;

const GUEST_PROJECT_DIR: &str = "/project";
const GUEST_TMP_DIR: &str = "/tmp";

/// Returns true when the configured runner is wasmer.
pub fn is_wasmer_runner(runner: &str) -> bool {
    if runner == "wasmer" {
        return true;
    }
    Path::new(runner)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "wasmer")
}

/// Configure a runtime invocation for the given runner.
///
/// Wasmer 7+ expects `wasmer run [OPTIONS] <INPUT> [-- ARGS...]`. Other runners
/// keep the legacy `runner [OPTIONS] -- <INPUT> [ARGS...]` shape.
pub fn configure_runtime_command(
    cmd: &mut Command,
    runner: &str,
    runtime_args: &[String],
    run: &[String],
) {
    if is_wasmer_runner(runner) {
        cmd.arg("run");
        cmd.args(runtime_args);
        if let Some((wasm, guest)) = run.split_first() {
            cmd.arg(wasm);
            if !guest.is_empty() {
                cmd.arg("--");
                cmd.args(guest);
            }
        }
    } else {
        cmd.args(runtime_args);
        cmd.arg("--");
        cmd.args(run);
    }
}

/// Whether automatic runtime defaults are disabled via environment variable.
pub fn run_defaults_disabled() -> bool {
    env::var("CARGO_WASIX_NO_RUN_DEFAULTS").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Build default wasmer/runtime flags for `test` and `bench`.
pub fn default_runtime_args(manifest_dir: &Path) -> Result<Vec<String>> {
    if run_defaults_disabled() {
        return Ok(Vec::new());
    }

    let manifest_dir = absolute_host_path(manifest_dir)?;
    let temp_dir = absolute_host_path(&env::temp_dir())?;

    let mut args = vec![
        volume_arg(&manifest_dir, GUEST_PROJECT_DIR),
        volume_arg(&temp_dir, GUEST_TMP_DIR),
        "--cwd".to_string(),
        GUEST_PROJECT_DIR.to_string(),
    ];

    forward_env(&mut args, "RUST_BACKTRACE");
    forward_env(&mut args, "RUST_TEST_THREADS");

    Ok(args)
}

/// Resolve the active package manifest directory from cargo metadata.
pub fn resolve_manifest_dir(cargo_args: &[OsString]) -> Result<PathBuf> {
    let package_name = package_name_from_args(cargo_args)?;

    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1")
        .capture_stdout()?;
    let metadata = serde_json::from_str::<cargo_metadata::Metadata>(&output)
        .context("failed to deserialize `cargo metadata`")?;

    let package = if let Some(name) = package_name {
        metadata
            .packages
            .iter()
            .find(|pkg| pkg.name == name)
            .with_context(|| format!("package `{name}` not found in workspace"))?
    } else if let Some(root_id) = metadata
        .resolve
        .as_ref()
        .and_then(|resolve| resolve.root.as_ref())
    {
        metadata
            .packages
            .iter()
            .find(|pkg| pkg.id == *root_id)
            .context("root package not found in `cargo metadata`")?
    } else if metadata.packages.len() == 1 {
        &metadata.packages[0]
    } else {
        let cwd = absolute_host_path(Path::new("."))?;
        metadata
            .packages
            .iter()
            .find(|pkg| {
                pkg.manifest_path
                    .parent()
                    .map(|manifest_dir| absolute_host_path(Path::new(manifest_dir.as_str())))
                    .transpose()
                    .ok()
                    .flatten()
                    .is_some_and(|manifest_dir| manifest_dir == cwd)
            })
            .with_context(|| {
                format!(
                    "failed to determine active package from `cargo metadata`; \
                     use `-p` to select one of: {}",
                    metadata
                        .packages
                        .iter()
                        .map(|pkg| pkg.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
    };

    package
        .manifest_path
        .parent()
        .context("package manifest path has no parent directory")
        .map(|path| PathBuf::from(path.as_str()))
}

fn volume_arg(host_dir: &Path, guest_dir: &str) -> String {
    format!(
        "--volume={}:{}",
        host_dir.display(),
        guest_dir
    )
}

fn forward_env(args: &mut Vec<String>, key: &str) {
    if let Ok(value) = env::var(key) {
        args.push("--env".to_string());
        args.push(format!("{key}={value}"));
    }
}

fn absolute_host_path(path: &Path) -> Result<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    Ok(fs::canonicalize(&abs).unwrap_or(abs))
}

fn package_name_from_args(cargo_args: &[OsString]) -> Result<Option<String>> {
    let mut iter = cargo_args.iter().peekable();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            continue;
        };

        if text == "--" {
            break;
        }

        if text == "-p" || text == "--package" {
            let Some(value) = iter.next() else {
                bail!("`{text}` requires a package name");
            };
            return value
                .clone()
                .into_string()
                .map(Some)
                .map_err(|_| anyhow::anyhow!("package name must be valid UTF-8"));
        }

        if let Some(value) = text.strip_prefix("--package=") {
            if value.is_empty() {
                bail!("`--package=` requires a package name");
            }
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_flag() {
        let args = [
            OsString::from("--release"),
            OsString::from("-p"),
            OsString::from("member"),
        ];
        assert_eq!(
            package_name_from_args(&args).unwrap(),
            Some("member".to_string())
        );
    }

    #[test]
    fn parses_long_package_flag() {
        let args = [OsString::from("--package=member")];
        assert_eq!(
            package_name_from_args(&args).unwrap(),
            Some("member".to_string())
        );
    }

    #[test]
    fn ignores_package_after_double_dash() {
        let args = [
            OsString::from("--"),
            OsString::from("-p"),
            OsString::from("member"),
        ];
        assert_eq!(package_name_from_args(&args).unwrap(), None);
    }

    #[test]
    fn recognizes_wasmer_runner() {
        assert!(is_wasmer_runner("wasmer"));
        assert!(is_wasmer_runner("/usr/bin/wasmer"));
        assert!(!is_wasmer_runner("echo"));
    }

    #[test]
    fn default_runtime_args_include_mounts_and_cwd() {
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-test");
        fs::create_dir_all(&dir).unwrap();
        let args = default_runtime_args(&dir).unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("--volume="));
        assert!(joined.contains("/project"));
        assert!(joined.contains("/tmp"));
        assert!(joined.contains("--cwd /project"));
    }

    #[test]
    fn default_runtime_args_respect_disable_env_var() {
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-disabled");
        fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only single-threaded env mutation.
        unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", "1") };
        let args = default_runtime_args(&dir).unwrap();
        unsafe { env::remove_var("CARGO_WASIX_NO_RUN_DEFAULTS") };
        assert!(args.is_empty());
    }
}
