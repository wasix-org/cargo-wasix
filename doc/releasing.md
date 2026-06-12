# Releasing `cargo-wasix`

The GitHub release pipeline is triggered by pushing a version tag that matches
the crate version in `Cargo.toml`.

## Release checklist

1. Update `version` in the root `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. Run the normal pre-release checks locally:
   - `cargo test --all-features`
   - `cargo build --release`
4. Commit and push the release commit to the default branch.
5. Create and push a single version tag in `vX.Y.Z` format.

Example:

```bash
git tag v0.1.26
git push origin v0.1.26
```

## What the workflow does

Pushing the tag runs `.github/workflows/release.yml`, which:

- creates a draft GitHub Release for the tag
- builds release artifacts with `cargo-dist`
- uploads the generated artifacts to the GitHub Release
- publishes the GitHub Release once all artifacts succeed
- publishes `cargo-wasix` to crates.io after the GitHub Release succeeds

## Trusted publishing setup

Before crates.io publishing from GitHub Actions can work, you need to configure
trusted publishing for the `cargo-wasix` crate on crates.io.

Important details:

- Brand-new crates must be published manually once before trusted publishing
  can be enabled. `cargo-wasix` is already on crates.io.
- The crates.io publish job only runs for `vX.Y.Z` tags.
