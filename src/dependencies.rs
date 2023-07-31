//! Module with check related to the dependencies.

use crate::config::Config;
use crate::utils::{self, CommandExt};
use anyhow::{bail, Context, Result};
use std::collections::hash_map::{self, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Timeout for downling the incompatible crates data from
/// [`KNOWN_INCOMPATIBLE_CRATES_URL`].
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

const KNOWN_INCOMPATIBLE_CRATES_URL: &str =
    "https://raw.githubusercontent.com/wasix-org/cargo-wasix/main/incompatible_crates/data.json";

/// Maximum age before updating the incompatible crate data.
const MAX_CACHE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60); // Roughly 30 days.

/// Known incompatible crate.
#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct IncompatibleCrate {
    /// Name of the crate.
    name: String,
    /// Version(s) that are known to be compatible. If this is `None` we
    /// assume that all versions are incompatible.
    /// For example `>= 0.9` can be used to indicate the version 0.9 gained
    /// support for wasix.
    compatible_versions: Option<cargo_metadata::semver::VersionReq>,
    /// Replacement dependency that supports wasix.
    replacements: Vec<Replacement>,
}

/// Replacement crate for an `IncompatibleCrate`.
#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct Replacement {
    /// Version that needs to be used, in case the latest version isn't
    /// supported.
    version: cargo_metadata::semver::VersionReq,
    /// Git repository to use.
    repo: String,
    /// Git branch to use.
    branch: Option<String>,
}

fn known_incompatible_crates(config: &Config) -> Vec<IncompatibleCrate> {
    match read_known_incompatible_crates(config) {
        Ok(crates) => crates,
        Err(err) => {
            config.print_error(&err.context("not checking known incompatible crates"));
            Vec::new()
        }
    }
}

fn read_known_incompatible_crates(config: &Config) -> Result<Vec<IncompatibleCrate>> {
    let mut path = Config::cache_dir()?;
    path.push("incompatible_crates.json");

    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(ref err) if err.kind() == io::ErrorKind::NotFound => {
            // Don't have to file cached yet, let's do that now.
            return download_known_incompatible_crates(config, &path);
        }
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read '{}'", path.display()))
        }
    };

    let metadata = file.metadata()?;
    if metadata.modified()?.elapsed()? >= MAX_CACHE_AGE {
        return download_known_incompatible_crates(config, &path);
    }

    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("failed to deserialize '{}'", path.display()))
}

fn download_known_incompatible_crates(
    config: &Config,
    path: &Path,
) -> Result<Vec<IncompatibleCrate>> {
    if config.is_offline {
        static INCLUDED_CRATES: &str = include_str!("../incompatible_crates/data.json");
        // NOTE: we don't cache this file as this may be really outdated.
        return serde_json::from_str(INCLUDED_CRATES)
            .with_context(|| format!("failed to deserialize incompatible crates"));
    }

    let url = KNOWN_INCOMPATIBLE_CRATES_URL;

    config.status("Downloading", "known incompatible crates list");
    config.verbose(|| config.status("Get", url));

    let response = utils::get(url, DOWNLOAD_TIMEOUT)?;
    let incompatible_crates = response
        .json()
        .with_context(|| format!("failed to deserialize incompatible crates"))?;

    let dir = path.parent().unwrap_or(path);
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create cache directory '{}'", dir.display()))?;
    let file =
        fs::File::create(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    let mut file = BufWriter::new(file);
    serde_json::to_writer(&mut file, &incompatible_crates).with_context(|| {
        format!(
            "failed to write incompatible crates to '{}'",
            path.display()
        )
    })?;
    file.flush().with_context(|| {
        format!(
            "failed to write incompatible crates to '{}'",
            path.display()
        )
    })?;

    Ok(incompatible_crates)
}

/// Check the dependencies with well-known incompatible crates.
pub fn check(config: &Config, target: &str) -> Result<()> {
    let metadata = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version=1")
        // Only resolve dependencies for our target.
        .arg("--filter-platform")
        .arg(target)
        .capture_stdout()?;
    let metadata = serde_json::from_str::<cargo_metadata::Metadata>(&metadata)
        .context("failed to deserialize `cargo metadata`")?;

    let resolve = metadata
        .resolve
        .as_ref()
        .context("failed to resolve root package")?;
    let root_pkg_id = resolve
        .root
        .as_ref()
        .context("failed to resolve root package")?;

    // First we crate a map of all dependencies, and the dependencies of the
    // dependencies, etc.
    let mut dependencies = HashMap::new();
    let mut to_check = vec![root_pkg_id];
    while let Some(pkg_id) = to_check.pop() {
        let Some(node) = resolve.nodes.iter().find(|n| n.id == *pkg_id) else { continue; };
        for dependency in &node.deps {
            if is_build_dep(&dependency.dep_kinds) {
                continue;
            }

            match dependencies.entry(&dependency.name) {
                hash_map::Entry::Occupied(_) => { /* Already handled. */ }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(&dependency.pkg);
                    to_check.push(&dependency.pkg);
                }
            }
        }
    }

    let mut found_incompatible_crates = Vec::new();
    let known_incompatible_crates = known_incompatible_crates(config);
    for incompatible_crate in &known_incompatible_crates {
        if let Some(pkg_id) = dependencies.get(&incompatible_crate.name) {
            let Some(pkg) = metadata.packages.iter().find(|pkg| pkg.id == **pkg_id) else { continue; };

            if let Some(source) = pkg.source.as_ref() {
                if source.repr.starts_with("git+https://github.com/wasix-org") {
                    // Already using a replacement crate.
                    continue;
                }
            }

            // Filter out versions that are known to compatible.
            if let Some(versions) = &incompatible_crate.compatible_versions {
                if versions.matches(&pkg.version) {
                    continue;
                }
            }

            found_incompatible_crates.push((incompatible_crate, &pkg.version));
        }
    }

    if found_incompatible_crates.is_empty() {
        Ok(())
    } else {
        let mut msg = String::new();
        msg.push_str("Found incompatible crates in dependencies (of dependencies): ");
        for name in found_incompatible_crates.iter().map(|(c, _)| &c.name) {
            write!(&mut msg, "{name}, ")?;
        }
        msg.truncate(msg.len() - 2); // Remove last `, `.

        msg.push_str("\n\nTo fix this add the following to 'Cargo.toml':\n");
        msg.push_str("[patch.crates-io]\n");
        let mut no_replacements = Vec::new();
        for (incompatible_crate, need_version) in found_incompatible_crates {
            let replacement = incompatible_crate
                .replacements
                .iter()
                .find(|replacement| replacement.version.matches(need_version));

            match replacement {
                Some(replacement) => {
                    write!(
                        &mut msg,
                        "{} = {{ git = \"{}\"",
                        incompatible_crate.name, replacement.repo,
                    )?;
                    if let Some(branch) = replacement.branch.as_ref() {
                        writeln!(&mut msg, ", branch = \"{branch}\" }}")?;
                    } else {
                        msg.push_str(" }\n");
                    };
                }
                None => no_replacements.push((
                    &incompatible_crate.name,
                    &incompatible_crate.replacements,
                    need_version,
                )),
            }
        }

        msg.push_str(
            "\nYou might have to run `cargo update` to ensure the dependencies are used properly",
        );

        if !no_replacements.is_empty() {
            msg.push_str("\nNo replacements found for the following dependencies:\n");
            for (name, replacements, version) in no_replacements {
                write!(&mut msg, "* {name} v{version}, ")?;
                if !replacements.is_empty() {
                    msg.push_str("known replacement versions: ");
                    for replacement in replacements {
                        write!(&mut msg, "{}, ", replacement.version)?;
                    }
                    msg.truncate(msg.len() - 2); // Remove last `, `.
                    msg.push('\n');
                } else {
                    msg.push_str("no replacements known\n");
                }
            }
        }

        bail!(msg)
    }
}

fn is_build_dep(dep_kinds: &[cargo_metadata::DepKindInfo]) -> bool {
    use cargo_metadata::DependencyKind::*;
    !dep_kinds
        .iter()
        .any(|d| matches!(d.kind, Normal | Development))
}
