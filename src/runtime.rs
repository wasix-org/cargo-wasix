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
    if runner.eq_ignore_ascii_case("wasmer") {
        return true;
    }
    Path::new(runner)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("wasmer"))
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
    cmd.args(runtime_invocation_args(runner, runtime_args, run));
}

/// Build the argument list passed to a runtime executable (excluding argv[0]).
pub(crate) fn runtime_invocation_args(
    runner: &str,
    runtime_args: &[String],
    run: &[String],
) -> Vec<String> {
    if is_wasmer_runner(runner) {
        let mut args = vec!["run".to_string()];
        args.extend(runtime_args.iter().cloned());
        if let Some((wasm, guest)) = run.split_first() {
            args.push(wasm.clone());
            if !guest.is_empty() {
                args.push("--".to_string());
                args.extend(guest.iter().cloned());
            }
        }
        args
    } else {
        let mut args = runtime_args.to_vec();
        args.push("--".to_string());
        args.extend(run.iter().cloned());
        args
    }
}

/// Whether automatic runtime defaults are disabled via environment variable.
pub fn run_defaults_disabled() -> bool {
    env::var("CARGO_WASIX_NO_RUN_DEFAULTS").is_ok_and(|v| {
        v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
    })
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
    let manifest_path = manifest_path_from_args(cargo_args)?;
    if let Some(path) = manifest_path {
        let manifest_path = absolute_host_path(Path::new(
            path.to_str()
                .context("manifest path must be valid UTF-8")?,
        ))?;
        return manifest_path
            .parent()
            .context("package manifest path has no parent directory")
            .map(|path| path.to_path_buf());
    }

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
    format!("--volume={}:{}", host_dir.display(), guest_dir)
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

fn manifest_path_from_args(cargo_args: &[OsString]) -> Result<Option<OsString>> {
    let mut iter = cargo_args.iter().peekable();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            continue;
        };

        if text == "--" {
            break;
        }

        if text == "--manifest-path" {
            let Some(value) = iter.next() else {
                bail!("`--manifest-path` requires a path");
            };
            return Ok(Some(value.clone()));
        }

        if let Some(value) = text.strip_prefix("--manifest-path=") {
            if value.is_empty() {
                bail!("`--manifest-path=` requires a path");
            }
            return Ok(Some(OsString::from(value)));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn workspace_with_members(root: &Path) {
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::create_dir_all(root.join("b/src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"
                [workspace]
                members = ["a", "b"]
                resolver = "2"
            "#,
        )
        .unwrap();
        fs::write(
            root.join("a/Cargo.toml"),
            r#"
                [package]
                name = "a"
                version = "1.0.0"
            "#,
        )
        .unwrap();
        fs::write(root.join("a/src/lib.rs"), "").unwrap();
        fs::write(
            root.join("b/Cargo.toml"),
            r#"
                [package]
                name = "b"
                version = "1.0.0"
            "#,
        )
        .unwrap();
        fs::write(root.join("b/src/lib.rs"), "").unwrap();
    }

    #[test]
    fn resolve_manifest_dir_honors_manifest_path() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = env::temp_dir().join(format!(
            "cargo-wasix-resolve-manifest-path-{}",
            std::process::id()
        ));
        workspace_with_members(&root);

        let previous_cwd = env::current_dir().unwrap();
        env::set_current_dir(root.join("b")).unwrap();

        let manifest_path = root.join("a/Cargo.toml");
        let cargo_args = [
            OsString::from("--manifest-path"),
            manifest_path.into_os_string(),
        ];
        let resolved = resolve_manifest_dir(&cargo_args).unwrap();
        let expected = fs::canonicalize(root.join("a")).unwrap();

        env::set_current_dir(previous_cwd).unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_manifest_dir_honors_package_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = env::temp_dir().join(format!(
            "cargo-wasix-resolve-package-flag-{}",
            std::process::id()
        ));
        workspace_with_members(&root);

        let previous_cwd = env::current_dir().unwrap();
        env::set_current_dir(&root).unwrap();

        let cargo_args = [OsString::from("-p"), OsString::from("a")];
        let resolved = resolve_manifest_dir(&cargo_args).unwrap();
        let expected = fs::canonicalize(root.join("a")).unwrap();

        env::set_current_dir(previous_cwd).unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(resolved, expected);
    }

    #[test]
    fn wasmer_runtime_invocation_uses_run_subcommand() {
        let args = runtime_invocation_args(
            "wasmer",
            &["--quiet".to_string()],
            &[
                "foo.wasm".to_string(),
                "guest-arg".to_string(),
                "--color=never".to_string(),
            ],
        );
        assert_eq!(
            args,
            vec![
                "run".to_string(),
                "--quiet".to_string(),
                "foo.wasm".to_string(),
                "--".to_string(),
                "guest-arg".to_string(),
                "--color=never".to_string(),
            ]
        );
    }

    #[test]
    fn wasmer_runtime_invocation_omits_guest_separator_without_guest_args() {
        let args = runtime_invocation_args("wasmer", &[], &["foo.wasm".to_string()]);
        assert_eq!(args, vec!["run".to_string(), "foo.wasm".to_string()]);
    }

    #[test]
    fn non_wasmer_runtime_invocation_uses_legacy_shape() {
        let args = runtime_invocation_args(
            "echo",
            &["--volume".to_string(), ".:/app".to_string()],
            &["foo.wasm".to_string(), "guest".to_string()],
        );
        assert_eq!(
            args,
            vec![
                "--volume".to_string(),
                ".:/app".to_string(),
                "--".to_string(),
                "foo.wasm".to_string(),
                "guest".to_string(),
            ]
        );
    }

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
        assert!(is_wasmer_runner("/usr/bin/wasmer.exe"));
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
    fn parses_manifest_path_flag() {
        let args = [
            OsString::from("--release"),
            OsString::from("--manifest-path"),
            OsString::from("other/Cargo.toml"),
        ];
        assert_eq!(
            manifest_path_from_args(&args).unwrap(),
            Some(OsString::from("other/Cargo.toml"))
        );
    }

    #[test]
    fn parses_long_manifest_path_flag() {
        let args = [OsString::from("--manifest-path=other/Cargo.toml")];
        assert_eq!(
            manifest_path_from_args(&args).unwrap(),
            Some(OsString::from("other/Cargo.toml"))
        );
    }

    #[test]
    fn ignores_manifest_path_after_double_dash() {
        let args = [
            OsString::from("--"),
            OsString::from("--manifest-path"),
            OsString::from("other/Cargo.toml"),
        ];
        assert_eq!(manifest_path_from_args(&args).unwrap(), None);
    }

    #[test]
    fn default_runtime_args_forward_env_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-env");
        fs::create_dir_all(&dir).unwrap();
        let previous_backtrace = env::var("RUST_BACKTRACE").ok();
        let previous_threads = env::var("RUST_TEST_THREADS").ok();
        // SAFETY: guarded by ENV_LOCK and previous values are restored below.
        unsafe { env::set_var("RUST_BACKTRACE", "1") };
        unsafe { env::set_var("RUST_TEST_THREADS", "4") };
        let args = default_runtime_args(&dir).unwrap();
        match previous_backtrace {
            Some(value) => unsafe { env::set_var("RUST_BACKTRACE", value) },
            None => unsafe { env::remove_var("RUST_BACKTRACE") },
        }
        match previous_threads {
            Some(value) => unsafe { env::set_var("RUST_TEST_THREADS", value) },
            None => unsafe { env::remove_var("RUST_TEST_THREADS") },
        }
        assert!(args.contains(&"--env".to_string()));
        assert!(args.iter().any(|arg| arg == "RUST_BACKTRACE=1"));
        assert!(args.iter().any(|arg| arg == "RUST_TEST_THREADS=4"));
    }

    #[test]
    fn default_runtime_args_respect_disable_env_var_yes() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-disabled-yes");
        fs::create_dir_all(&dir).unwrap();
        let previous = env::var("CARGO_WASIX_NO_RUN_DEFAULTS").ok();
        // SAFETY: guarded by ENV_LOCK and previous value is restored below.
        unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", "Yes") };
        let args = default_runtime_args(&dir).unwrap();
        match previous {
            Some(value) => unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", value) },
            None => unsafe { env::remove_var("CARGO_WASIX_NO_RUN_DEFAULTS") },
        }
        assert!(args.is_empty());
    }

    #[test]
    fn default_runtime_args_respect_disable_env_var_true() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-disabled-true");
        fs::create_dir_all(&dir).unwrap();
        let previous = env::var("CARGO_WASIX_NO_RUN_DEFAULTS").ok();
        // SAFETY: guarded by ENV_LOCK and previous value is restored below.
        unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", "TRUE") };
        let args = default_runtime_args(&dir).unwrap();
        match previous {
            Some(value) => unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", value) },
            None => unsafe { env::remove_var("CARGO_WASIX_NO_RUN_DEFAULTS") },
        }
        assert!(args.is_empty());
    }

    #[test]
    fn default_runtime_args_respect_disable_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = env::temp_dir().join("cargo-wasix-runtime-defaults-disabled");
        fs::create_dir_all(&dir).unwrap();
        let previous = env::var("CARGO_WASIX_NO_RUN_DEFAULTS").ok();
        // SAFETY: guarded by ENV_LOCK and previous value is restored below.
        unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", "1") };
        let args = default_runtime_args(&dir).unwrap();
        match previous {
            Some(value) => unsafe { env::set_var("CARGO_WASIX_NO_RUN_DEFAULTS", value) },
            None => unsafe { env::remove_var("CARGO_WASIX_NO_RUN_DEFAULTS") },
        }
        assert!(args.is_empty());
    }
}
