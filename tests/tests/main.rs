use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use predicates::str::{contains, is_match};
use regex::Regex;
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
fn rust_names_mangled() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("build").assert().success();
    let bytes = std::fs::read(p.debug_wasm("foo")).context("failed to read wasm")?;
    assert_mangled(&bytes)?;

    p.cargo_wasix("build --release").assert().success();
    let bytes = std::fs::read(p.release_wasm("foo")).context("failed to read wasm")?;
    assert_mangled(&bytes)?;
    Ok(())
}

fn assert_mangled(wasm: &[u8]) -> Result<()> {
    let mut saw_name = false;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let reader = match payload? {
            wasmparser::Payload::CustomSection(sectionreader) => {
                let name = sectionreader.name();
                if name == "name" {
                    wasmparser::NameSectionReader::new(wasmparser::BinaryReader::new(
                        sectionreader.data(),
                        sectionreader.data_offset(),
                    ))
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        saw_name = true;

        for subsection in reader {
            let functions = match subsection? {
                wasmparser::Name::Module { .. } => continue,
                wasmparser::Name::Function(f) => f,
                wasmparser::Name::Local(_) => continue,
                wasmparser::Name::Label(_) => continue,
                wasmparser::Name::Type(_) => continue,
                wasmparser::Name::Table(_) => continue,
                wasmparser::Name::Memory(_) => continue,
                wasmparser::Name::Global(_) => continue,
                wasmparser::Name::Element(_) => continue,
                wasmparser::Name::Data(_) => continue,
                wasmparser::Name::Field(_) => continue,
                wasmparser::Name::Tag(_) => continue,
                wasmparser::Name::Unknown { .. } => continue,
            };
            for name in functions {
                let name = name?;
                // Legacy mangling contains `ZN`; the v0 scheme used by newer
                // toolchains prefixes symbols with `_R`.
                if name.name.contains("ZN") || name.name.starts_with("_R") {
                    return Ok(());
                }
            }
        }
    }
    assert!(saw_name);
    panic!("no mangled names seen");
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
.*info: Post-processing WebAssembly files
.*Processing .*foo.rustc.wasm
.*Optimizing with wasm-opt
.*Running .*wasm-opt.*--debuginfo.*
$",
        )?)
        .success();

    // Incremental verbose output
    p.cargo_wasix("build -v")
        .assert()
        .stdout("")
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    // Incremental non-verbose output
    p.cargo_wasix("build")
        .assert()
        .stdout("")
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    Ok(())
}

fn stderr_after_finished_matches(pattern: &'static str) -> Result<impl predicates::Predicate<str>> {
    let predicate = is_match(pattern)?;
    /* We are intentionally skipping the following 2 warnings:

    info: `cargo` is unavailable for the active toolchain
    info: falling back to \"/home/marxin/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/bin/cargo\"
    info: `cargo` is unavailable for the active toolchain
    info: falling back to \"/home/marxin/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/bin/cargo\"
    warning: `package.edition` is unspecified, defaulting to `2015` while the latest is `2024`
       Compiling foo v1.0.0 (/home/marxin/Programming/wasix-org/cargo-wasix/target/tests/t0)
    warning: unstable feature specified for `-Ctarget-feature`: `atomics`
      |
      = note: this feature is not stably supported; its behavior can change in the future

     */
    let start_re = Regex::new(r"\n\s+Finished ").unwrap();
    Ok(predicate::function(move |stderr: &str| {
        let start = start_re.find(stderr).map(|m| m.start() + 1);

        start
            .map(|offset| predicate.eval(&stderr[offset..]))
            .unwrap_or(false)
    }))
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
.*info: Post-processing WebAssembly files
$",
        )?)
        .success();

    // Incremental non-verbose output
    p.cargo_wasix("build --release")
        .assert()
        .stdout("")
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
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
        .stderr(stderr_after_finished_matches(
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
        .stderr(stderr_after_finished_matches(
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `dev` .*
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
fn wasmer_arg_stripped_on_build() -> Result<()> {
    support::project()
        .file("src/main.rs", "fn main() {}")
        .build()
        .cargo_wasix("build -W,--not-a-cargo-flag")
        .assert()
        .success();
    Ok(())
}

#[test]
fn run_forward_args_with_wasmer_arg() -> Result<()> {
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
        .cargo_wasix("run -W,--quiet a -b c")
        .assert()
        .stdout("[\"a\", \"-b\", \"c\", \"--color=never\"]\n")
        .success();
    Ok(())
}

#[test]
fn run_wasmer_arg_pass_through_echo() -> Result<()> {
    support::project()
        .file("src/main.rs", "fn main() {}")
        .override_runtime("echo")
        .build()
        .cargo_wasix("run -W,--volume,./src:/app,--cwd,/app")
        .assert()
        .stdout(is_match("--volume")?)
        .stdout(is_match("/app")?)
        .success();
    Ok(())
}

#[test]
fn test_reads_fixture_with_defaults() -> Result<()> {
    support::project()
        .file("tests/data.txt", "fixture-data\n")
        .file(
            "src/lib.rs",
            r#"
                #[test]
                fn reads_fixture() {
                    let content = std::fs::read_to_string("tests/data.txt").unwrap();
                    assert_eq!(content, "fixture-data\n");
                }
            "#,
        )
        .build()
        .cargo_wasix("test reads_fixture")
        .assert()
        .stdout(contains("test result: ok. 1 passed; 0 failed"))
        .success();
    Ok(())
}

#[test]
fn bench_reads_fixture_with_defaults() -> Result<()> {
    support::project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "1.0.0"

                [[bench]]
                name = "fixture"
                harness = false
            "#,
        )
        .file("src/lib.rs", "")
        .file("benches/data.txt", "fixture-data\n")
        .file(
            "benches/fixture.rs",
            r#"
                fn main() {
                    let content = std::fs::read_to_string("benches/data.txt").unwrap();
                    assert_eq!(content, "fixture-data\n");
                }
            "#,
        )
        .build()
        .cargo_wasix("bench --bench fixture")
        .assert()
        .success();
    Ok(())
}

#[test]
fn test_reads_fixture_without_defaults_fails() -> Result<()> {
    support::project()
        .file("tests/data.txt", "fixture-data\n")
        .file(
            "src/lib.rs",
            r#"
                #[test]
                fn reads_fixture() {
                    let content = std::fs::read_to_string("tests/data.txt").unwrap();
                    assert_eq!(content, "fixture-data\n");
                }
            "#,
        )
        .build()
        .cargo_wasix("test reads_fixture")
        .env("CARGO_WASIX_NO_RUN_DEFAULTS", "1")
        .assert()
        .failure();
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
        .stdout(contains("test result: ok. 1 passed; 0 failed"))
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `test` .*
.*Running unittests src/lib.rs .*wasm.
.*Doc-tests foo
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
        .stderr(
            // Newer wasmer versions include a thread id: `thread 'main' (1) panicked`.
            contains("Compiling foo v1.0.0").and(is_match(
                r"thread 'main'( \(\d+\))? panicked at src/main.rs",
            )?),
        )
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
        if let wasmparser::Payload::CustomSection(section) = payload? {
            sections.push(section.name())
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
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
        .stderr(stderr_after_finished_matches(
            "^\
.*Finished `release` .*
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
        .stderr(stderr_after_finished_matches(
            "(?m)^\
.*Finished `dev` .*
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
fn registry_config_written() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("check")
        .assert()
        .stderr(contains("to resolve crates through the WASIX registry"))
        .success();

    let config_path = p.root().join(".cargo").join("config.toml");
    let written = std::fs::read_to_string(&config_path)?;
    assert!(written.contains("[source.crates-io]"), "{written}");
    assert!(written.contains("replace-with = \"wasix\""), "{written}");
    assert!(
        written.contains("registry = \"sparse+https://cargo-registry.wasix.org/\""),
        "{written}"
    );

    // A second run finds the config in place and doesn't rewrite it.
    p.cargo_wasix("check")
        .assert()
        .stderr(contains("to resolve crates through the WASIX registry").not())
        .success();
    assert_eq!(std::fs::read_to_string(&config_path)?, written);
    Ok(())
}

#[test]
fn registry_config_write_can_be_disabled() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    let mut cmd = p.cargo_wasix("check");
    cmd.env("CARGO_WASIX_NO_REGISTRY_CONFIG", "1");
    cmd.assert().success();
    assert!(!p.root().join(".cargo").exists());

    // An explicit init still works with the variable set.
    let mut cmd = p.cargo_wasix("init");
    cmd.env("CARGO_WASIX_NO_REGISTRY_CONFIG", "1");
    cmd.assert().success();
    assert!(p.root().join(".cargo").join("config.toml").exists());
    Ok(())
}

#[test]
fn init_writes_config_and_nothing_else() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("init")
        .assert()
        .stderr(contains("to resolve crates through the WASIX registry"))
        .success();

    let written = std::fs::read_to_string(p.root().join(".cargo").join("config.toml"))?;
    assert!(written.contains("replace-with = \"wasix\""), "{written}");
    // No build happened.
    assert!(!p.build_dir().exists());

    // Re-running reports there's nothing to do.
    p.cargo_wasix("init")
        .assert()
        .stderr(contains("the WASIX registry is already configured"))
        .success();
    Ok(())
}

#[test]
fn registry_config_written_on_tree() -> Result<()> {
    // `tree` resolves the dependency graph too, so it must go through the
    // overlay registry like a build would.
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .build();

    p.cargo_wasix("tree").assert().success();

    let written = std::fs::read_to_string(p.root().join(".cargo").join("config.toml"))?;
    assert!(written.contains("replace-with = \"wasix\""), "{written}");
    Ok(())
}

#[test]
fn registry_config_preserves_existing() -> Result<()> {
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(
            ".cargo/config.toml",
            "# keep me\n\
             [build]\n\
             jobs = 2\n",
        )
        .build();

    p.cargo_wasix("check").assert().success();

    let written = std::fs::read_to_string(p.root().join(".cargo").join("config.toml"))?;
    assert!(written.contains("# keep me"), "{written}");
    assert!(written.contains("jobs = 2"), "{written}");
    assert!(written.contains("replace-with = \"wasix\""), "{written}");
    Ok(())
}

#[test]
fn registry_config_respects_existing_replacement() -> Result<()> {
    let existing = "[source.crates-io]\n\
                    replace-with = \"my-mirror\"\n\
                    \n\
                    [source.my-mirror]\n\
                    registry = \"sparse+https://mirror.invalid/\"\n";
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(".cargo/config.toml", existing)
        .build();

    p.cargo_wasix("check")
        .assert()
        .stderr(contains(
            "already replaces crates-io with source `my-mirror`",
        ))
        .success();

    let written = std::fs::read_to_string(p.root().join(".cargo").join("config.toml"))?;
    assert_eq!(written, existing);
    Ok(())
}
#[test]
fn wasixcc_env_vars_set() -> Result<()> {
    // The wasixcc tools are pointed at via target-scoped variables (the ones
    // the `cc` crate checks first), so a host compiler in the generic CC/CXX
    // never blocks them and is never modified.
    if which::which("wasixcc").is_err() {
        eprintln!("SKIPPED wasixcc_env_vars_set: wasixcc not on PATH");
        return Ok(());
    }

    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(
            "build.rs",
            r#"
                fn main() {
                    for var in [
                        "CC_wasm32_wasmer_wasi",
                        "CXX_wasm32_wasmer_wasi",
                        "AR_wasm32_wasmer_wasi",
                        "CC",
                    ] {
                        // Changed env vars must re-run this script, or later
                        // builds replay the first run's cached warnings.
                        println!("cargo:rerun-if-env-changed={}", var);
                        match std::env::var(var) {
                            Ok(v) => println!("cargo:warning={} is set to: {}", var, v),
                            Err(_) => println!("cargo:warning={} is not set", var),
                        }
                    }
                    println!("cargo:rerun-if-env-changed=CC_wasm32-wasmer-wasi");
                }
            "#,
        )
        .build();

    // A generic host CC in the environment must not block the target-scoped
    // variables (and must be passed through untouched).
    let mut cmd = p.cargo_wasix("build");
    cmd.env("CC", "host-cc-do-not-use");
    let output = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("CC_wasm32_wasmer_wasi is set to: wasixcc"),
        "{stderr}"
    );
    assert!(
        stderr.contains("CXX_wasm32_wasmer_wasi is set to: wasixcc++"),
        "{stderr}"
    );
    assert!(
        stderr.contains("AR_wasm32_wasmer_wasi is set to: wasixar"),
        "{stderr}"
    );
    assert!(
        stderr.contains("CC is set to: host-cc-do-not-use"),
        "{stderr}"
    );

    // A user-set target-scoped variable wins over the wasixcc default.
    let mut cmd = p.cargo_wasix("build");
    cmd.env("CC_wasm32_wasmer_wasi", "my-custom-wasix-cc");
    let output = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("CC_wasm32_wasmer_wasi is set to: my-custom-wasix-cc"),
        "{stderr}"
    );

    // The dashed spelling (checked first by `cc`) also counts as a user
    // override: the underscored variant must not be set on top of it.
    let mut cmd = p.cargo_wasix("build");
    cmd.env("CC_wasm32-wasmer-wasi", "my-dashed-wasix-cc");
    let output = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("CC_wasm32_wasmer_wasi is not set"),
        "{stderr}"
    );

    Ok(())
}

#[test]
fn wasixcc_pic_and_exceptions_for_dl_target() -> Result<()> {
    // Test that WASIXCC_PIC and WASIXCC_WASM_EXCEPTIONS are set for -dl target
    let p = support::project()
        .file("src/main.rs", "fn main() {}")
        .file(
            "build.rs",
            r#"
                fn main() {
                    for var in ["WASIXCC_PIC", "WASIXCC_WASM_EXCEPTIONS"] {
                        println!("cargo:rerun-if-env-changed={}", var);
                        if let Ok(v) = std::env::var(var) {
                            println!("cargo:warning={} is set to: {}", var, v);
                        }
                    }
                }
            "#,
        )
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = '1.0.0'

                [package.metadata]
                dl = true
            "#,
        )
        .build();

    let output = p.cargo_wasix("build").assert().success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);

    // For -dl target, WASIXCC_PIC should be set
    assert!(
        stderr.contains("WASIXCC_PIC is set to: 1"),
        "Expected WASIXCC_PIC=1 for -dl target, stderr:\n{}",
        stderr
    );
    // The Rust toolchain builds in the legacy EH configuration, so wasixcc
    // must too (its own default is exnref).
    if which::which("wasixcc").is_ok() {
        assert!(
            stderr.contains("WASIXCC_WASM_EXCEPTIONS is set to: legacy"),
            "Expected WASIXCC_WASM_EXCEPTIONS=legacy, stderr:\n{}",
            stderr
        );

        // Even a value from the environment is overridden: mismatched EH
        // configurations must not reach the build.
        let mut cmd = p.cargo_wasix("build");
        cmd.env("WASIXCC_WASM_EXCEPTIONS", "exnref");
        let output = cmd.assert().success();
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(
            stderr.contains("WASIXCC_WASM_EXCEPTIONS is set to: legacy"),
            "Expected the exnref value from the environment to be overridden, stderr:\n{}",
            stderr
        );
    }

    Ok(())
}
