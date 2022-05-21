# Installation

To install `cargo-wasix` you'll first want to [install Rust
itself](https://www.rust-lang.org/tools/install), which you'll need anyway for
building Rust code! Once you've got Rust installed you can install `cargo-wasix`
with:

```
$ cargo install cargo-wasix
```

This will install a precompiled binary for most major platforms or install from
source if we don't have a precompiled binary for your platform.

To verify that your installation works, you can execute:

```
$ cargo wasix --version
```

and that should print both the version number as well as git information about
where the binary was built from.

Now that everything is set, let's build some code for wasix!

## Building from Source

Installing from crates.io via `cargo install cargo-wasix` will install
precompiled binaries. These binaries are built on the `cargo-wasix` repository's
CI and are uploaded to crates.io as part of the publication process. If you'd
prefer to install from source, you can execute this command instead:

```
$ cargo install cargo-wasix-src
```
