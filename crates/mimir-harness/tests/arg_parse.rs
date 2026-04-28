//! Transparent harness argument parsing tests.

use mimir_harness::{parse_launch_args, HarnessError};

fn parse(args: &[&str]) -> Result<mimir_harness::LaunchPlan, HarnessError> {
    parse_launch_args(args.iter().copied(), "test-session")
}

#[test]
fn agent_arguments_pass_through_unchanged() -> Result<(), Box<dyn std::error::Error>> {
    let plan = parse(&["claude", "--r"])?;

    assert_eq!(plan.agent(), "claude");
    assert_eq!(plan.agent_args(), ["--r"]);
    assert_eq!(plan.session_id(), "test-session");
    assert_eq!(plan.project(), None);
    Ok(())
}

#[test]
fn mimir_flags_are_consumed_before_agent_only() -> Result<(), Box<dyn std::error::Error>> {
    let plan = parse(&[
        "--project",
        "buildepicshit/Mimir",
        "codex",
        "--project",
        "child-native-project",
    ])?;

    assert_eq!(plan.project(), Some("buildepicshit/Mimir"));
    assert_eq!(plan.agent(), "codex");
    assert_eq!(plan.agent_args(), ["--project", "child-native-project"]);
    Ok(())
}

#[test]
fn delimiter_allows_agent_after_mimir_flags() -> Result<(), Box<dyn std::error::Error>> {
    let plan = parse(&["--project", "Mimir", "--", "claude", "--r"])?;

    assert_eq!(plan.project(), Some("Mimir"));
    assert_eq!(plan.agent(), "claude");
    assert_eq!(plan.agent_args(), ["--r"]);
    Ok(())
}

#[test]
fn command_spec_injects_stable_session_environment() -> Result<(), Box<dyn std::error::Error>> {
    let plan = parse(&["--project", "Mimir", "codex", "--model", "gpt-5.4"])?;
    let spec = plan.child_command_spec();

    assert_eq!(spec.program(), "codex");
    assert_eq!(spec.args(), ["--model", "gpt-5.4"]);
    assert_eq!(
        spec.env(),
        [
            ("MIMIR_AGENT", "codex"),
            ("MIMIR_BOOTSTRAP", "auto"),
            ("MIMIR_HARNESS", "1"),
            ("MIMIR_LIBRARIAN_AFTER_CAPTURE", "off"),
            ("MIMIR_PROJECT", "Mimir"),
            ("MIMIR_SESSION_ID", "test-session"),
        ]
    );
    Ok(())
}

#[test]
fn missing_agent_is_an_argument_error() -> Result<(), Box<dyn std::error::Error>> {
    let err = match parse(&[]) {
        Err(err) => err,
        Ok(plan) => return Err(format!("missing agent should fail; got {plan:?}").into()),
    };
    assert!(matches!(err, HarnessError::MissingAgent));
    Ok(())
}

#[test]
fn missing_project_value_is_an_argument_error() -> Result<(), Box<dyn std::error::Error>> {
    let err = match parse(&["--project"]) {
        Err(err) => err,
        Ok(plan) => return Err(format!("missing project value should fail; got {plan:?}").into()),
    };
    assert!(matches!(
        err,
        HarnessError::MissingFlagValue { flag } if flag == "--project"
    ));
    Ok(())
}

#[test]
fn unknown_mimir_flag_before_agent_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let err = match parse(&["--unknown", "claude"]) {
        Err(err) => err,
        Ok(plan) => return Err(format!("unknown flag should fail; got {plan:?}").into()),
    };
    assert!(matches!(
        err,
        HarnessError::UnknownFlag { flag } if flag == "--unknown"
    ));
    Ok(())
}
