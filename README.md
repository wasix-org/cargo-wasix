<div align="center">
  <h1><code>cargo wasix</code></h1>

## Installation

To install this Cargo subcommand, first you'll want to [install
Rust](https://www.rust-lang.org/tools/install) and then you'll execute:

```
$ cargo install cargo-wasix
```

After that you can verify it works via:

```
$ cargo wasix --version
```

## Usage

The `cargo wasix` subcommand is a thin wrapper around `cargo` subcommands,
providing optimized defaults for the `wasm64-wasi` target. Using `cargo wasix`
looks very similar to using `cargo`:

* `cargo wasix build` — build your code in debug mode for the wasix target.

* `cargo wasix build --release` — build the optimized version of your `*.wasm`.

* `cargo wasix run` — execute a binary.

* `cargo wasix test` — run your tests in `wasm64-wasi`.

* `cargo wasix bench` — run your benchmarks in `wasm64-wasi`.

In general, if you'd otherwise execute `cargo foo --flag` you can likely execute
`cargo wasix foo --flag` and everything will "just work" for the `wasm64-wasi`
target.

To give it a spin yourself, try out the hello-world versions of programs!

```
$ cargo new wasix-hello-world
     Created binary (application) `wasix-hello-world` package
$ cd wasix-hello-world
$ cargo wasix run
   Compiling wasix-hello-world v0.1.0 (/code/wasix-hello-world)
    Finished dev [unoptimized + debuginfo] target(s) in 0.15s
     Running `cargo-wasix target/wasm64-wasi/debug/wasix-hello-world.wasm`
     Running `target/wasm64-wasi/debug/wasix-hello-world.wasm`
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
     Running target/wasm64-wasi/debug/deps/wasix_hello_world-9aa88657c21196a1.wasm
     Running `/code/wasix-hello-world/target/wasm64-wasi/debug/deps/wasix_hello_world-9aa88657c21196a1.wasm`

running 1 test
test tests::it_works ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## License

This project is license under the Apache 2.0 license with the LLVM exception.
See [LICENSE] for more details.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be licensed as above, without any additional terms or conditions.