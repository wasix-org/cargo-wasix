//! Points cargo at the WASIX overlay registry.
//!
//! The overlay registry serves WASIX-specific forks of crates and redirects
//! every other request to crates.io, so a project only needs the standard
//! source-replacement stanzas in `.cargo/config.toml`:
//!
//! ```toml
//! [source.crates-io]
//! replace-with = "wasix"
//!
//! [source.wasix]
//! registry = "sparse+https://cargo-registry.wasix.org/"
//! ```
//!
//! This module writes those stanzas into the workspace's cargo config,
//! preserving everything already in the file.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table};

use crate::config::Config;

/// The source name used in the replacement stanzas.
const SOURCE_NAME: &str = "wasix";

/// URL of the WASIX overlay registry.
const REGISTRY_URL: &str = "sparse+https://cargo-registry.wasix.org/";

/// Ensures the workspace's `.cargo/config.toml` routes crates.io through the
/// WASIX overlay registry, creating or minimally editing the file as needed.
/// Existing content — including a crates-io replacement pointing somewhere
/// else, or a custom URL for the `wasix` source — is left untouched.
pub fn ensure_config(config: &Config, workspace_root: &Path) -> Result<()> {
    let path = config_path(workspace_root);

    let existing = match fs::read_to_string(&path) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).context(format!("failed to read `{}`", path.display()));
        }
    };

    match merge(existing.as_deref())? {
        Merge::Unchanged => Ok(()),
        Merge::KeptForeignReplacement(source) => {
            config.warn(&format!(
                "`{}` already replaces crates-io with source `{source}`; leaving it \
                 as-is. WASIX crate forks are served by the overlay registry at \
                 `{REGISTRY_URL}` — route `{source}` through it if builds fail to \
                 resolve WASIX-only versions.",
                path.display(),
            ));
            Ok(())
        }
        Merge::Updated(new_contents) => {
            write_atomically(&path, &new_contents)
                .with_context(|| format!("failed to write `{}`", path.display()))?;
            config.status(
                "Updating",
                &format!(
                    "`{}` to resolve crates through the WASIX registry",
                    path.display()
                ),
            );
            Ok(())
        }
    }
}

/// The cargo config file to edit. Cargo reads the extension-less `.cargo/config`
/// over `config.toml` when both exist, so an existing legacy file must be the
/// one we edit — additions to `config.toml` next to it would be ignored.
fn config_path(workspace_root: &Path) -> PathBuf {
    let dir = workspace_root.join(".cargo");
    let legacy = dir.join("config");
    if legacy.exists() {
        legacy
    } else {
        dir.join("config.toml")
    }
}

#[derive(Debug)]
enum Merge {
    /// The registry stanzas are already in place.
    Unchanged,
    /// crates-io is already replaced with the named third-party source.
    KeptForeignReplacement(String),
    /// The stanzas were merged in; the full new file contents.
    Updated(String),
}

/// Merges the source-replacement stanzas into an existing config (or a fresh
/// one when `existing` is `None`). Pure so it's easy to test: no filesystem.
fn merge(existing: Option<&str>) -> Result<Merge> {
    let mut doc: DocumentMut = match existing {
        Some(contents) => contents.parse().context(
            "existing config is not valid TOML; \
             refusing to touch it (add the WASIX registry stanzas manually)",
        )?,
        None => DocumentMut::new(),
    };

    // A crates-io replacement that points at some other source belongs to the
    // user; report it instead of overwriting.
    if let Some(current) = doc
        .get("source")
        .and_then(|s| s.get("crates-io"))
        .and_then(|t| t.get("replace-with"))
    {
        let Some(current) = current.as_str() else {
            bail!("existing config has an unexpected shape for `source.crates-io.replace-with`");
        };
        if current != SOURCE_NAME {
            return Ok(Merge::KeptForeignReplacement(current.to_string()));
        }
    }

    let mut changed = false;

    let source = table_entry(doc.as_table_mut(), "source")?;
    source.set_implicit(true);

    let crates_io = table_entry(source, "crates-io")?;
    if crates_io.get("replace-with").and_then(Item::as_str) != Some(SOURCE_NAME) {
        crates_io["replace-with"] = toml_edit::value(SOURCE_NAME);
        changed = true;
    }

    // An existing `[source.wasix]` with a different registry URL (e.g. a
    // staging deployment) is a deliberate override; only fill in a missing one.
    let wasix = table_entry(source, SOURCE_NAME)?;
    if wasix.get("registry").is_none() {
        wasix["registry"] = toml_edit::value(REGISTRY_URL);
        changed = true;
    }

    if !changed {
        return Ok(Merge::Unchanged);
    }
    let mut contents = doc.to_string();
    if existing.is_none() {
        contents = concat!(
            "# Resolve crates through the WASIX overlay registry: WASIX-specific\n",
            "# forks come from the registry, everything else from crates.io.\n",
            "# Written by cargo-wasix.\n\n",
        )
        .to_string()
            + &contents;
    }
    Ok(Merge::Updated(contents))
}

/// Gets `table[key]` as a mutable table, inserting an empty one if absent.
fn table_entry<'a>(table: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    let item = table
        .entry(key)
        .or_insert_with(|| Item::Table(Table::new()));
    match item.as_table_mut() {
        Some(table) => Ok(table),
        // e.g. `source = 1` or an inline `crates-io = { ... }`; too unusual to
        // rewrite safely.
        None => bail!("existing config has an unexpected shape for `{key}`"),
    }
}

/// Replaces `path` via a temp file + rename so a half-written config can
/// never be observed.
fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().expect("config path always has a parent");
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(contents.as_bytes())?;
    // Renaming over an existing file can fail on Windows, so remove the
    // target first.
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    tmp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn updated(existing: Option<&str>) -> String {
        match merge(existing).unwrap() {
            Merge::Updated(contents) => contents,
            other => panic!("expected Merge::Updated, got {other:?}"),
        }
    }

    /// The merged output must itself parse and point crates-io at the overlay.
    fn assert_configured(contents: &str) {
        let doc: DocumentMut = contents.parse().unwrap();
        assert_eq!(
            doc["source"]["crates-io"]["replace-with"].as_str(),
            Some(SOURCE_NAME),
            "{contents}"
        );
        assert!(
            doc["source"][SOURCE_NAME]["registry"].as_str().is_some(),
            "{contents}"
        );
    }

    #[test]
    fn fresh_config() {
        let out = updated(None);
        assert_configured(&out);
        assert!(out.starts_with("# Resolve crates through the WASIX overlay registry"));
    }

    #[test]
    fn existing_content_preserved() {
        let out = updated(Some(
            "# my comment\n\
             [build]\n\
             jobs = 4 # inline comment\n\
             \n\
             [target.x86_64-unknown-linux-gnu]\n\
             rustflags = [\"-C\", \"link-arg=-fuse-ld=lld\"]\n",
        ));
        assert_configured(&out);
        assert!(out.contains("# my comment"));
        assert!(out.contains("jobs = 4 # inline comment"));
        assert!(out.contains("link-arg=-fuse-ld=lld"));
    }

    #[test]
    fn already_configured_is_untouched() {
        let existing = "[source.crates-io]\n\
                        replace-with = \"wasix\"\n\
                        \n\
                        [source.wasix]\n\
                        registry = \"sparse+https://cargo-registry.wasix.org/\"\n";
        assert!(matches!(merge(Some(existing)).unwrap(), Merge::Unchanged));
    }

    #[test]
    fn custom_wasix_registry_url_kept() {
        let existing = "[source.crates-io]\n\
                        replace-with = \"wasix\"\n\
                        \n\
                        [source.wasix]\n\
                        registry = \"sparse+https://staging.example.com/\"\n";
        assert!(matches!(merge(Some(existing)).unwrap(), Merge::Unchanged));
    }

    #[test]
    fn missing_wasix_source_is_filled_in() {
        // replace-with = "wasix" without the [source.wasix] stanza breaks
        // cargo outright; complete it.
        let out = updated(Some(
            "[source.crates-io]\n\
             replace-with = \"wasix\"\n",
        ));
        assert_configured(&out);
        assert!(out.contains(REGISTRY_URL));
    }

    #[test]
    fn foreign_replacement_left_alone() {
        let existing = "[source.crates-io]\n\
                        replace-with = \"my-mirror\"\n\
                        \n\
                        [source.my-mirror]\n\
                        registry = \"sparse+https://mirror.example.com/\"\n";
        match merge(Some(existing)).unwrap() {
            Merge::KeptForeignReplacement(source) => assert_eq!(source, "my-mirror"),
            _ => panic!("expected KeptForeignReplacement"),
        }
    }

    #[test]
    fn existing_source_section_extended() {
        let out = updated(Some(
            "[source.vendored-sources]\n\
             directory = \"vendor\"\n",
        ));
        assert_configured(&out);
        assert!(out.contains("directory = \"vendor\""));
    }

    #[test]
    fn invalid_toml_is_refused() {
        let err = merge(Some("[source.crates-io\nnot toml")).unwrap_err();
        assert!(err.to_string().contains("refusing to touch it"), "{err}");
    }

    #[test]
    fn unexpected_shape_is_refused() {
        assert!(merge(Some("source = 1\n")).is_err());
        assert!(merge(Some("[source]\ncrates-io = 1\n")).is_err());
        assert!(merge(Some("[source.crates-io]\nreplace-with = 1\n")).is_err());
    }
}
