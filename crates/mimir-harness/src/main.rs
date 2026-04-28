//! Entry point for the `mimir` transparent agent harness.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mimir_harness::{
    capture_session_drafts, generate_session_id, list_checkpoint_notes, prepare_launch_plan,
    prepare_remote_sync_plan, render_draft_next, render_draft_show, render_draft_triage,
    render_drafts_list, render_drafts_status, render_launch_banner, render_memory_context,
    render_memory_explain, render_memory_health, render_memory_list, render_memory_show,
    render_operator_status, render_project_doctor, render_remote_dry_run,
    render_remote_restore_drill_dry_run, render_remote_restore_drill_report, render_remote_status,
    render_remote_sync_report, run_child, run_remote_restore_drill, run_remote_sync,
    submit_memory_revoke_request, write_checkpoint_note, CheckpointNoteMetadata, HarnessError,
    RemoteSyncDirection,
};
use mimir_librarian::DraftState;

const USAGE: &str = "\
mimir — transparent Mimir agent launch harness.

Usage:
    mimir [mimir flags] <agent> [agent args...]
    mimir checkpoint [--title <title>] [--list] [text...]
    mimir status [options]
    mimir doctor [options]
    mimir health [options]
    mimir context [options]
    mimir memory <list|show|explain|revoke> [options]
    mimir drafts <status|list|show|next|skip|quarantine> [options]
    mimir hook-context
    mimir config init [options]
    mimir remote <status|push|pull|drill> [options]
    mimir setup-agent <status|doctor|install|remove> --agent <claude|codex> --scope <project|user> [options]

Mimir flags:
    --project <name>    Override detected project/scope label.
    -h, --help          Show this help.
    --version           Show version.

Arguments after <agent> are passed to the child unchanged.
On first run, launch still enters the requested agent; Mimir exposes
bootstrap/config state through MIMIR_* environment variables and a
structured session capsule.

Examples:
    mimir claude --r
    mimir codex --model gpt-5.4
    mimir --project Mimir claude --r
    mimir checkpoint --title \"Handoff\" \"The librarian now processes captured drafts.\"
    mimir status
    mimir doctor
    mimir health
    mimir context --limit 12
    mimir memory list --limit 20
    mimir memory explain --id @__mem_0
    mimir drafts list --state pending
    mimir hook-context
    mimir config init --operator hasnobeef --organization buildepicshit --remote-url git@github.com:org/mimir-memory.git
    mimir remote status
    mimir remote push --dry-run
    mimir remote drill --dry-run
    mimir setup-agent doctor --agent codex --scope project
    mimir setup-agent install --agent codex --scope project --from \"$MIMIR_AGENT_SETUP_DIR\"
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.as_slice(), [flag] if flag == "-h" || flag == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.len() == 1 && args[0] == "--version" {
        println!("mimir {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|arg| arg == "checkpoint") {
        return run_checkpoint_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "status") {
        return run_status_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "doctor") {
        return run_doctor_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "health") {
        return run_health_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "context") {
        return run_context_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "memory") {
        return run_memory_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "drafts") {
        return run_drafts_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "hook-context") {
        return run_hook_context_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "config") {
        return run_config_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "remote") {
        return run_remote_command(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "setup-agent") {
        return run_setup_agent_command(&args[1..]);
    }

    let session_id = generate_session_id();
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let plan = match prepare_launch_plan(args, session_id, &current_dir, &env) {
        Ok(plan) => plan,
        Err(err) => return report_argument_error(&err),
    };

    eprint!("{}", render_launch_banner(&plan));

    match run_child(&plan) {
        Ok(status) => {
            if let Err(err) =
                capture_session_drafts(&plan, status.code(), std::time::SystemTime::now())
            {
                eprintln!("mimir: {err}");
            }
            exit_code_from_status(status.code())
        }
        Err(err) => report_spawn_error(&err),
    }
}

fn run_status_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", status_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match status_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn status_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = ProjectCommandOptions::parse(args, cwd, status_usage)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    render_operator_status(&options.project_root, &env)
}

fn status_usage() -> String {
    "Usage: mimir status [--project-root <dir>] [--config <file>]".to_string()
}

fn run_doctor_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", doctor_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match doctor_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn doctor_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = ProjectCommandOptions::parse(args, cwd, doctor_usage)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    render_project_doctor(&options.project_root, &env)
}

fn doctor_usage() -> String {
    "Usage: mimir doctor [--project-root <dir>] [--config <file>]".to_string()
}

fn run_health_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", health_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match health_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn health_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = ProjectCommandOptions::parse(args, cwd, health_usage)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    render_memory_health(&options.project_root, &env)
}

fn health_usage() -> String {
    "Usage: mimir health [--project-root <dir>] [--config <file>]".to_string()
}

fn run_context_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", context_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match context_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn context_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = ContextCommandOptions::parse(args, cwd)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    render_memory_context(&options.project_root, &env, options.limit)
}

fn context_usage() -> String {
    "Usage: mimir context [--project-root <dir>] [--config <file>] [--limit <records>]".to_string()
}

fn run_memory_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", memory_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match memory_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn memory_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = MemoryCommandOptions::parse(args, cwd)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    match options.action {
        MemoryCommandAction::List => render_memory_list(
            &options.project_root,
            &env,
            options.limit,
            options.kind.as_deref(),
        ),
        MemoryCommandAction::Show => render_memory_show(
            &options.project_root,
            &env,
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory show requires --id <memory-id>".to_string(),
                })?,
        ),
        MemoryCommandAction::Explain => render_memory_explain(
            &options.project_root,
            &env,
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory explain requires --id <memory-id>".to_string(),
                })?,
        ),
        MemoryCommandAction::Revoke => submit_memory_revoke_request(
            &options.project_root,
            &env,
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory revoke requires --id <memory-id>".to_string(),
                })?,
            options
                .reason
                .as_deref()
                .ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory revoke requires --reason <reason>".to_string(),
                })?,
            options.dry_run,
        ),
    }
}

fn memory_usage() -> String {
    "Usage: mimir memory <list|show|explain|revoke> [--project-root <dir>] [--config <file>] [--limit <records>] [--kind all|sem|epi|pro|inf] [--id <memory-id>] [--reason <text>] [--dry-run]"
        .to_string()
}

fn run_drafts_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", drafts_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match drafts_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn drafts_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = DraftsCommandOptions::parse(args, cwd)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    match options.action {
        DraftsCommandAction::Status => {
            render_drafts_status(&options.project_root, &env, options.drafts_dir.as_deref())
        }
        DraftsCommandAction::List => render_drafts_list(
            &options.project_root,
            &env,
            options.drafts_dir.as_deref(),
            options.state.unwrap_or(DraftState::Pending),
        ),
        DraftsCommandAction::Next => render_draft_next(
            &options.project_root,
            &env,
            options.drafts_dir.as_deref(),
            options.state.unwrap_or(DraftState::Pending),
        ),
        DraftsCommandAction::Skip => render_draft_triage(
            &options.project_root,
            &env,
            options.drafts_dir.as_deref(),
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                    message: "drafts skip requires a draft id".to_string(),
                })?,
            options.state.unwrap_or(DraftState::Pending),
            DraftState::Skipped,
            options
                .reason
                .as_deref()
                .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                    message: "drafts skip requires --reason".to_string(),
                })?,
        ),
        DraftsCommandAction::Quarantine => render_draft_triage(
            &options.project_root,
            &env,
            options.drafts_dir.as_deref(),
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                    message: "drafts quarantine requires a draft id".to_string(),
                })?,
            options.state.unwrap_or(DraftState::Pending),
            DraftState::Quarantined,
            options
                .reason
                .as_deref()
                .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                    message: "drafts quarantine requires --reason".to_string(),
                })?,
        ),
        DraftsCommandAction::Show => render_draft_show(
            &options.project_root,
            &env,
            options.drafts_dir.as_deref(),
            options
                .id
                .as_deref()
                .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                    message: "drafts show requires a draft id".to_string(),
                })?,
            options.state,
        ),
    }
}

fn drafts_usage() -> String {
    "Usage: mimir drafts <status|list|show|next|skip|quarantine> [ID] [--reason <text>] [--state pending|processing|accepted|skipped|failed|quarantined] [--project-root <dir>] [--config <file>] [--drafts-dir <dir>]"
        .to_string()
}

fn run_hook_context_command(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("mimir: hook-context does not accept arguments");
        return ExitCode::from(2);
    }
    let env: BTreeMap<String, String> = std::env::vars().collect();
    print!("{}", hook_context_output(&env));
    ExitCode::SUCCESS
}

fn run_config_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", config_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match config_command(args, &cwd) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn config_command(args: &[String], cwd: &Path) -> Result<String, String> {
    let Some(action) = args.first() else {
        return Err(config_usage());
    };
    match action.as_str() {
        "init" => config_init_command(&args[1..], cwd),
        "-h" | "--help" => Err(config_usage()),
        unknown => Err(format!("unknown config action `{unknown}`; expected init")),
    }
}

fn config_usage() -> String {
    "Usage: mimir config init [--path <file>|--project-root <dir>] [--data-root <dir>] [--drafts-dir <dir>] [--operator <id>] [--organization <id>] [--remote-url <url>] [--remote-kind git|service] [--remote-branch <branch>] [--librarian-after-capture off|defer|archive_raw|process] [--dry-run]"
        .to_string()
}

fn run_remote_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", remote_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match remote_command(args, &cwd, std::env::vars().collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn remote_command(
    args: &[String],
    cwd: &Path,
    mut env: BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let options = RemoteCommandOptions::parse(args, cwd)?;
    if options.project_root_source.is_explicit() || options.config_path.is_some() {
        remove_inherited_project_env(&mut env);
    }
    if let Some(config_path) = &options.config_path {
        env.insert(
            "MIMIR_CONFIG_PATH".to_string(),
            config_path.display().to_string(),
        );
    }
    match options.action {
        RemoteCommandAction::Status => {
            render_remote_status(&options.project_root, &env, options.refresh)
        }
        RemoteCommandAction::Push => {
            if options.dry_run {
                render_remote_dry_run(&options.project_root, &env, RemoteSyncDirection::Push)
            } else {
                let plan = prepare_remote_sync_plan(&options.project_root, &env)?;
                let report = run_remote_sync(&plan, RemoteSyncDirection::Push)?;
                Ok(render_remote_sync_report(&report))
            }
        }
        RemoteCommandAction::Pull => {
            if options.dry_run {
                render_remote_dry_run(&options.project_root, &env, RemoteSyncDirection::Pull)
            } else {
                let plan = prepare_remote_sync_plan(&options.project_root, &env)?;
                let report = run_remote_sync(&plan, RemoteSyncDirection::Pull)?;
                Ok(render_remote_sync_report(&report))
            }
        }
        RemoteCommandAction::Drill => {
            let plan = prepare_remote_sync_plan(&options.project_root, &env)?;
            if options.dry_run {
                Ok(render_remote_restore_drill_dry_run(&plan))
            } else {
                let report = run_remote_restore_drill(&plan, options.destructive)?;
                Ok(render_remote_restore_drill_report(&report))
            }
        }
    }
}

fn remote_usage() -> String {
    "Usage: mimir remote <status|push|pull|drill> [--project-root <dir>] [--config <file>] [--dry-run] [--refresh] [--destructive]"
        .to_string()
}

fn remove_inherited_project_env(env: &mut BTreeMap<String, String>) {
    env.remove("MIMIR_CONFIG_PATH");
    env.remove("MIMIR_DRAFTS_DIR");
    env.remove("MIMIR_CAPTURE_SUMMARY_PATH");
}

#[derive(Debug, Clone)]
struct ProjectCommandOptions {
    project_root: PathBuf,
    config_path: Option<PathBuf>,
    project_root_source: ProjectRootSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectRootSource {
    Cwd,
    Flag,
}

impl ProjectRootSource {
    const fn is_explicit(self) -> bool {
        matches!(self, Self::Flag)
    }
}

impl ProjectCommandOptions {
    fn parse(args: &[String], cwd: &Path, usage: fn() -> String) -> Result<Self, HarnessError> {
        let mut project_root = cwd.to_path_buf();
        let mut config_path = None;
        let mut project_root_source = ProjectRootSource::Cwd;

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--project-root" => {
                    project_root_source = ProjectRootSource::Flag;
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "status --project-root requires a value".to_string(),
                        })?;
                    project_root = PathBuf::from(non_empty_command_flag_value(
                        "status",
                        "--project-root",
                        value,
                    )?);
                }
                "--config" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "status --config requires a value".to_string(),
                        })?;
                    config_path = Some(PathBuf::from(non_empty_command_flag_value(
                        "status", "--config", value,
                    )?));
                }
                "-h" | "--help" => {
                    return Err(HarnessError::RemoteSyncUnavailable { message: usage() });
                }
                unknown if unknown.starts_with('-') => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unknown status flag `{unknown}`"),
                    });
                }
                unknown => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unexpected status argument `{unknown}`"),
                    });
                }
            }
        }

        Ok(Self {
            project_root,
            config_path,
            project_root_source,
        })
    }
}

#[derive(Debug, Clone)]
struct ContextCommandOptions {
    project_root: PathBuf,
    config_path: Option<PathBuf>,
    project_root_source: ProjectRootSource,
    limit: usize,
}

impl ContextCommandOptions {
    fn parse(args: &[String], cwd: &Path) -> Result<Self, HarnessError> {
        let mut project_root = cwd.to_path_buf();
        let mut config_path = None;
        let mut project_root_source = ProjectRootSource::Cwd;
        let mut limit = 12_usize;

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--project-root" => {
                    project_root_source = ProjectRootSource::Flag;
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "context --project-root requires a value".to_string(),
                        })?;
                    project_root = PathBuf::from(non_empty_command_flag_value(
                        "context",
                        "--project-root",
                        value,
                    )?);
                }
                "--config" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "context --config requires a value".to_string(),
                        })?;
                    config_path = Some(PathBuf::from(non_empty_command_flag_value(
                        "context", "--config", value,
                    )?));
                }
                "--limit" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "context --limit requires a value".to_string(),
                        })?;
                    let value = non_empty_command_flag_value("context", "--limit", value)?;
                    limit = value.parse::<usize>().map_err(|_| {
                        HarnessError::RemoteSyncUnavailable {
                            message: "context --limit must be a positive integer between 1 and 64"
                                .to_string(),
                        }
                    })?;
                    if !(1..=64).contains(&limit) {
                        return Err(HarnessError::RemoteSyncUnavailable {
                            message: "context --limit must be a positive integer between 1 and 64"
                                .to_string(),
                        });
                    }
                }
                "-h" | "--help" => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: context_usage(),
                    });
                }
                unknown if unknown.starts_with('-') => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unknown context flag `{unknown}`"),
                    });
                }
                unknown => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unexpected context argument `{unknown}`"),
                    });
                }
            }
        }

        Ok(Self {
            project_root,
            config_path,
            project_root_source,
            limit,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryCommandAction {
    List,
    Show,
    Explain,
    Revoke,
}

impl MemoryCommandAction {
    fn parse(value: &str) -> Result<Self, HarnessError> {
        match value {
            "list" => Ok(Self::List),
            "show" => Ok(Self::Show),
            "explain" => Ok(Self::Explain),
            "revoke" => Ok(Self::Revoke),
            _ => Err(HarnessError::MemoryUnavailable {
                message: format!(
                    "unknown memory action `{value}`; expected list, show, explain, or revoke"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct MemoryCommandOptions {
    action: MemoryCommandAction,
    project_root: PathBuf,
    project_root_source: ProjectRootSource,
    config_path: Option<PathBuf>,
    limit: usize,
    kind: Option<String>,
    id: Option<String>,
    reason: Option<String>,
    dry_run: bool,
}

impl MemoryCommandOptions {
    fn parse(args: &[String], cwd: &Path) -> Result<Self, HarnessError> {
        let Some(action_arg) = args.first() else {
            return Err(HarnessError::MemoryUnavailable {
                message: memory_usage(),
            });
        };
        if action_arg == "-h" || action_arg == "--help" {
            return Err(HarnessError::MemoryUnavailable {
                message: memory_usage(),
            });
        }
        let action = MemoryCommandAction::parse(action_arg)?;
        let mut options = Self {
            action,
            project_root: cwd.to_path_buf(),
            project_root_source: ProjectRootSource::Cwd,
            config_path: None,
            limit: 50,
            kind: None,
            id: None,
            reason: None,
            dry_run: false,
        };
        let mut iter = args[1..].iter();
        while let Some(arg) = iter.next() {
            options.parse_arg(arg, &mut iter)?;
        }
        options.validate()?;
        Ok(options)
    }

    fn parse_arg(
        &mut self,
        arg: &str,
        iter: &mut std::slice::Iter<'_, String>,
    ) -> Result<(), HarnessError> {
        match arg {
            "--project-root" => {
                self.project_root_source = ProjectRootSource::Flag;
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --project-root requires a value".to_string(),
                })?;
                self.project_root = PathBuf::from(non_empty_command_flag_value(
                    "memory",
                    "--project-root",
                    value,
                )?);
            }
            "--config" => {
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --config requires a value".to_string(),
                })?;
                self.config_path = Some(PathBuf::from(non_empty_command_flag_value(
                    "memory", "--config", value,
                )?));
            }
            "--limit" => {
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --limit requires a value".to_string(),
                })?;
                let value = non_empty_command_flag_value("memory", "--limit", value)?;
                self.limit =
                    value
                        .parse::<usize>()
                        .map_err(|_| HarnessError::MemoryUnavailable {
                            message: "memory --limit must be a positive integer between 1 and 1000"
                                .to_string(),
                        })?;
                if !(1..=1000).contains(&self.limit) {
                    return Err(HarnessError::MemoryUnavailable {
                        message: "memory --limit must be a positive integer between 1 and 1000"
                            .to_string(),
                    });
                }
            }
            "--kind" => {
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --kind requires a value".to_string(),
                })?;
                self.kind =
                    Some(non_empty_command_flag_value("memory", "--kind", value)?.to_string());
            }
            "--id" => {
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --id requires a value".to_string(),
                })?;
                self.id = Some(non_empty_command_flag_value("memory", "--id", value)?.to_string());
            }
            "--reason" => {
                let value = iter.next().ok_or_else(|| HarnessError::MemoryUnavailable {
                    message: "memory --reason requires a value".to_string(),
                })?;
                self.reason =
                    Some(non_empty_command_flag_value("memory", "--reason", value)?.to_string());
            }
            "--dry-run" => self.dry_run = true,
            "-h" | "--help" => {
                return Err(HarnessError::MemoryUnavailable {
                    message: memory_usage(),
                });
            }
            unknown if unknown.starts_with('-') => {
                return Err(HarnessError::MemoryUnavailable {
                    message: format!("unknown memory flag `{unknown}`"),
                });
            }
            unknown => {
                return Err(HarnessError::MemoryUnavailable {
                    message: format!("unexpected memory argument `{unknown}`"),
                });
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), HarnessError> {
        match self.action {
            MemoryCommandAction::List => Ok(()),
            MemoryCommandAction::Show | MemoryCommandAction::Explain => {
                if self.id.is_none() {
                    return Err(HarnessError::MemoryUnavailable {
                        message: "memory show/explain requires --id <memory-id>".to_string(),
                    });
                }
                Ok(())
            }
            MemoryCommandAction::Revoke => {
                if self.id.is_none() {
                    return Err(HarnessError::MemoryUnavailable {
                        message: "memory revoke requires --id <memory-id>".to_string(),
                    });
                }
                if self.reason.is_none() {
                    return Err(HarnessError::MemoryUnavailable {
                        message: "memory revoke requires --reason <reason>".to_string(),
                    });
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftsCommandAction {
    Status,
    List,
    Show,
    Next,
    Skip,
    Quarantine,
}

impl DraftsCommandAction {
    fn parse(value: &str) -> Result<Self, HarnessError> {
        match value {
            "status" => Ok(Self::Status),
            "list" => Ok(Self::List),
            "show" => Ok(Self::Show),
            "next" => Ok(Self::Next),
            "skip" => Ok(Self::Skip),
            "quarantine" => Ok(Self::Quarantine),
            _ => Err(HarnessError::RemoteSyncUnavailable {
                message: format!(
                    "unknown drafts action `{value}`; expected status, list, show, next, skip, or quarantine"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct DraftsCommandOptions {
    action: DraftsCommandAction,
    project_root: PathBuf,
    project_root_source: ProjectRootSource,
    config_path: Option<PathBuf>,
    drafts_dir: Option<PathBuf>,
    state: Option<DraftState>,
    id: Option<String>,
    reason: Option<String>,
}

impl DraftsCommandOptions {
    fn parse(args: &[String], cwd: &Path) -> Result<Self, HarnessError> {
        let Some(action_arg) = args.first() else {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: drafts_usage(),
            });
        };
        if action_arg == "-h" || action_arg == "--help" {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: drafts_usage(),
            });
        }
        let action = DraftsCommandAction::parse(action_arg)?;
        let mut options = Self::new(action, cwd);

        let mut iter = args[1..].iter();
        while let Some(arg) = iter.next() {
            options.parse_arg(arg, &mut iter)?;
        }
        options.validate()?;
        Ok(options)
    }

    fn new(action: DraftsCommandAction, cwd: &Path) -> Self {
        Self {
            action,
            project_root: cwd.to_path_buf(),
            project_root_source: ProjectRootSource::Cwd,
            config_path: None,
            drafts_dir: None,
            state: None,
            id: None,
            reason: None,
        }
    }

    fn parse_arg(
        &mut self,
        arg: &str,
        iter: &mut std::slice::Iter<'_, String>,
    ) -> Result<(), HarnessError> {
        match arg {
            "--project-root" => {
                self.project_root_source = ProjectRootSource::Flag;
                self.project_root =
                    PathBuf::from(take_drafts_option_value(iter, "--project-root")?);
            }
            "--config" => {
                self.config_path = Some(PathBuf::from(take_drafts_option_value(iter, "--config")?));
            }
            "--drafts-dir" => {
                self.drafts_dir = Some(PathBuf::from(take_drafts_option_value(
                    iter,
                    "--drafts-dir",
                )?));
            }
            "--state" => {
                self.state = Some(parse_draft_state(&take_drafts_option_value(
                    iter, "--state",
                )?)?);
            }
            "--reason" => {
                self.reason = Some(take_drafts_option_value(iter, "--reason")?);
            }
            "-h" | "--help" => {
                return Err(HarnessError::RemoteSyncUnavailable {
                    message: drafts_usage(),
                });
            }
            unknown if unknown.starts_with('-') => {
                return Err(HarnessError::RemoteSyncUnavailable {
                    message: format!("unknown drafts flag `{unknown}`"),
                });
            }
            value => self.parse_positional(value)?,
        }
        Ok(())
    }

    fn parse_positional(&mut self, value: &str) -> Result<(), HarnessError> {
        if !matches!(
            self.action,
            DraftsCommandAction::Show | DraftsCommandAction::Skip | DraftsCommandAction::Quarantine
        ) {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: format!("unexpected drafts argument `{value}`"),
            });
        }
        if self.id.is_some() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: format!("unexpected drafts argument `{value}`"),
            });
        }
        self.id = Some(value.to_string());
        Ok(())
    }

    fn validate(&self) -> Result<(), HarnessError> {
        if self.action == DraftsCommandAction::Show && self.id.is_none() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts show requires a draft id".to_string(),
            });
        }
        if self.action == DraftsCommandAction::Skip && self.id.is_none() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts skip requires a draft id".to_string(),
            });
        }
        if self.action == DraftsCommandAction::Quarantine && self.id.is_none() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts quarantine requires a draft id".to_string(),
            });
        }
        if self.action == DraftsCommandAction::Skip && self.reason.is_none() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts skip requires --reason".to_string(),
            });
        }
        if self.action == DraftsCommandAction::Quarantine && self.reason.is_none() {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts quarantine requires --reason".to_string(),
            });
        }
        if !matches!(
            self.action,
            DraftsCommandAction::Skip | DraftsCommandAction::Quarantine
        ) && self.reason.is_some()
        {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "drafts --reason is only supported for skip or quarantine".to_string(),
            });
        }
        Ok(())
    }
}

fn take_drafts_option_value(
    iter: &mut std::slice::Iter<'_, String>,
    flag: &str,
) -> Result<String, HarnessError> {
    let value = iter
        .next()
        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
            message: format!("drafts {flag} requires a value"),
        })?;
    Ok(non_empty_command_flag_value("drafts", flag, value)?.to_string())
}

fn parse_draft_state(value: &str) -> Result<DraftState, HarnessError> {
    match value {
        "pending" => Ok(DraftState::Pending),
        "processing" => Ok(DraftState::Processing),
        "accepted" => Ok(DraftState::Accepted),
        "skipped" => Ok(DraftState::Skipped),
        "failed" => Ok(DraftState::Failed),
        "quarantined" => Ok(DraftState::Quarantined),
        _ => Err(HarnessError::RemoteSyncUnavailable {
            message: format!(
                "unknown draft state `{value}`; expected pending, processing, accepted, skipped, failed, or quarantined"
            ),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteCommandAction {
    Status,
    Push,
    Pull,
    Drill,
}

impl RemoteCommandAction {
    fn parse(value: &str) -> Result<Self, HarnessError> {
        match value {
            "status" => Ok(Self::Status),
            "push" => Ok(Self::Push),
            "pull" => Ok(Self::Pull),
            "drill" => Ok(Self::Drill),
            _ => Err(HarnessError::RemoteSyncUnavailable {
                message: format!(
                    "unknown remote action `{value}`; expected status, push, pull, or drill"
                ),
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct RemoteCommandOptions {
    action: RemoteCommandAction,
    project_root: PathBuf,
    project_root_source: ProjectRootSource,
    config_path: Option<PathBuf>,
    dry_run: bool,
    refresh: bool,
    destructive: bool,
}

impl RemoteCommandOptions {
    fn parse(args: &[String], cwd: &Path) -> Result<Self, HarnessError> {
        let Some(action_arg) = args.first() else {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: remote_usage(),
            });
        };
        if action_arg == "-h" || action_arg == "--help" {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: remote_usage(),
            });
        }
        let action = RemoteCommandAction::parse(action_arg)?;
        let mut project_root = cwd.to_path_buf();
        let mut project_root_source = ProjectRootSource::Cwd;
        let mut config_path = None;
        let mut dry_run = false;
        let mut refresh = false;
        let mut destructive = false;

        let mut iter = args[1..].iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--project-root" => {
                    project_root_source = ProjectRootSource::Flag;
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "remote --project-root requires a value".to_string(),
                        })?;
                    project_root =
                        PathBuf::from(non_empty_remote_flag_value("--project-root", value)?);
                }
                "--config" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| HarnessError::RemoteSyncUnavailable {
                            message: "remote --config requires a value".to_string(),
                        })?;
                    config_path = Some(PathBuf::from(non_empty_remote_flag_value(
                        "--config", value,
                    )?));
                }
                "--dry-run" => dry_run = true,
                "--refresh" => refresh = true,
                "--destructive" => destructive = true,
                "-h" | "--help" => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: remote_usage(),
                    });
                }
                unknown if unknown.starts_with('-') => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unknown remote flag `{unknown}`"),
                    });
                }
                unknown => {
                    return Err(HarnessError::RemoteSyncUnavailable {
                        message: format!("unexpected remote argument `{unknown}`"),
                    });
                }
            }
        }

        if refresh && action != RemoteCommandAction::Status {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "remote --refresh is only valid with status".to_string(),
            });
        }
        if destructive && action != RemoteCommandAction::Drill {
            return Err(HarnessError::RemoteSyncUnavailable {
                message: "remote --destructive is only valid with drill".to_string(),
            });
        }

        Ok(Self {
            action,
            project_root,
            project_root_source,
            config_path,
            dry_run,
            refresh,
            destructive,
        })
    }
}

fn non_empty_remote_flag_value<'a>(flag: &str, value: &'a str) -> Result<&'a str, HarnessError> {
    non_empty_command_flag_value("remote", flag, value)
}

fn non_empty_command_flag_value<'a>(
    command: &str,
    flag: &str,
    value: &'a str,
) -> Result<&'a str, HarnessError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(HarnessError::RemoteSyncUnavailable {
            message: format!("{command} {flag} cannot be empty"),
        });
    }
    Ok(value)
}

#[derive(Debug, Clone)]
struct ConfigInitOptions {
    path: Option<PathBuf>,
    project_root: PathBuf,
    data_root: String,
    drafts_dir: Option<String>,
    operator: String,
    organization: String,
    remote_kind: String,
    remote_url: String,
    remote_branch: String,
    librarian_after_capture: String,
    dry_run: bool,
}

impl ConfigInitOptions {
    fn parse(args: &[String], cwd: &Path) -> Result<Self, String> {
        let mut options = Self {
            path: None,
            project_root: cwd.to_path_buf(),
            data_root: "state".to_string(),
            drafts_dir: None,
            operator: String::new(),
            organization: String::new(),
            remote_kind: "git".to_string(),
            remote_url: String::new(),
            remote_branch: "main".to_string(),
            librarian_after_capture: "process".to_string(),
            dry_run: false,
        };

        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--path" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --path requires a value".to_string())?;
                    options.path = Some(PathBuf::from(non_empty_flag_value("--path", value)?));
                }
                "--project-root" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --project-root requires a value".to_string())?;
                    options.project_root =
                        PathBuf::from(non_empty_flag_value("--project-root", value)?);
                }
                "--data-root" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --data-root requires a value".to_string())?;
                    options.data_root = non_empty_flag_value("--data-root", value)?.to_string();
                }
                "--drafts-dir" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --drafts-dir requires a value".to_string())?;
                    options.drafts_dir =
                        Some(non_empty_flag_value("--drafts-dir", value)?.to_string());
                }
                "--operator" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --operator requires a value".to_string())?;
                    options.operator = non_empty_flag_value("--operator", value)?.to_string();
                }
                "--organization" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --organization requires a value".to_string())?;
                    options.organization =
                        non_empty_flag_value("--organization", value)?.to_string();
                }
                "--remote-kind" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --remote-kind requires a value".to_string())?;
                    let value = non_empty_flag_value("--remote-kind", value)?;
                    validate_remote_kind(value)?;
                    options.remote_kind = value.to_string();
                }
                "--remote-url" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "config init --remote-url requires a value".to_string())?;
                    options.remote_url = non_empty_flag_value("--remote-url", value)?.to_string();
                }
                "--remote-branch" => {
                    let value = iter.next().ok_or_else(|| {
                        "config init --remote-branch requires a value".to_string()
                    })?;
                    options.remote_branch =
                        non_empty_flag_value("--remote-branch", value)?.to_string();
                }
                "--librarian-after-capture" => {
                    let value = iter.next().ok_or_else(|| {
                        "config init --librarian-after-capture requires a value".to_string()
                    })?;
                    let value = non_empty_flag_value("--librarian-after-capture", value)?;
                    options.librarian_after_capture =
                        normalize_librarian_after_capture(value)?.to_string();
                }
                "--dry-run" => options.dry_run = true,
                "-h" | "--help" => return Err(config_usage()),
                unknown if unknown.starts_with('-') => {
                    return Err(format!("unknown config init flag `{unknown}`"));
                }
                unknown => return Err(format!("unexpected config init argument `{unknown}`")),
            }
        }

        validate_remote_kind(&options.remote_kind)?;
        options.librarian_after_capture =
            normalize_librarian_after_capture(&options.librarian_after_capture)?.to_string();
        Ok(options)
    }

    fn config_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| self.project_root.join(".mimir").join("config.toml"))
    }
}

fn config_init_command(args: &[String], cwd: &Path) -> Result<String, String> {
    let options = ConfigInitOptions::parse(args, cwd)?;
    let path = options.config_path();
    let config = render_config_init_toml(&options);
    if !options.dry_run {
        if path.exists() {
            return Err(format!(
                "refusing to overwrite existing config: {}",
                path.display()
            ));
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        fs::write(&path, &config)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }

    let mut output = String::new();
    output.push_str("mode=");
    output.push_str(if options.dry_run {
        "dry-run"
    } else {
        "written"
    });
    output.push('\n');
    output.push_str("path=");
    output.push_str(&path.display().to_string());
    output.push('\n');
    if options.dry_run {
        output.push_str(&config);
    }
    Ok(output)
}

fn render_config_init_toml(options: &ConfigInitOptions) -> String {
    let mut text = String::new();
    text.push_str("[storage]\n");
    text.push_str("data_root = ");
    text.push_str(&toml_string_literal(&options.data_root));
    text.push_str("\n\n");
    if let Some(drafts_dir) = &options.drafts_dir {
        text.push_str("[drafts]\n");
        text.push_str("dir = ");
        text.push_str(&toml_string_literal(drafts_dir));
        text.push_str("\n\n");
    }
    text.push_str("[native_memory]\n");
    text.push_str("claude = []\n");
    text.push_str("codex = []\n\n");
    text.push_str("[remote]\n");
    text.push_str("kind = ");
    text.push_str(&toml_string_literal(&options.remote_kind));
    text.push('\n');
    text.push_str("url = ");
    text.push_str(&toml_string_literal(&options.remote_url));
    text.push('\n');
    text.push_str("branch = ");
    text.push_str(&toml_string_literal(&options.remote_branch));
    text.push_str("\nauto_push_after_capture = false\n\n");
    text.push_str("[librarian]\n");
    text.push_str("after_capture = ");
    text.push_str(&toml_string_literal(&options.librarian_after_capture));
    text.push_str("\n\n");
    text.push_str("[identity]\n");
    text.push_str("operator = ");
    text.push_str(&toml_string_literal(&options.operator));
    text.push('\n');
    text.push_str("organization = ");
    text.push_str(&toml_string_literal(&options.organization));
    text.push('\n');
    text
}

fn non_empty_flag_value<'a>(flag: &str, value: &'a str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("config init {flag} cannot be empty"));
    }
    Ok(value)
}

fn validate_remote_kind(value: &str) -> Result<(), String> {
    match value {
        "git" | "service" => Ok(()),
        _ => Err(format!(
            "config init --remote-kind `{value}` is invalid; expected git or service"
        )),
    }
}

fn normalize_librarian_after_capture(value: &str) -> Result<&'static str, String> {
    match value {
        "off" => Ok("off"),
        "defer" => Ok("defer"),
        "archive_raw" | "archive-raw" => Ok("archive_raw"),
        "process" => Ok("process"),
        _ => Err(format!(
            "config init --librarian-after-capture `{value}` is invalid; expected off, defer, archive_raw, or process"
        )),
    }
}

fn toml_string_literal(value: &str) -> String {
    let mut literal = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            other => literal.push(other),
        }
    }
    literal.push('"');
    literal
}

fn hook_context_output(env: &BTreeMap<String, String>) -> String {
    if env.get("MIMIR_HARNESS").map(String::as_str) != Some("1") {
        return String::new();
    }

    let agent = env.get("MIMIR_AGENT").map_or("agent", String::as_str);
    let checkpoint = env
        .get("MIMIR_CHECKPOINT_COMMAND")
        .map_or("mimir checkpoint", String::as_str);
    let mut output = String::new();
    output.push_str("Mimir wrapper active for ");
    output.push_str(agent);
    output.push_str(". Use `");
    output.push_str(checkpoint);
    output.push_str(" --title \"Short title\" \"Memory note\"` for durable session memories. ");
    output.push_str("Do not write canonical Mimir memory directly; checkpoint notes are untrusted librarian drafts.");
    match env
        .get("MIMIR_SESSION_DRAFTS_DIR")
        .filter(|value| !value.trim().is_empty())
    {
        Some(path) => {
            output.push_str(" Checkpoint route: ready at ");
            output.push_str(path);
            output.push('.');
        }
        None => output.push_str(
            " Checkpoint route: missing MIMIR_SESSION_DRAFTS_DIR; checkpoint capture will not work until the session is launched through Mimir.",
        ),
    }
    output.push_str(" Before compaction or a long handoff, capture durable decisions, unresolved blockers, and setup changes with the checkpoint command.");

    if let Some(path) = env
        .get("MIMIR_AGENT_GUIDE_PATH")
        .filter(|value| !value.trim().is_empty())
    {
        output.push_str(" Guide: ");
        output.push_str(path);
        output.push('.');
    }
    if let Some(path) = env
        .get("MIMIR_AGENT_SETUP_DIR")
        .filter(|value| !value.trim().is_empty())
    {
        output.push_str(" Native setup artifacts: ");
        output.push_str(path);
        output.push('.');
    }
    if env.get("MIMIR_BOOTSTRAP").map(String::as_str) == Some("required") {
        output.push_str(" First-run setup is pending; read MIMIR_BOOTSTRAP_GUIDE_PATH before assuming governed memory is active.");
    }
    output.push('\n');
    output
}

fn run_checkpoint_command(args: &[String]) -> ExitCode {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    match checkpoint_command(args, &env) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_setup_agent_command(args: &[String]) -> ExitCode {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{}", setup_agent_usage());
        return ExitCode::SUCCESS;
    }
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("mimir: failed to detect current directory: {err}");
            return ExitCode::from(2);
        }
    };
    match setup_agent_command(args, &env, &cwd) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mimir: {err}");
            ExitCode::from(2)
        }
    }
}

fn setup_agent_command(
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<String, String> {
    let options = SetupAgentOptions::parse(args, env, cwd)?;
    if !options.dry_run {
        match options.action {
            SetupAgentAction::Status | SetupAgentAction::Doctor => {}
            SetupAgentAction::Install => install_agent_setup(&options)?,
            SetupAgentAction::Remove => remove_agent_setup(&options)?,
        }
    }
    Ok(if options.action == SetupAgentAction::Doctor {
        render_setup_agent_doctor(&options)
    } else {
        render_setup_agent_status(&options)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupAgentAction {
    Status,
    Doctor,
    Install,
    Remove,
}

impl SetupAgentAction {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "status" => Ok(Self::Status),
            "doctor" => Ok(Self::Doctor),
            "install" => Ok(Self::Install),
            "remove" => Ok(Self::Remove),
            _ => Err(format!(
                "unknown setup-agent action `{value}`; expected status, doctor, install, or remove"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupAgentKind {
    Claude,
    Codex,
}

impl SetupAgentKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err(format!(
                "unknown setup-agent --agent `{value}`; expected claude or codex"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupAgentScope {
    Project,
    User,
}

impl SetupAgentScope {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "project" => Ok(Self::Project),
            "user" => Ok(Self::User),
            _ => Err(format!(
                "unknown setup-agent --scope `{value}`; expected project or user"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SetupFeatures {
    skill: bool,
    hook: bool,
}

impl SetupFeatures {
    const ALL: Self = Self {
        skill: true,
        hook: true,
    };

    fn parse(value: &str) -> Result<Self, String> {
        let mut features = Self {
            skill: false,
            hook: false,
        };
        for part in value.split(',') {
            match part.trim() {
                "all" => features = Self::ALL,
                "skill" | "skills" => features.skill = true,
                "hook" | "hooks" => features.hook = true,
                "" => {}
                unknown => {
                    return Err(format!(
                        "unknown setup-agent --features value `{unknown}`; expected all, skill, or hook"
                    ));
                }
            }
        }
        if !features.skill && !features.hook {
            return Err("setup-agent --features must select at least one feature".to_string());
        }
        Ok(features)
    }
}

#[derive(Debug, Clone)]
struct SetupAgentOptions {
    action: SetupAgentAction,
    agent: SetupAgentKind,
    scope: SetupAgentScope,
    features: SetupFeatures,
    dry_run: bool,
    setup_dir: Option<PathBuf>,
    project_root: PathBuf,
    home_dir: PathBuf,
}

impl SetupAgentOptions {
    fn parse(args: &[String], env: &BTreeMap<String, String>, cwd: &Path) -> Result<Self, String> {
        let Some(action_arg) = args.first() else {
            return Err(setup_agent_usage());
        };
        if action_arg == "-h" || action_arg == "--help" {
            return Err(setup_agent_usage());
        }
        let action = SetupAgentAction::parse(action_arg)?;
        let mut agent = None;
        let mut scope = None;
        let mut features = SetupFeatures::ALL;
        let mut dry_run = false;
        let mut setup_dir = env
            .get("MIMIR_AGENT_SETUP_DIR")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let mut project_root = cwd.to_path_buf();
        let mut home_dir = env
            .get("HOME")
            .filter(|value| !value.trim().is_empty())
            .map_or_else(|| cwd.to_path_buf(), PathBuf::from);

        let mut iter = args[1..].iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--agent" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --agent requires a value".to_string())?;
                    agent = Some(SetupAgentKind::parse(value)?);
                }
                "--scope" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --scope requires a value".to_string())?;
                    scope = Some(SetupAgentScope::parse(value)?);
                }
                "--features" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --features requires a value".to_string())?;
                    features = SetupFeatures::parse(value)?;
                }
                "--from" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --from requires a value".to_string())?;
                    setup_dir = Some(PathBuf::from(value));
                }
                "--project-root" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --project-root requires a value".to_string())?;
                    project_root = PathBuf::from(value);
                }
                "--home" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "setup-agent --home requires a value".to_string())?;
                    home_dir = PathBuf::from(value);
                }
                "--dry-run" => dry_run = true,
                "-h" | "--help" => return Err(setup_agent_usage()),
                unknown if unknown.starts_with('-') => {
                    return Err(format!("unknown setup-agent flag `{unknown}`"));
                }
                unknown => return Err(format!("unexpected setup-agent argument `{unknown}`")),
            }
        }

        let agent = agent.ok_or_else(|| "setup-agent requires --agent claude|codex".to_string())?;
        let scope = scope.ok_or_else(|| "setup-agent requires --scope project|user".to_string())?;
        if action == SetupAgentAction::Install && setup_dir.is_none() {
            return Err(
                "setup-agent install requires --from <MIMIR_AGENT_SETUP_DIR> or MIMIR_AGENT_SETUP_DIR"
                    .to_string(),
            );
        }

        Ok(Self {
            action,
            agent,
            scope,
            features,
            dry_run,
            setup_dir,
            project_root,
            home_dir,
        })
    }

    fn root(&self) -> &Path {
        match self.scope {
            SetupAgentScope::Project => &self.project_root,
            SetupAgentScope::User => &self.home_dir,
        }
    }

    fn skill_source_dir(&self) -> Result<PathBuf, String> {
        let setup_dir = self
            .setup_dir
            .as_ref()
            .ok_or_else(|| "MIMIR_AGENT_SETUP_DIR is not set".to_string())?;
        Ok(setup_dir
            .join(self.agent.as_str())
            .join("skills")
            .join("mimir-checkpoint"))
    }

    fn skill_target_dir(&self) -> PathBuf {
        match self.agent {
            SetupAgentKind::Claude => self
                .root()
                .join(".claude")
                .join("skills")
                .join("mimir-checkpoint"),
            SetupAgentKind::Codex => self
                .root()
                .join(".agents")
                .join("skills")
                .join("mimir-checkpoint"),
        }
    }

    fn hook_json_path(&self) -> PathBuf {
        match self.agent {
            SetupAgentKind::Claude => self.root().join(".claude").join("settings.json"),
            SetupAgentKind::Codex => self.root().join(".codex").join("hooks.json"),
        }
    }

    fn codex_config_path(&self) -> Option<PathBuf> {
        (self.agent == SetupAgentKind::Codex)
            .then(|| self.root().join(".codex").join("config.toml"))
    }
}

fn setup_agent_usage() -> String {
    "Usage: mimir setup-agent <status|doctor|install|remove> --agent <claude|codex> --scope <project|user> [--features all|skill|hook] [--from <dir>] [--project-root <dir>] [--home <dir>] [--dry-run]"
        .to_string()
}

fn install_agent_setup(options: &SetupAgentOptions) -> Result<(), String> {
    if options.features.hook {
        validate_agent_hook_install(options)?;
    }
    if options.features.skill {
        install_agent_skill(options)?;
    }
    if options.features.hook {
        install_agent_hook(options)?;
    }
    Ok(())
}

fn remove_agent_setup(options: &SetupAgentOptions) -> Result<(), String> {
    if options.features.skill {
        remove_agent_skill(options)?;
    }
    if options.features.hook {
        remove_agent_hook(options)?;
    }
    Ok(())
}

fn install_agent_skill(options: &SetupAgentOptions) -> Result<(), String> {
    let source = options.skill_source_dir()?;
    let target = options.skill_target_dir();
    if !source.join("SKILL.md").is_file() {
        return Err(format!(
            "setup skill source is missing: {}",
            source.join("SKILL.md").display()
        ));
    }
    if target.exists() {
        if skill_files_match(&source, &target)? {
            return Ok(());
        }
        return Err(format!(
            "target skill already exists and differs: {}; remove it first",
            target.display()
        ));
    }
    copy_dir_recursive(&source, &target)
}

fn remove_agent_skill(options: &SetupAgentOptions) -> Result<(), String> {
    let target = options.skill_target_dir();
    if target.exists() {
        if !skill_target_owned_by_mimir(&target)? {
            return Err(format!(
                "refusing to remove non-Mimir skill target: {}",
                target.display()
            ));
        }
        fs::remove_dir_all(&target)
            .map_err(|source| format!("failed to remove {}: {source}", target.display()))?;
    }
    Ok(())
}

fn skill_target_owned_by_mimir(target: &Path) -> Result<bool, String> {
    let path = target.join("SKILL.md");
    if !path.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .any(|line| line == "name: mimir-checkpoint"))
}

fn skill_files_match(source: &Path, target: &Path) -> Result<bool, String> {
    let source_text = fs::read(source.join("SKILL.md"))
        .map_err(|err| format!("failed to read source skill: {err}"))?;
    let target_text = fs::read(target.join("SKILL.md"))
        .map_err(|err| format!("failed to read target skill: {err}"))?;
    Ok(source_text == target_text)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target)
        .map_err(|err| format!("failed to create {}: {err}", target.display()))?;
    for entry in
        fs::read_dir(source).map_err(|err| format!("failed to read {}: {err}", source.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {err}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {err}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn install_agent_hook(options: &SetupAgentOptions) -> Result<(), String> {
    validate_agent_hook_install(options)?;
    let hook_path = options.hook_json_path();
    upsert_mimir_hook(&hook_path, options.agent)?;
    if options.agent == SetupAgentKind::Codex {
        if let Some(config_path) = options.codex_config_path() {
            ensure_codex_hooks_feature(&config_path)?;
        }
    }
    Ok(())
}

fn validate_agent_hook_install(options: &SetupAgentOptions) -> Result<(), String> {
    if options.agent == SetupAgentKind::Codex {
        if let Some(config_path) = options.codex_config_path() {
            validate_codex_hooks_not_disabled(&config_path)?;
        }
    }
    Ok(())
}

fn remove_agent_hook(options: &SetupAgentOptions) -> Result<(), String> {
    let hook_path = options.hook_json_path();
    if hook_path.exists() {
        remove_mimir_hook(&hook_path)?;
    }
    Ok(())
}

fn validate_codex_hooks_not_disabled(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if codex_hooks_feature_disabled(&text) {
        return Err(format!(
            "codex hooks are explicitly disabled in {}; set codex_hooks = true before installing the Mimir hook",
            path.display()
        ));
    }
    Ok(())
}

fn upsert_mimir_hook(path: &Path, agent: SetupAgentKind) -> Result<bool, String> {
    let mut value = read_hook_json(path)?;
    let missing_specs = required_mimir_hook_specs(agent)
        .iter()
        .copied()
        .filter(|spec| !event_has_mimir_hook(&value, spec.event))
        .collect::<Vec<_>>();
    if missing_specs.is_empty() {
        return Ok(false);
    }
    let root = value
        .as_object_mut()
        .ok_or_else(|| format!("hook file must be a JSON object: {}", path.display()))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("hook file `hooks` must be an object: {}", path.display()))?;
    for spec in missing_specs {
        let event_hooks = hooks
            .entry(spec.event)
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                format!(
                    "hook file `hooks.{}` must be an array: {}",
                    spec.event,
                    path.display()
                )
            })?;
        event_hooks.push(mimir_hook_group(spec.matcher));
    }
    write_pretty_json(path, &value)?;
    Ok(true)
}

fn remove_mimir_hook(path: &Path) -> Result<(), String> {
    let mut value = read_hook_json(path)?;
    let mut changed = false;
    let Some(hooks) = value
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(());
    };
    let event_names = hooks.keys().cloned().collect::<Vec<_>>();
    for event_name in event_names {
        let Some(event_hooks) = hooks
            .get_mut(&event_name)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for group in event_hooks.iter_mut() {
            if let Some(handlers) = group
                .get_mut("hooks")
                .and_then(serde_json::Value::as_array_mut)
            {
                let before = handlers.len();
                handlers.retain(|handler| !is_mimir_hook_handler(handler));
                changed |= handlers.len() != before;
            }
        }
        let before_groups = event_hooks.len();
        event_hooks.retain(|group| {
            group
                .get("hooks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(|hooks| !hooks.is_empty())
        });
        changed |= event_hooks.len() != before_groups;
        if event_hooks.is_empty() {
            hooks.remove(&event_name);
        }
    }
    if changed {
        write_pretty_json(path, &value)?;
    }
    Ok(())
}

fn read_hook_json(path: &Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn write_pretty_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("failed to render {}: {err}", path.display()))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

#[derive(Debug, Clone, Copy)]
struct MimirHookSpec {
    event: &'static str,
    matcher: &'static str,
}

const CODEX_MIMIR_HOOK_SPECS: &[MimirHookSpec] = &[MimirHookSpec {
    event: "SessionStart",
    matcher: "startup|resume",
}];

const CLAUDE_MIMIR_HOOK_SPECS: &[MimirHookSpec] = &[
    MimirHookSpec {
        event: "SessionStart",
        matcher: "startup|resume|compact",
    },
    MimirHookSpec {
        event: "PreCompact",
        matcher: "manual|auto",
    },
];

fn required_mimir_hook_specs(agent: SetupAgentKind) -> &'static [MimirHookSpec] {
    match agent {
        SetupAgentKind::Claude => CLAUDE_MIMIR_HOOK_SPECS,
        SetupAgentKind::Codex => CODEX_MIMIR_HOOK_SPECS,
    }
}

fn mimir_hook_group(matcher: &'static str) -> serde_json::Value {
    serde_json::json!({
        "matcher": matcher,
        "hooks": [
            {
                "type": "command",
                "command": "mimir hook-context"
            }
        ]
    })
}

fn has_mimir_hook(value: &serde_json::Value, agent: SetupAgentKind) -> bool {
    required_mimir_hook_specs(agent)
        .iter()
        .all(|spec| event_has_mimir_hook(value, spec.event))
}

fn event_has_mimir_hook(value: &serde_json::Value, event: &str) -> bool {
    value
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|handlers| handlers.iter().any(is_mimir_hook_handler))
            })
        })
}

fn missing_mimir_hook_reason(
    value: &serde_json::Value,
    agent: SetupAgentKind,
) -> Option<&'static str> {
    if !event_has_mimir_hook(value, "SessionStart") {
        return Some("mimir_session_start_hook_missing");
    }
    if agent == SetupAgentKind::Claude && !event_has_mimir_hook(value, "PreCompact") {
        return Some("mimir_precompact_hook_missing");
    }
    None
}

fn is_mimir_hook_handler(value: &serde_json::Value) -> bool {
    value.get("type").and_then(serde_json::Value::as_str) == Some("command")
        && value.get("command").and_then(serde_json::Value::as_str) == Some("mimir hook-context")
}

fn ensure_codex_hooks_feature(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    if !path.exists() {
        fs::write(path, "[features]\ncodex_hooks = true\n")
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        return Ok(());
    }

    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    if codex_hooks_feature_enabled(&text) {
        return Ok(());
    }
    if codex_hooks_feature_disabled(&text) {
        return Err(format!(
            "codex hooks are explicitly disabled in {}; set codex_hooks = true before installing the Mimir hook",
            path.display()
        ));
    }
    let updated = insert_codex_hooks_feature(&text);
    fs::write(path, updated).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn codex_hooks_feature_enabled(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .any(|line| line == "codex_hooks = true")
}

fn codex_hooks_feature_disabled(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .any(|line| line == "codex_hooks = false")
}

fn insert_codex_hooks_feature(text: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(index) = lines.iter().position(|line| line.trim() == "[features]") {
        lines.insert(index + 1, "codex_hooks = true".to_string());
        let mut updated = lines.join("\n");
        updated.push('\n');
        return updated;
    }
    let mut updated = text.to_string();
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("\n[features]\ncodex_hooks = true\n");
    updated
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupSurfaceState {
    Installed,
    Missing,
    Partial,
}

impl SetupSurfaceState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Missing => "missing",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone)]
struct SetupSurfaceStatus {
    state: SetupSurfaceState,
    reason: &'static str,
    path: PathBuf,
    config_path: Option<PathBuf>,
}

fn render_setup_agent_status(options: &SetupAgentOptions) -> String {
    let mut output = String::new();
    output.push_str("agent=");
    output.push_str(options.agent.as_str());
    output.push('\n');
    output.push_str("scope=");
    output.push_str(options.scope.as_str());
    output.push('\n');
    if options.dry_run {
        output.push_str("mode=dry-run\n");
    }
    if options.features.skill {
        let status = skill_setup_status(options);
        output.push_str("skill=");
        output.push_str(status.state.as_str());
        output.push_str(" path=");
        output.push_str(&status.path.display().to_string());
        output.push_str(" reason=");
        output.push_str(status.reason);
        if let Some(action) = dry_run_action(options, status.state) {
            output.push_str(" action=");
            output.push_str(action);
        }
        output.push('\n');
    }
    if options.features.hook {
        let status = hook_setup_status(options);
        output.push_str("hook=");
        output.push_str(status.state.as_str());
        output.push_str(" path=");
        output.push_str(&status.path.display().to_string());
        output.push_str(" reason=");
        output.push_str(status.reason);
        if let Some(config_path) = status.config_path {
            output.push_str(" config_path=");
            output.push_str(&config_path.display().to_string());
        }
        if let Some(action) = dry_run_action(options, status.state) {
            output.push_str(" action=");
            output.push_str(action);
        }
        output.push('\n');
    }
    output
}

fn render_setup_agent_doctor(options: &SetupAgentOptions) -> String {
    let skill = options.features.skill.then(|| skill_setup_status(options));
    let hook = options.features.hook.then(|| hook_setup_status(options));
    let ready = skill
        .as_ref()
        .is_none_or(|status| status.state == SetupSurfaceState::Installed)
        && hook
            .as_ref()
            .is_none_or(|status| status.state == SetupSurfaceState::Installed);

    let mut output = render_setup_agent_status(options);
    output.push_str("doctor_status=");
    output.push_str(if ready { "ready" } else { "action_required" });
    output.push('\n');
    if let Some(config_path) = options.codex_config_path() {
        output.push_str("codex_config_status=");
        output.push_str(codex_config_feature_status(&config_path));
        output.push('\n');
    }
    output.push_str("status_command=");
    output.push_str(&setup_agent_command_line("status", options, None));
    output.push('\n');
    output.push_str("install_command=");
    output.push_str(&setup_agent_command_line(
        "install",
        options,
        options.setup_dir.as_deref(),
    ));
    output.push('\n');
    output.push_str("remove_command=");
    output.push_str(&setup_agent_command_line("remove", options, None));
    output.push('\n');
    output.push_str("context_command=mimir context");
    if options.scope == SetupAgentScope::Project {
        output.push_str(" --project-root ");
        output.push_str(&shell_path_arg(&options.project_root));
    }
    output.push('\n');
    output.push_str("checkpoint_command=");
    output.push_str("mimir checkpoint --title \"Short title\" \"Memory note\"");
    output.push('\n');
    output.push_str("next_action=");
    if ready {
        output.push_str("none");
    } else {
        output.push_str(&setup_agent_command_line(
            "install",
            options,
            options.setup_dir.as_deref(),
        ));
    }
    output.push('\n');
    output
}

fn setup_agent_command_line(
    action: &str,
    options: &SetupAgentOptions,
    setup_dir: Option<&Path>,
) -> String {
    let mut command = format!(
        "mimir setup-agent {action} --agent {} --scope {}",
        options.agent.as_str(),
        options.scope.as_str()
    );
    if action == "install" {
        command.push_str(" --from ");
        command.push_str(
            &setup_dir.map_or_else(|| "$MIMIR_AGENT_SETUP_DIR".to_string(), shell_path_arg),
        );
    }
    match options.scope {
        SetupAgentScope::Project => {
            command.push_str(" --project-root ");
            command.push_str(&shell_path_arg(&options.project_root));
        }
        SetupAgentScope::User => {
            command.push_str(" --home ");
            command.push_str(&shell_path_arg(&options.home_dir));
        }
    }
    command
}

fn shell_path_arg(path: &Path) -> String {
    let value = path.display().to_string();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '+'))
    {
        return value;
    }
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn codex_config_feature_status(path: &Path) -> &'static str {
    let Ok(text) = fs::read_to_string(path) else {
        return "missing";
    };
    if codex_hooks_feature_enabled(&text) {
        "enabled"
    } else if codex_hooks_feature_disabled(&text) {
        "disabled"
    } else {
        "missing"
    }
}

fn skill_setup_status(options: &SetupAgentOptions) -> SetupSurfaceStatus {
    let path = options.skill_target_dir();
    let skill_file = path.join("SKILL.md");
    let (state, reason) = if skill_file.is_file() {
        match skill_target_owned_by_mimir(&path) {
            Ok(true) => (SetupSurfaceState::Installed, "mimir_skill_present"),
            Ok(false) => (SetupSurfaceState::Partial, "non_mimir_skill_present"),
            Err(_) => (SetupSurfaceState::Partial, "skill_file_unreadable"),
        }
    } else if path.exists() {
        (
            SetupSurfaceState::Partial,
            "skill_directory_without_skill_file",
        )
    } else {
        (SetupSurfaceState::Missing, "skill_file_missing")
    };
    SetupSurfaceStatus {
        state,
        reason,
        path,
        config_path: None,
    }
}

fn hook_setup_status(options: &SetupAgentOptions) -> SetupSurfaceStatus {
    let path = options.hook_json_path();
    if !path.exists() {
        return SetupSurfaceStatus {
            state: SetupSurfaceState::Missing,
            reason: "hook_file_missing",
            path,
            config_path: options.codex_config_path(),
        };
    }
    let Ok(hook_value) = read_hook_json(&path) else {
        return SetupSurfaceStatus {
            state: SetupSurfaceState::Partial,
            reason: "hook_json_invalid",
            path,
            config_path: options.codex_config_path(),
        };
    };
    if !has_mimir_hook(&hook_value, options.agent) {
        let reason =
            missing_mimir_hook_reason(&hook_value, options.agent).unwrap_or("mimir_hook_missing");
        return SetupSurfaceStatus {
            state: if reason == "mimir_session_start_hook_missing" {
                SetupSurfaceState::Missing
            } else {
                SetupSurfaceState::Partial
            },
            reason,
            path,
            config_path: options.codex_config_path(),
        };
    }
    if let Some(config_path) = options.codex_config_path() {
        let Ok(text) = fs::read_to_string(&config_path) else {
            return SetupSurfaceStatus {
                state: SetupSurfaceState::Partial,
                reason: "codex_hooks_feature_missing",
                path,
                config_path: Some(config_path),
            };
        };
        if codex_hooks_feature_enabled(&text) {
            return SetupSurfaceStatus {
                state: SetupSurfaceState::Installed,
                reason: "mimir_hook_present",
                path,
                config_path: Some(config_path),
            };
        }
        let reason = if codex_hooks_feature_disabled(&text) {
            "codex_hooks_feature_disabled"
        } else {
            "codex_hooks_feature_missing"
        };
        return SetupSurfaceStatus {
            state: SetupSurfaceState::Partial,
            reason,
            path,
            config_path: Some(config_path),
        };
    }
    SetupSurfaceStatus {
        state: SetupSurfaceState::Installed,
        reason: "mimir_hook_present",
        path,
        config_path: None,
    }
}

fn dry_run_action(options: &SetupAgentOptions, state: SetupSurfaceState) -> Option<&'static str> {
    if !options.dry_run {
        return None;
    }
    match options.action {
        SetupAgentAction::Status | SetupAgentAction::Doctor => Some("none"),
        SetupAgentAction::Install => match state {
            SetupSurfaceState::Installed => Some("none"),
            SetupSurfaceState::Missing | SetupSurfaceState::Partial => Some("would_install"),
        },
        SetupAgentAction::Remove => match state {
            SetupSurfaceState::Installed | SetupSurfaceState::Partial => Some("would_remove"),
            SetupSurfaceState::Missing => Some("none"),
        },
    }
}

fn checkpoint_command(
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<String, HarnessError> {
    let mut title = None;
    let mut list = false;
    let mut body_parts = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(checkpoint_usage()),
            "--list" => list = true,
            "--title" => {
                let value = iter.next().ok_or_else(|| HarnessError::MissingFlagValue {
                    flag: "--title".to_string(),
                })?;
                title = Some(value.clone());
            }
            other if other.starts_with('-') => {
                return Err(HarnessError::UnknownFlag {
                    flag: other.to_string(),
                });
            }
            _ => body_parts.push(arg.clone()),
        }
    }

    let session_drafts_dir = checkpoint_session_dir(env)?;
    if list {
        let mut output = String::new();
        for note in list_checkpoint_notes(&session_drafts_dir)? {
            output.push_str(&note.display().to_string());
            output.push('\n');
        }
        return Ok(output);
    }

    let body = checkpoint_body(&body_parts)?;
    let metadata = CheckpointNoteMetadata {
        session_id: env.get("MIMIR_SESSION_ID").cloned(),
        agent: env.get("MIMIR_AGENT").cloned(),
        project: env.get("MIMIR_PROJECT").cloned(),
        operator: None,
    };
    let note = write_checkpoint_note(
        &session_drafts_dir,
        title.as_deref(),
        &body,
        &metadata,
        std::time::SystemTime::now(),
    )?;
    Ok(format!("{}\n", note.path.display()))
}

fn checkpoint_usage() -> String {
    "Usage: mimir checkpoint [--title <title>] [--list] [text...]\n\
     \n\
     Writes a session-local checkpoint note into MIMIR_SESSION_DRAFTS_DIR.\n\
     Run inside a wrapped `mimir <agent>` session.\n"
        .to_string()
}

fn checkpoint_session_dir(env: &BTreeMap<String, String>) -> Result<PathBuf, HarnessError> {
    env.get("MIMIR_SESSION_DRAFTS_DIR")
        .filter(|value| !value.trim().is_empty())
        .map(Path::new)
        .map(Path::to_path_buf)
        .ok_or(HarnessError::CheckpointSessionDraftsDirMissing)
}

fn checkpoint_body(body_parts: &[String]) -> Result<String, HarnessError> {
    if body_parts.is_empty() {
        if io::stdin().is_terminal() {
            return Err(HarnessError::CheckpointEmpty);
        }
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|source| HarnessError::DraftWrite {
                path: PathBuf::from("stdin"),
                source,
            })?;
        if input.trim().is_empty() {
            return Err(HarnessError::CheckpointEmpty);
        }
        Ok(input)
    } else {
        Ok(body_parts.join(" "))
    }
}

fn report_argument_error(error: &HarnessError) -> ExitCode {
    eprintln!("mimir: {error}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn report_spawn_error(error: &HarnessError) -> ExitCode {
    let exit = match error {
        HarnessError::Spawn { source, .. } if source.kind() == std::io::ErrorKind::NotFound => 127,
        _ => 2,
    };
    eprintln!("mimir: {error}");
    ExitCode::from(exit)
}

fn exit_code_from_status(code: Option<i32>) -> ExitCode {
    match code {
        Some(code) => match u8::try_from(code) {
            Ok(code) => ExitCode::from(code),
            Err(_) => ExitCode::from(1),
        },
        _ => ExitCode::from(1),
    }
}
