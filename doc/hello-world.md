# Hello, World!

Let's see an example of how to run the WASIX version of "Hello, World!". This'll
end up looking very familiar to the Rust version of "Hello, World!" as well.
First up let's create a new project with Cargo:

```
$ cargo new wasix-hello-world
     Created binary (application) `wasix-hello-world` package
$ cd wasix-hello-world
```

This creates a `wasix-hello-world` folder which has a default `Cargo.toml` and
`src/main.rs`. The `main.rs` is the entry point for our program and currently
contains `println!("Hello, world!");`. Everything should be set up for us to
execute (no code needed!) so let's run the code inside of the `wasix-hello-world`
directory:

```
$ cargo wasix run
...
```

Note that you may have to open a new shell for this to ensure `PATH` changes
take effect.

Ok, now that we've got a runtime installed, let's retry executing our binary:

```
$ cargo wasix run
info: downloading component 'rust-std' for 'wasm32-wasix'
info: installing component 'rust-std' for 'wasm32-wasix'
   Compiling wasix-hello-world v0.1.0 (/code/wasix-hello-world)
    Finished dev [unoptimized + debuginfo] target(s) in 0.15s
     Running `/.cargo/bin/cargo-wasix target/wasm32-wasix/debug/wasix-hello-world.wasm`
     Running `target/wasm32-wasix/debug/wasix-hello-world.wasm`
Hello, world!
```

Success! The command first used
[`rustup`](https://github.com/rust-lang/rustup.rs) to install the Rust
`wasm32-wasix` target automatically, and then we executed `cargo` to build the
WebAssembly binary. Finally `wasmer` was used and we can see that `Hello,
world!` was printed by our program.

After this we're off to the races in developing our crate. Be sure to check out
the rest of this book for more information about what you can do with `cargo
wasix`. Additionally if this is your first time using Cargo, be sure to check
out [Cargo's introductory
documentation](https://doc.rust-lang.org/book/ch01-03-hello-cargo.html) as well
