use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use predicates::str::is_match;
use std::process::Command;

mod support;

fn cargo_wasix(args: &str) -> Command {
    let mut me = std::env::current_exe().unwrap();
    me.pop();
    me.pop();
    me.push("cargo-wasix");
    me.set_extension(std::env::consts::EXE_EXTENSION);

    let mut cmd = Command::new(&me);
    cmd.arg("wasix");
    for arg in args.split_whitespace() {
        cmd.arg(arg);
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::env::split_paths(&path).collect::<Vec<_>>();
    path.insert(0, me);
    cmd.env("PATH", std::env::join_paths(&path).unwrap());

    cmd
}

#[test]
fn help() {
    cargo_wasix("help").assert().success();
}

#[test]
fn version() {
    cargo_wasix("-V")
        .assert()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        .success();
    cargo_wasix("--version")
        .assert()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        .success();
    cargo_wasix("version")
        .assert()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        .success();
}

#[test]
fn contains_debuginfo() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build").assert().success();
    let bytes = std::fs::read(p.debug_wasm("foo")).context("failed to read wasm")?;
    let sections = custom_sections(&bytes)?;
    assert!(sections.iter().any(|s| s.starts_with(".debug_info")));
    assert!(sections.contains(&"name"));
    Ok(())
}

#[test]
fn strip_debuginfo() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build --release").assert().success();
    let bytes = std::fs::read(p.release_wasm("foo")).context("failed to read wasm")?;
    let sections = custom_sections(&bytes)?;
    assert!(!sections.iter().any(|s| s.starts_with(".debug_info")));
    assert!(sections.contains(&"name"));
    Ok(())
}

#[test]
fn check_works() {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("check").assert().success();
}

#[test]
fn fix_works() {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("fix --allow-no-vcs").assert().success();
}

#[test]
fn rust_names_demangled() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build").assert().success();
    let bytes = std::fs::read(p.debug_wasm("foo")).context("failed to read wasm")?;
    assert_demangled(&bytes)?;

    p.cargo_wasix("build --release").assert().success();
    let bytes = std::fs::read(p.release_wasm("foo")).context("failed to read wasm")?;
    assert_demangled(&bytes)?;
    Ok(())
}

fn assert_demangled(wasm: &[u8]) -> Result<()> {
    let mut saw_name = false;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let mut reader = match payload? {
            wasmparser::Payload::CustomSection {
                name: "name",
                data,
                data_offset,
                ..
            } => wasmparser::NameSectionReader::new(data, data_offset)?,
            _ => continue,
        };
        saw_name = true;

        while !reader.eof() {
            let functions = match reader.read()? {
                wasmparser::Name::Module(_) => continue,
                wasmparser::Name::Function(f) => f,
                wasmparser::Name::Local(_) => continue,
                wasmparser::Name::Unknown { .. } => continue,
            };
            let mut map = functions.get_map()?;
            for _ in 0..map.get_count() {
                let name = map.read()?;
                if name.name.contains("ZN") {
                    panic!("still-mangled name {:?}", name.name);
                }
            }
        }
    }
    assert!(saw_name);
    Ok(())
}

#[test]
fn check_output() -> Result<()> {
    // download the wasix target and get that out of the way
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("check")
        .assert()
        .success();

    // Default output
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("build")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
$",
        )?)
        .success();

    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    // Default verbose output
    p.cargo_wasix("build -v")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Running \"cargo\" .*
.*Compiling foo v1.0.0 .*
.*Running `.*rustc .*`
.*Finished dev .*
.*info: Post-processing WebAssembly files
.*Processing .*foo.rustc.wasm
.*Optimizing with wasm-opt
.*Running .*wasm-opt.*--asyncify.*--debuginfo.*
$",
        )?)
        .success();

    // Incremental verbose output
    p.cargo_wasix("build -v")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Running \"cargo\" .*
.*Fresh foo v1.0.0 .*
.*Finished dev .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    // Incremental non-verbose output
    p.cargo_wasix("build")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Finished dev .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    Ok(())
}

// FIXME: wasm-opt isn't running in release mode, so this test is disabled for now
#[test]
fn check_output_release() -> Result<()> {
    // download the wasix target and get that out of the way
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("build --release")
        .assert()
        .success();

    // Default output
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("build --release")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished release .*
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
$",
        )?)
        .success();

    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    // Default verbose output
    p.cargo_wasix("build -v --release")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Running \"cargo\" .*
.*Compiling foo v1.0.0 .*
.*Running `.*rustc .*`
.*Finished release .*
.*info: Post-processing WebAssembly files
.*Processing .*foo.rustc.wasm
.*Optimizing with wasm-opt
.*Running .*wasm-opt.*
$",
        )?)
        .success();

    // Incremental verbose output
    p.cargo_wasix("build -v --release")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Running \"cargo\" .*
.*Fresh foo v1.0.0 .*
.*Finished release .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    // Incremental non-verbose output
    p.cargo_wasix("build --release")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Finished release .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    Ok(())
}

// Don't understand this test. Why is `my-wasm-bindgen` required ? @theduke
// feign the actual `wasm-bindgen` here because it takes too long to compile
// ignoring this test as I don't think we build for wasm-bindgen in the first place
#[test]
#[ignore]
fn wasm_bindgen() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = '1.0.0'

                [dependencies]
                wasm-bindgen = { path = 'wasm-bindgen' }
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file(
            "wasm-bindgen/Cargo.toml",
            r#"
                [package]
                name = "wasm-bindgen"
                version = '1.0.0'
            "#,
        )
        .file("wasm-bindgen/src/lib.rs", "")
        .build();

    p.cargo_wasix("build -v")
        .env("WASM_BINDGEN", "my-wasm-bindgen")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Running \"cargo\" .*
.*Compiling wasm-bindgen v1.0.0 .*
.*Running `rustc.*`
.*Compiling foo v1.0.0 .*
.*Running `rustc.*`
.*Finished dev .*
error: failed to process wasm at `.*foo.rustc.wasm`

Caused by:
    failed to create process \"my-wasm-bindgen.* \"--keep-debug\".*

Caused by:
    .*
$",
        )?)
        .code(1);

    p.cargo_wasix("build")
        .env("WASM_BINDGEN", "my-wasm-bindgen")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Finished dev .*
error: failed to process wasm at `.*foo.rustc.wasm`

Caused by:
    failed to create process \"my-wasm-bindgen.*

Caused by:
    .*
$",
        )?)
        .code(1);

    p.cargo_wasix("build --release")
        .env("WASM_BINDGEN", "my-wasm-bindgen")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Compiling wasm-bindgen .*
.*Compiling foo .*
.*Finished release .*
error: failed to process wasm at `.*foo.rustc.wasm`

Caused by:
    failed to create process \"my-wasm-bindgen.*

Caused by:
    .*
$",
        )?)
        .code(1);

    Ok(())
}

#[test]
fn run() -> Result<()> {
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();

    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() { println!("hello") }
            "#,
        )
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("hello\n")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();
    Ok(())
}

#[test]
fn run_override_runtime() -> Result<()> {
    support::project()
        .file("src/main.rs", "fn main() {}")
        .override_runtime("wasmer")
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();

    // override fails properly
    support::project()
        .file("src/main.rs", "fn main() {}")
        .override_runtime(
            "command-and-path-that-is-unlikely-to-exist-eac9cb6c-fa25-4487-b07f-38116cc6dade",
        )
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("")
        // error should include this environment variable
        .stderr(is_match("CARGO_TARGET_WASM32_WASMER_WASI_RUNNER")?)
        .failure();

    // override with a working runtime works
    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() { println!("hello") }
            "#,
        )
        .override_runtime("wasmer")
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("hello\n")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();

    let wasmer_path = which::which("wasmer")
        .unwrap()
        .to_string_lossy()
        .to_string();
    // override with a file path works
    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() { println!("hello") }
            "#,
        )
        .override_runtime(&wasmer_path)
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout("hello\n")
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();

    // override is not accidentally using wasmer
    // use the `echo` program to test this
    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() { println!("hello") }
            "#,
        )
        .override_runtime("echo")
        .build()
        .cargo_wasix("run")
        .assert()
        .stdout(is_match("target.wasm32-wasmer-wasi.debug.foo.wasm")?)
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
$",
        )?)
        .success();

    Ok(())
}

#[test]
fn run_forward_args() -> Result<()> {
    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() {
                    println!("{:?}", std::env::args().skip(1).collect::<Vec<_>>());
                }
            "#,
        )
        .build()
        .cargo_wasix("run a -b c")
        .assert()
        .stdout("[\"a\", \"-b\", \"c\", \"--color=never\"]\n")
        .success();
    Ok(())
}

#[test]
fn test() -> Result<()> {
    support::project()
        .file(
            "src/lib.rs",
            r#"
                #[test]
                fn smoke() {}
            "#,
        )
        .build()
        .cargo_wasix("test")
        .assert()
        .stdout(
            "
running 1 test
test smoke ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

",
        )
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished test .*
.*Running unittests src/lib.rs .*wasm.
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*wasm`
$",
        )?)
        .success();
    Ok(())
}

#[test]
fn run_nothing() -> Result<()> {
    support::project()
        .file("src/lib.rs", "")
        .build()
        .cargo_wasix("run")
        .assert()
        .code(101);
    Ok(())
}

#[test]
fn run_many() -> Result<()> {
    support::project()
        .file("src/bin/foo.rs", "")
        .file("src/bin/bar.rs", "")
        .build()
        .cargo_wasix("run")
        .assert()
        .code(101);
    Ok(())
}

#[test]
fn run_one() -> Result<()> {
    support::project()
        .file("src/bin/foo.rs", "fn main() {}")
        .file("src/bin/bar.rs", "")
        .build()
        .cargo_wasix("run --bin foo")
        .assert()
        .code(0);
    Ok(())
}

#[test]
fn test_flags() -> Result<()> {
    support::project()
        .file(
            "src/lib.rs",
            r#"
                #[test]
                fn smoke() {}
            "#,
        )
        .build()
        .cargo_wasix("test -- --nocapture")
        .assert()
        .success();
    Ok(())
}

#[test]
fn run_panic() -> Result<()> {
    support::project()
        .file(
            "src/main.rs",
            r#"
                fn main() {
                    panic!("test");
                }
            "#,
        )
        .build()
        .cargo_wasix("run")
        .assert()
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*Running `.*cargo-wasix .*foo.wasm`
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
.*Running `.*foo.wasm`
thread 'main' panicked at 'test', src.main.rs.*
note: run with `RUST_BACKTRACE=1` .*
",
        )?)
        .failure();
    Ok(())
}

#[test]
fn producers_section() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"

                [package.metadata]
                wasm-producers-section = false
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    // Should be included in debug build
    p.cargo_wasix("build").assert().success();
    let bytes = std::fs::read(p.debug_wasm("foo")).context("failed to read wasm")?;
    assert!(custom_sections(&bytes)?.contains(&"producers"));

    // ... and shouldnt be included in release build w/o debuginfo
    p.cargo_wasix("build --release").assert().success();
    let bytes = std::fs::read(p.release_wasm("foo")).context("failed to read wasm")?;
    assert!(!custom_sections(&bytes)?.contains(&"producers"));
    Ok(())
}

#[test]
fn name_section() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"

                [package.metadata]
                wasm-name-section = false
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    // Should be included in debug build
    p.cargo_wasix("build").assert().success();
    let bytes = std::fs::read(p.debug_wasm("foo")).context("failed to read wasm")?;
    assert!(custom_sections(&bytes)?.contains(&"name"));

    // ... and shouldnt be included in release build w/o debuginfo
    p.cargo_wasix("build --release").assert().success();
    let bytes = std::fs::read(p.release_wasm("foo")).context("failed to read wasm")?;
    assert!(!custom_sections(&bytes)?.contains(&"name"));
    Ok(())
}

fn custom_sections(bytes: &[u8]) -> Result<Vec<&str>> {
    let mut sections = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload? {
            wasmparser::Payload::CustomSection { name, .. } => sections.push(name),
            _ => {}
        }
    }
    Ok(sections)
}

#[test]
fn release_skip_wasm_opt() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"

                [package.metadata]
                wasm-opt = false
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build --release")
        .assert()
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished release .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();
    Ok(())
}

#[test]
fn skip_wasm_opt_if_debug() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"

                [profile.release]
                debug = 1
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build --release")
        .assert()
        .stderr(is_match(
            "^\
.*Compiling foo v1.0.0 .*
.*Finished release .*
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
$",
        )?)
        .success();
    Ok(())
}

#[test]
fn self_bad() {
    cargo_wasix("self")
        .assert()
        .stderr("error: `self` command must be followed by `clean` or `update-check`\n")
        .code(1);
    cargo_wasix("self x")
        .assert()
        .stderr("error: unsupported `self` command: x\n")
        .code(1);
}

// REMOVE ME: The cargo wasix build with workspace doens't work with incompatible crates PR
#[test]
fn workspace_works() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [workspace]
                members = ['a']
            "#,
        )
        .file(
            "a/Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"
            "#,
        )
        .file("a/src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build")
        .assert()
        .stderr(is_match(
            "(?m)^\
.*Compiling foo v1.0.0 .*
.*Finished dev .*
.*info: Post-processing WebAssembly files
.*Optimizing with wasm-opt
$",
        )?)
        .success();
    Ok(())
}

#[test]
fn verbose_build_script_works() -> Result<()> {
    let p = support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"
            "#,
        )
        .file("src/main.rs", "fn main() {}")
        .file(
            "build.rs",
            r#"
                fn main() {
                    println!("hello");
                }
            "#,
        )
        .build();

    p.cargo_wasix("build -vv").assert().success();
    Ok(())
}

#[test]
fn dependencies_check() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = '1.0.0'

                [dependencies]
                mio = "0.8.8"
            "#,
        )
        .build();

    p.cargo_wasix("check")
        .assert()
        .stderr(predicates::str::contains("Found incompatible crates in dependencies (of dependencies): libc, mio\n\nTo fix this add the following to \'Cargo.toml\':\n[patch.crates-io]\nlibc = { git = \"https://github.com/wasix-org/libc\", branch = \"master\" }\nmio = { git = \"https://github.com/wasix-org/mio\" }\n\nYou might have to run `cargo update` to ensure the dependencies are used properly\n"))
        .success();
    Ok(())
}

#[test]
fn dependencies_replaced_are_ignored() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = '1.0.0'

                [dependencies]
                mio = "0.8.8"

                [patch.crates-io]
                mio = { git = "https://github.com/wasix-org/mio" }
                libc = { git = "https://github.com/wasix-org/libc" }
            "#,
        )
        .build();

    p.cargo_wasix("check").assert().stdout("").success();
    Ok(())
}
