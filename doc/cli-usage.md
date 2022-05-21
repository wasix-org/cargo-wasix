# CLI Usage

In general `cargo wasix` takes no CLI flags specifically, since it will forward
*everything* to `cargo` under the hood. The subcommand, however, will attempt
to infer flags such as `-v` from the Cargo arguments pass, switching itself to
a verbose output if it looks like Cargo is using a verbose output.

The supported subcommands for `cargo wasix` are:

## `cargo wasix build`

This is the primary subcommand used to build WebAssembly code. This will build
your crate for the `wasm64-wasix` target and run any postprocessing (like
`wasm-bindgen` or `wasm-opt`) over any produced binary.

```
$ cargo wasix build
$ cargo wasix build --release
$ cargo wasix build --lib
$ cargo wasix build --test foo
```

Output `*.wasm` files will be located in `target/wasm64-wasix/debug` for debug
builds or `target/wasm64-wasix/release` for release builds.

## `cargo wasix check`

This subcommands forwards everything to `cargo check`, allowing to perform
quick compile-time checks over your code without actually producing any
`*.wasm` binaries or running any wasm code.

```
$ cargo wasix check
$ cargo wasix check --lib
$ cargo wasix check --tests
```

## `cargo wasix run`

Forwards everything to `cargo run`, and runs all binaries in `wasmer`.
Arguments passed will be forwarded to `wasmer`. Note that it's not
necessary to run `cargo wasix build` before this subcommand. Example usage looks
like:

```
$ cargo wasix run
$ cargo wasix run --release
$ cargo wasix run arg1 arg2
$ cargo wasix run -- --flag-for-wasm-binary
$ cargo wasix run --bin foo
```

> **Note**: Using `cargo wasix` will print `Running ...` twice, that's normal
> but only one wasm binary is actually run.

## `cargo wasix test`

Forwards everything to `cargo test`, and runs all tests in `wasmer`.
Arguments passed will be forwarded to `cargo test`. Note that it's not
necessary to run `cargo wasix build` before executing this command. Example
usage looks like:

```
$ cargo wasix test
$ cargo wasix test my_test_to_run
$ cargo wasix test --lib
$ cargo wasix test --test foo
$ cargo wasix test -- --nocpature
```

You can find some more info about writing tests in the [Rust book's chapter on
writing tests](https://doc.rust-lang.org/book/ch11-01-writing-tests.html).

> **Note**: You'll also want to be sure to consult [WASIX-specific caveats when
testing](testing.md) since there are some gotchas today.

## `cargo wasix fix`

Forwards everything to `cargo fix`, but again with the `--target wasm64-wasix`
option which ensures that the fixes are also applied to wasix-specific code (if
any).

## `cargo wasix version`

This subcommand will print out version information about `cargo wasix` itself.
This is also known as `cargo wasix -V` and `cargo wasix --version`.

```
$ cargo wasix version
$ cargo wasix -V
$ cargo wasix --version
```

## `cargo wasix self clean`

This is an internal management subcommand for `cargo wasix` which completely
clears out the cache that `cargo wasix` uses for itself. This cache includes
various metadata files and downloaded versions of tools like `wasm-opt` and
`wasm-bindgen`.

```
$ cargo wasix self clean
```

## `cargo wasix self update-check`

Checks to see if an update is ready for `cargo-wasix`. If it is then instructions
to acquire the new update will be printed out.

```
$ cargo wasix self update-check
```
