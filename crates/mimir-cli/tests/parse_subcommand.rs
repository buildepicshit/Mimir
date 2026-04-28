//! Integration tests for `mimir-cli parse` — the stdin parse-check
//! subcommand used by the Phase 3.2 LLM-fluency benchmark's corpus
//! validator and by any future parse-smoke workflow.
//!
//! The three contracted exit codes are exercised:
//!
//! - `0` — input parses cleanly.
//! - `1` — parse error (human-readable message on stderr).
//! - `2` — argument misuse (any positional args passed).
//!
//! Invokes the `CARGO_BIN_EXE` binary via a child process so we exercise
//! the real stdin / stdout / exit-code path rather than the library
//! entry point.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};

fn parse_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mimir-cli"))
}

fn run_parse(stdin_input: &str) -> (i32, String, String) {
    let mut child = parse_bin()
        .arg("parse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mimir-cli parse");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(stdin_input.as_bytes())
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait_with_output");
    let code = output.status.code().expect("status has code");
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
    (code, stdout, stderr)
}

#[test]
fn help_flag_exits_zero_with_usage() {
    let output = parse_bin().arg("--help").output().expect("run --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    assert!(stdout.contains("mimir-cli"));
    assert!(stdout.contains("Usage:"));
}

#[test]
fn version_flag_exits_zero_with_version() {
    let output = parse_bin()
        .arg("--version")
        .output()
        .expect("run --version");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn valid_sem_form_exits_zero() {
    let (code, _stdout, stderr) =
        run_parse("(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-15)");
    assert_eq!(code, 0, "clean parse must exit 0; stderr={stderr}");
}

#[test]
fn valid_multi_form_input_exits_zero() {
    // `parse()` accepts multiple top-level forms — the benchmark's
    // corpus is single-form per prompt but this exercises the general
    // path.
    let (code, _stdout, _) = run_parse(
        "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-15)\n\
         (sem @alice @trusts @carol :src @observation :c 0.8 :v 2024-01-16)",
    );
    assert_eq!(code, 0);
}

#[test]
fn truncated_form_exits_one_with_stderr_diagnostic() {
    let (code, stdout, stderr) = run_parse("(sem @alice @knows");
    assert_eq!(code, 1, "parse error must exit 1");
    assert!(
        stdout.is_empty(),
        "parse subcommand emits diagnostics to stderr only; got stdout={stdout:?}"
    );
    assert!(
        stderr.contains("parse error"),
        "stderr must carry a parse-error diagnostic; got: {stderr:?}"
    );
}

#[test]
fn positional_arg_exits_two_with_usage() {
    let child = parse_bin()
        .arg("parse")
        .arg("stray-arg")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(2), "arg misuse must exit 2");
    let stderr = String::from_utf8(output.stderr).expect("utf-8");
    assert!(
        stderr.contains("no positional arguments"),
        "stderr must explain the arg contract; got: {stderr:?}"
    );
}
