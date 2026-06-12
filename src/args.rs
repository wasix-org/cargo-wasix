use anyhow::{Result, bail};
use std::ffi::OsString;

/// Split user arguments into cargo-forwarded args and wasmer/runtime args.
///
/// Recognizes clang-style `-W,<args>` tokens (comma-separated runtime args).
/// Parsing stops at the first bare `--`; everything from `--` onward goes to
/// cargo unchanged.
pub fn split_cargo_and_wasmer_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(Vec<OsString>, Vec<String>)> {
    let mut cargo_args = Vec::new();
    let mut wasmer_args = Vec::new();
    let mut iter = args.into_iter().peekable();

    while let Some(arg) = iter.next() {
        if arg == "--" {
            cargo_args.push(arg);
            cargo_args.extend(iter);
            break;
        }

        let Some(text) = arg.to_str() else {
            cargo_args.push(arg);
            continue;
        };

        if let Some(rest) = text.strip_prefix("-W,") {
            if rest.is_empty() {
                bail!("`-W,` must be followed by at least one runtime argument");
            }
            let mut saw_segment = false;
            for segment in rest.split(',') {
                if segment.is_empty() {
                    bail!("`-W,` contains an empty comma-separated segment");
                }
                wasmer_args.push(segment.to_string());
                saw_segment = true;
            }
            if !saw_segment {
                bail!("`-W,` must be followed by at least one runtime argument");
            }
            continue;
        }

        if text == "-W" || text.starts_with("-W") && !text.starts_with("-W,") {
            bail!("runtime arguments must use `-W,<args>` syntax (like clang's `-Wl,`)");
        }

        cargo_args.push(arg);
    }

    Ok((cargo_args, wasmer_args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(args: &[&str]) -> (Vec<String>, Vec<String>) {
        let (cargo, wasmer) =
            split_cargo_and_wasmer_args(args.iter().map(|s| OsString::from(*s))).unwrap();
        (
            cargo.into_iter().map(|s| s.into_string().unwrap()).collect(),
            wasmer,
        )
    }

    #[test]
    fn splits_comma_separated_runtime_args() {
        assert_eq!(
            split(&["-W,--mapdir,/tmp:/tmp"]),
            (vec![], vec!["--mapdir".to_string(), "/tmp:/tmp".to_string()])
        );
    }

    #[test]
    fn splits_multiple_runtime_tokens_in_order() {
        assert_eq!(
            split(&["-W,--foo", "--release", "-W,--bar,--baz"]),
            (
                vec!["--release".to_string()],
                vec![
                    "--foo".to_string(),
                    "--bar".to_string(),
                    "--baz".to_string(),
                ]
            )
        );
    }

    #[test]
    fn stops_parsing_at_double_dash() {
        assert_eq!(
            split(&["-W,--foo", "--", "-W,--bar", "guest"]),
            (
                vec!["--".to_string(), "-W,--bar".to_string(), "guest".to_string()],
                vec!["--foo".to_string()]
            )
        );
    }

    #[test]
    fn rejects_bare_w_flag() {
        let err = split_cargo_and_wasmer_args([OsString::from("-W")]).unwrap_err();
        assert!(err.to_string().contains("`-W,<args>`"));
    }

    #[test]
    fn rejects_w_flag_without_comma() {
        let err = split_cargo_and_wasmer_args([OsString::from("-W--foo")]).unwrap_err();
        assert!(err.to_string().contains("`-W,<args>`"));
    }

    #[test]
    fn rejects_empty_w_comma() {
        let err = split_cargo_and_wasmer_args([OsString::from("-W,")]).unwrap_err();
        assert!(err.to_string().contains("at least one runtime argument"));
    }

    #[test]
    fn rejects_empty_segment() {
        let err = split_cargo_and_wasmer_args([OsString::from("-W,--foo,,")]).unwrap_err();
        assert!(err.to_string().contains("empty comma-separated segment"));
    }
}
