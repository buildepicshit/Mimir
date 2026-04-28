//! Binary smoke tests for side-effect-free `mimir-mcp` metadata flags.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

fn mcp_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mimir-mcp"))
}

#[test]
fn help_flag_exits_zero_without_starting_transport() {
    let output = mcp_bin().arg("--help").output().expect("run --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
    assert!(stdout.contains("mimir-mcp"));
    assert!(stdout.contains("Usage:"));
    assert!(
        stderr.is_empty(),
        "--help should not initialize tracing or start MCP transport; got {stderr:?}"
    );
}

#[test]
fn version_flag_exits_zero_without_starting_transport() {
    let output = mcp_bin().arg("--version").output().expect("run --version");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(
        stderr.is_empty(),
        "--version should not initialize tracing or start MCP transport; got {stderr:?}"
    );
}
