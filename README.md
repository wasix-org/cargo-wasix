# cargo-wasix

A cargo subcommand that wraps regular cargo commands for compiling Rust code
to `wasix`, a superset of Webassembly `wasi` with additional functionality.

See [wasix.org](https://wasix.org) for more.

## Installation

> **Installation requires**
> ‣ [Rust](https://www.rust-lang.org/tools/install) installed via [rustup](https://rustup.rs/)

### Information

This subcommand is available on [crates.io](https://crates.io/crates/cargo-wasix)

Available for platforms:

- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

You can install this Cargo subcommand via:

### Cargo Install

```shell
$ cargo install cargo-wasix
```

### Cargo Binstall

> [`cargo binstall`](https://github.com/cargo-bins/cargo-binstall) provides a low-complexity mechanism for installing rust binaries as an alternative to building from source (via `cargo install`) or manually downloading packages.

> Uses pre-built binaries.

```shell
$ cargo binstall cargo-wasix
```

### Install from install script

> Uses pre-built binaries.

#### For Linux and macOS

```shell
$ curl --proto '=https' --tlsv1.2 -LsSf https://github.com/wasix-org/cargo-wasix/releases/latest/download/cargo-wasix-installer.sh | sh
```

#### For Windows

```shell
irm https://github.com/wasix-org/cargo-wasix/releases/latest/download/cargo-wasix-installer.ps1 | iex
```

### Verify Installation

```shell
$ cargo wasix --version
```

## Usage

The `cargo wasix` subcommand is a thin wrapper around `cargo` subcommands,
providing optimized defaults for the `wasm32-wasmer-wasi` target. Using `cargo wasix`
looks very similar to using `cargo`:

- `cargo wasix build` — build your code in debug mode for the wasix target.

- `cargo wasix build --release` — build the optimized version of your `*.wasm`.

- `cargo wasix run` — execute a binary.

- `cargo wasix test` — run your tests in `wasm32-wasmer-wasi`.

- `cargo wasix bench` — run your benchmarks in `wasm32-wasmer-wasi`.

In general, if you'd otherwise execute `cargo foo --flag` you can likely execute
`cargo wasix foo --flag` and everything will "just work" for the `wasm32-wasmer-wasi`
target.

To give it a spin yourself, try out the hello-world versions of programs!

```
$ cargo new wasix-hello-world
     Created binary (application) `wasix-hello-world` package
$ cd wasix-hello-world
$ cargo wasix run
   Compiling wasix-hello-world v0.1.0 (/code/wasix-hello-world)
    Finished dev [unoptimized + debuginfo] target(s) in 0.15s
     Running `cargo-wasix target/wasm32-wasmer-wasi/debug/wasix-hello-world.wasm`
     Running `target/wasm32-wasmer-wasi/debug/wasix-hello-world.wasm`
Hello, world!
```

Or a library with some tests:

```
$ cargo new wasix-hello-world --lib
     Created library `wasix-hello-world` package
$ cd wasix-hello-world
$ cargo wasix test
   Compiling wasix-hello-world v0.1.0 (/code/wasix-hello-world)
    Finished dev [unoptimized + debuginfo] target(s) in 0.19s
     Running target/wasm32-wasmer-wasi/debug/deps/wasix_hello_world-9aa88657c21196a1.wasm
     Running `/code/wasix-hello-world/target/wasm32-wasmer-wasi/debug/deps/wasix_hello_world-9aa88657c21196a1.wasm`

running 1 test
test tests::it_works ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## License

This project is license under the Apache 2.0 license with the LLVM exception.
See [LICENSE](https://github.com/wasix-org/cargo-wasix/blob/main/LICENSE) for more details.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be licensed as above, without any additional terms or conditions.
