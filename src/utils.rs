use crate::config::Config;
use anyhow::{anyhow, bail, Context, Error, Result};
use fs2::FileExt;
use reqwest::blocking::{Client, Response};
use reqwest::header::USER_AGENT;
use reqwest::Proxy;
use std::fs;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::Duration;
use std::{env, fmt};

/// Make sure a binary exists and runs with the given arguments.
pub fn ensure_binary(command: &str, args: &[&str]) -> Result<(), anyhow::Error> {
    Command::new(command)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .run_verbose()
        .with_context(|| format!("Could not find or execute binary: {command}"))?;
    Ok(())
}

pub trait CommandExt {
    fn as_command_mut(&mut self) -> &mut Command;

    fn capture_stdout(&mut self) -> Result<String> {
        let cmd = self.as_command_mut();
        let output = cmd.stderr(Stdio::inherit()).output_if_success()?;
        let s = String::from_utf8(output.stdout)
            .map_err(|_| anyhow!("process output was not utf-8"))
            .with_context(|| format!("failed to execute {:?}", cmd))?;
        Ok(s)
    }

    fn run_verbose(&mut self) -> Result<()> {
        let c = self.as_command_mut();
        eprintln!(
            "Running {} {}:",
            c.get_program().to_string_lossy(),
            c.get_args()
                .map(|x| x.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        self.run()
    }

    fn run(&mut self) -> Result<()> {
        let cmd = self.as_command_mut();
        cmd.stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stdin(Stdio::inherit())
            .output_if_success()?;
        Ok(())
    }

    fn output_if_success(&mut self) -> Result<Output> {
        let cmd = self.as_command_mut();
        let output = cmd
            .output()
            .with_context(|| format!("failed to create process {:?}", cmd))?;
        check_success(cmd, &output.status, &output.stdout, &output.stderr)?;
        Ok(output)
    }
}

impl CommandExt for Command {
    fn as_command_mut(&mut self) -> &mut Command {
        self
    }
}

pub fn check_success(
    cmd: &Command,
    status: &ExitStatus,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    Err(ProcessError {
        cmd_desc: format!("{:?}", cmd),
        status: *status,
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
        hidden: false,
    }
    .into())
}

pub struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        drop(self.0.unlock());
    }
}

pub fn flock(path: &Path) -> Result<FileLock> {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)
        .context(format!("failed to create directory `{}`", parent.display()))?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock_exclusive()?;
    return Ok(FileLock(file));
}

/// If `Error` is a `ProcessError` and it looks like a "normal exit", then it
/// flags that the `ProcessError` will be hidden.
///
/// Hidden errors won't get printed at the top-level as they propagate outwards
/// since it's trusted that the relevant program printed out all the relevant
/// information.
pub fn hide_normal_process_exit(error: Error, config: &Config) -> Error {
    if config.is_verbose() {
        return error;
    }
    let mut error = match error.downcast::<ProcessError>() {
        Ok(e) => e,
        Err(e) => return e,
    };
    if let Some(code) = error.status.code() {
        // Allowed because suggestions is less readable...
        #[allow(clippy::manual_range_contains)]
        if 0 <= code && code < 128 && error.stdout.is_empty() && error.stderr.is_empty() {
            error.hidden = true;
        }
    }
    error.into()
}

/// Checks if `Error` has been hidden via `hide_normal_process_exit` above.
pub fn normal_process_exit_code(error: &Error) -> Option<i32> {
    let process_error = error.downcast_ref::<ProcessError>()?;
    if !process_error.hidden {
        return None;
    }
    process_error.status.code()
}

#[derive(Debug)]
struct ProcessError {
    status: ExitStatus,
    hidden: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    cmd_desc: String,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to execute {}", self.cmd_desc)?;
        write!(f, "\n    status: {}", self.status)?;
        if !self.stdout.is_empty() {
            let stdout = String::from_utf8_lossy(&self.stdout);
            let stdout = stdout.replace('\n', "\n        ");
            write!(f, "\n    stdout:\n        {}", stdout)?;
        }
        if !self.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&self.stderr);
            let stderr = stderr.replace('\n', "\n        ");
            write!(f, "\n    stderr:\n        {}", stderr)?;
        }
        Ok(())
    }
}

impl std::error::Error for ProcessError {}

/// Finds an HTTP proxy, in order:
/// * `http_proxy` env var
/// * `HTTP_PROXY` env var
/// * `https_proxy` env var
/// * `HTTPS_PROXY` env var
fn get_http_proxy() -> Option<String> {
    ["http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY"]
        .iter()
        .map(env::var)
        .find(|v| v.is_ok())
        .and_then(|v| v.ok())
}

pub fn get(url: &str, timeout: Duration) -> Result<Response> {
    let mut client = Client::builder()
        // This is only for the connect phase.
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout);
    if let Some(proxy_url) = get_http_proxy() {
        if let Ok(proxy) = Proxy::all(&proxy_url) {
            client = client.proxy(proxy);
        }
    }
    let client = client.build()?;

    let response = client
        .get(url)
        .header(
            USER_AGENT,
            format!("cargo-wasix/v{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .context(format!("failed to fetch {}", url))?;
    if !response.status().is_success() {
        bail!(
            "failed to get successful response from {}: {}",
            url,
            response.status()
        );
    }
    Ok(response)
}

/// Recursively copy one filesystem path to another.
///
// Hand-written to prevent an extra dependency.
#[allow(dead_code)]
pub fn copy_path(
    src: &Path,
    target: &Path,
    ignore_existing: bool,
    verbose: bool,
) -> Result<(), anyhow::Error> {
    let meta = src
        .metadata()
        .with_context(|| format!("Could not determine metadata for path '{}'", src.display()))?;
    if meta.is_file() {
        if target.is_file() {
            if ignore_existing {
                Ok(())
            } else {
                bail!(
                    "Could not copy from '{}' to '{}': destination already exists",
                    src.display(),
                    target.display()
                );
            }
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Could not create directory '{}'", parent.display())
                })?;
            }

            let mut input = std::fs::File::open(src)?;
            let mut output = std::fs::File::create(target)?;
            std::io::copy(&mut input, &mut output)?;

            if verbose {
                eprintln!("Copied '{}' to '{}'", src.display(), target.display());
            }
            Ok(())
        }
    } else if meta.is_dir() {
        let iter = std::fs::read_dir(src)
            .with_context(|| format!("Could not list directory '{}'", src.display()))?;
        for res in iter {
            let entry = res?;
            copy_path(
                &entry.path(),
                &target.join(entry.file_name()),
                ignore_existing,
                verbose,
            )?;
        }

        Ok(())
    } else if meta.is_symlink() {
        todo!()
    } else {
        bail!(
            "Could not copy from '{}' to '{}': unknown file type",
            src.display(),
            target.display()
        );
    }
}
