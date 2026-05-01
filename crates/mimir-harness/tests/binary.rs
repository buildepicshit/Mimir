//! Binary-level smoke tests for the `mimir` launcher.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use mimir_core::{ClockTime, Store, WorkspaceId};
use mimir_librarian::{Draft, DraftMetadata, DraftSourceSurface, DraftStore};

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_path(path: &Path) -> String {
    toml_string(&path.display().to_string())
}

fn full_workspace_hex(workspace_id: WorkspaceId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(workspace_id.as_bytes().len() * 2);
    for byte in workspace_id.as_bytes() {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}

fn run_git(args: &[&str], cwd: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn remove_live_mimir_env(command: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIMIR_") {
            command.env_remove(key);
        }
    }
    command
}

fn write_git_origin(root: &Path, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    let git = root.join(".git");
    fs::create_dir_all(&git)?;
    fs::write(
        git.join("config"),
        format!(
            "[core]\n\
             repositoryformatversion = 0\n\
             [remote \"origin\"]\n\
             \turl = {remote}\n"
        ),
    )?;
    Ok(())
}

fn write_remote_config(
    path: &Path,
    data_root: &Path,
    drafts_dir: &Path,
    remote_url: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!(
            "[storage]\n\
             data_root = {}\n\
             \n\
             [drafts]\n\
             dir = {}\n\
             \n\
             [remote]\n\
             kind = \"git\"\n\
             url = {}\n\
             branch = \"main\"\n\
             \n\
             [identity]\n\
             operator = \"hasnobeef\"\n\
             organization = \"buildepicshit\"\n",
            toml_path(data_root),
            toml_path(drafts_dir),
            toml_path(remote_url)
        ),
    )?;
    Ok(())
}

fn write_service_remote_config(
    path: &Path,
    data_root: &Path,
    remote_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!(
            "[storage]\n\
             data_root = {}\n\
             \n\
             [remote]\n\
             kind = \"service\"\n\
             url = {}\n\
             \n\
             [identity]\n\
             operator = \"hasnobeef\"\n\
             organization = \"buildepicshit\"\n",
            toml_path(data_root),
            toml_string(remote_url)
        ),
    )?;
    Ok(())
}

fn prepare_bare_remote(root: &Path) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let bare = root.join("memory.git");
    let seed = root.join("seed");
    fs::create_dir_all(&seed)?;
    run_git(&["init", "--bare", &bare.display().to_string()], root)?;
    run_git(&["init"], &seed)?;
    run_git(&["config", "user.name", "Mimir Test"], &seed)?;
    run_git(&["config", "user.email", "mimir@example.invalid"], &seed)?;
    fs::write(seed.join("README.md"), "Mimir memory remote\n")?;
    run_git(&["add", "README.md"], &seed)?;
    run_git(&["commit", "-m", "seed remote"], &seed)?;
    run_git(&["branch", "-M", "main"], &seed)?;
    run_git(
        &["remote", "add", "origin", &bare.display().to_string()],
        &seed,
    )?;
    run_git(&["push", "-u", "origin", "main"], &seed)?;
    run_git(
        &[
            "--git-dir",
            &bare.display().to_string(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
        root,
    )?;
    Ok(bare)
}

fn assert_remote_status_reports_synced_checkout(
    project: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("status")
        .arg("--project-root")
        .arg(project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(status.status.success(), "status: {:?}", status.status);
    let status_stdout = String::from_utf8(status.stdout)?;
    assert!(status_stdout.contains("remote_checkout_status=present"));
    assert!(status_stdout.contains("remote_workspace_log_status=present"));
    assert!(status_stdout.contains("remote_draft_files=1"));
    assert!(status_stdout.contains("workspace_log_relation=synced"));
    assert!(status_stdout.contains("next_action=none"));
    Ok(())
}

fn status_value<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    stdout.lines().find_map(|line| line.strip_prefix(&prefix))
}

fn normalize_status_path_text(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    if let Some(stripped) = normalized.strip_prefix("/private/var/") {
        return format!("/var/{stripped}");
    }
    normalized
}

fn expected_status_path(path: &Path) -> String {
    normalize_status_path_text(&path.display().to_string())
}

fn remote_status_stdout(project: &Path) -> Result<String, Box<dyn std::error::Error>> {
    remote_status_stdout_with_args(project, &[])
}

fn remote_status_stdout_with_args(
    project: &Path,
    extra_args: &[&str],
) -> Result<String, Box<dyn std::error::Error>> {
    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("status")
        .arg("--project-root")
        .arg(project)
        .args(extra_args)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(status.status.success(), "status: {:?}", status.status);
    Ok(String::from_utf8(status.stdout)?)
}

fn assert_remote_clone_contains_workspace_state(
    root: &Path,
    bare: &Path,
    workspace_hex: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let verify = root.join("verify");
    run_git(
        &[
            "clone",
            "--branch",
            "main",
            &bare.display().to_string(),
            &verify.display().to_string(),
        ],
        root,
    )?;
    assert!(verify
        .join("workspaces")
        .join(workspace_hex)
        .join("canonical.log")
        .is_file());
    assert!(verify
        .join("drafts")
        .join(workspace_hex)
        .join("pending")
        .join("draft.json")
        .is_file());
    Ok(())
}

fn assert_remote_pull_restores_workspace_state(
    root: &Path,
    bare: &Path,
    project_remote: &str,
    workspace_hex: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let restored_project = root.join("restored-project");
    fs::create_dir_all(&restored_project)?;
    write_git_origin(&restored_project, project_remote)?;
    let restored_data = root.join("restored-data");
    let restored_drafts = root.join("restored-drafts");
    write_remote_config(
        &restored_project.join(".mimir/config.toml"),
        &restored_data,
        &restored_drafts,
        bare,
    )?;

    let pull = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("pull")
        .arg("--project-root")
        .arg(&restored_project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(pull.status.success(), "status: {:?}", pull.status);
    let pull_stdout = String::from_utf8(pull.stdout)?;
    assert!(pull_stdout.contains("direction=pull"));
    assert!(pull_stdout.contains("status=synced"));
    assert!(restored_data
        .join(workspace_hex)
        .join("canonical.log")
        .is_file());
    assert!(restored_drafts.join("pending").join("draft.json").is_file());
    Ok(())
}

fn assert_remote_pull_refuses_divergent_log(
    root: &Path,
    bare: &Path,
    project_remote: &str,
    workspace_hex: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let conflict_project = root.join("conflict-project");
    fs::create_dir_all(&conflict_project)?;
    write_git_origin(&conflict_project, project_remote)?;
    let conflict_data = root.join("conflict-data");
    let conflict_drafts = root.join("conflict-drafts");
    write_remote_config(
        &conflict_project.join(".mimir/config.toml"),
        &conflict_data,
        &conflict_drafts,
        bare,
    )?;
    let conflict_log = conflict_data.join(workspace_hex).join("canonical.log");
    if let Some(parent) = conflict_log.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&conflict_log, b"divergent-local-log")?;

    let conflict = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("pull")
        .arg("--project-root")
        .arg(&conflict_project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert_eq!(conflict.status.code(), Some(2));
    let conflict_stderr = String::from_utf8(conflict.stderr)?;
    assert!(conflict_stderr.contains("local canonical log diverges"));
    Ok(())
}

fn write_setup_artifacts(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let claude_skill = root.join("claude/skills/mimir-checkpoint");
    let codex_skill = root.join("codex/skills/mimir-checkpoint");
    fs::create_dir_all(&claude_skill)?;
    fs::create_dir_all(&codex_skill)?;
    fs::write(
        claude_skill.join("SKILL.md"),
        "---\nname: mimir-checkpoint\ndescription: test claude checkpoint\n---\n",
    )?;
    fs::write(
        codex_skill.join("SKILL.md"),
        "---\nname: mimir-checkpoint\ndescription: test codex checkpoint\n---\n",
    )?;
    Ok(())
}

fn submit_test_draft(
    drafts_dir: &Path,
    raw_text: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    submit_test_draft_at(
        drafts_dir,
        raw_text,
        UNIX_EPOCH + Duration::from_millis(1_772_000_000_000),
    )
}

fn submit_test_draft_at(
    drafts_dir: &Path,
    raw_text: &str,
    submitted_at: std::time::SystemTime,
) -> Result<String, Box<dyn std::error::Error>> {
    let metadata = DraftMetadata {
        source_surface: DraftSourceSurface::AgentExport,
        source_agent: Some("codex".to_string()),
        source_project: Some("buildepicshit/Mimir".to_string()),
        operator: Some("hasnobeef".to_string()),
        provenance_uri: Some("test://draft".to_string()),
        context_tags: vec!["status-test".to_string()],
        submitted_at,
    };
    let draft = Draft::with_metadata(raw_text.to_string(), metadata);
    let id = draft.id().to_string();
    DraftStore::new(drafts_dir).submit(&draft)?;
    Ok(id)
}

#[test]
fn version_flag_reports_binary_version() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("--version")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout did not contain version: {stdout}"
    );
    Ok(())
}

#[test]
fn wrapper_launches_child_and_propagates_success() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let mut command = Command::new(env!("CARGO_BIN_EXE_mimir"));
    let output = remove_live_mimir_env(&mut command)
        .current_dir(tmp.path())
        .arg(rustc)
        .arg("--version")
        .output()?;

    assert!(output.status.success(), "child status: {:?}", output.status);
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("+ Mimir"));
    assert!(stderr.contains("Native setup artifacts"));
    Ok(())
}

#[test]
fn status_project_root_does_not_fallback_to_wrapped_env_config(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let inspected = tmp.path().join("inspected");
    let wrapped = tmp.path().join("wrapped");
    let wrapped_config = wrapped.join(".mimir/config.toml");
    let wrapped_drafts = wrapped.join(".mimir/state/drafts");
    fs::create_dir_all(&inspected)?;
    let config_parent = wrapped_config.parent().ok_or_else(|| {
        format!(
            "wrapped config path has no parent: {}",
            wrapped_config.display()
        )
    })?;
    fs::create_dir_all(config_parent)?;
    fs::write(
        &wrapped_config,
        "[storage]\n\
             data_root = \"state\"\n\
             \n\
             [identity]\n\
             operator = \"hasnobeef\"\n\
             organization = \"buildepicshit\"\n",
    )?;

    let mut command = Command::new(env!("CARGO_BIN_EXE_mimir"));
    let output = remove_live_mimir_env(&mut command)
        .arg("status")
        .arg("--project-root")
        .arg(&inspected)
        .env("MIMIR_CONFIG_PATH", &wrapped_config)
        .env("MIMIR_DRAFTS_DIR", &wrapped_drafts)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("config_status=missing"),
        "status should not use wrapped config for explicit project root:\n{stdout}"
    );
    assert!(
        stdout.contains("drafts_dir=\n"),
        "status should not use wrapped drafts dir for explicit project root:\n{stdout}"
    );

    let foreign_drafts = tmp.path().join("foreign-drafts");
    let mut command = Command::new(env!("CARGO_BIN_EXE_mimir"));
    let explicit_config_output = remove_live_mimir_env(&mut command)
        .arg("status")
        .arg("--project-root")
        .arg(&inspected)
        .arg("--config")
        .arg(&wrapped_config)
        .env("MIMIR_DRAFTS_DIR", &foreign_drafts)
        .output()?;

    assert!(
        explicit_config_output.status.success(),
        "status: {:?}",
        explicit_config_output.status
    );
    let explicit_config_stdout = String::from_utf8(explicit_config_output.stdout)?;
    assert!(
        explicit_config_stdout.contains("config_status=ready"),
        "status should use the explicit config path:\n{explicit_config_stdout}"
    );
    let actual_drafts_dir = status_value(&explicit_config_stdout, "drafts_dir")
        .map(normalize_status_path_text)
        .ok_or("status output should include drafts_dir")?;
    assert_eq!(
        actual_drafts_dir,
        expected_status_path(&wrapped_drafts),
        "status should use the explicit config's drafts root:\n{explicit_config_stdout}"
    );
    assert_ne!(
        actual_drafts_dir,
        expected_status_path(&foreign_drafts),
        "explicit config should not inherit wrapped drafts dir:\n{explicit_config_stdout}"
    );
    Ok(())
}

#[test]
fn hook_context_is_quiet_outside_wrapped_session() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("hook-context")
        .env_remove("MIMIR_HARNESS")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    assert!(output.stdout.is_empty());
    Ok(())
}

#[test]
fn hook_context_prints_wrapped_session_guidance() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let drafts = tmp.path().join("session-drafts");
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("hook-context")
        .env("MIMIR_HARNESS", "1")
        .env("MIMIR_AGENT", "codex")
        .env("MIMIR_CHECKPOINT_COMMAND", "mimir checkpoint")
        .env("MIMIR_SESSION_DRAFTS_DIR", &drafts)
        .env("MIMIR_AGENT_GUIDE_PATH", "/tmp/mimir/agent-guide.md")
        .env("MIMIR_AGENT_SETUP_DIR", "/tmp/mimir/setup")
        .env("MIMIR_BOOTSTRAP", "required")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Mimir wrapper active for codex"));
    assert!(stdout.contains("mimir checkpoint"));
    assert!(stdout.contains("Checkpoint route: ready"));
    assert!(stdout.contains("Before compaction"));
    assert!(stdout.contains("Guide: /tmp/mimir/agent-guide.md"));
    assert!(stdout.contains("Native setup artifacts: /tmp/mimir/setup"));
    assert!(stdout.contains("First-run setup is pending"));
    Ok(())
}

#[test]
fn hook_context_warns_when_checkpoint_route_is_unavailable(
) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("hook-context")
        .env("MIMIR_HARNESS", "1")
        .env("MIMIR_AGENT", "claude")
        .env_remove("MIMIR_SESSION_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Checkpoint route: missing MIMIR_SESSION_DRAFTS_DIR"));
    assert!(stdout.contains("Before compaction"));
    Ok(())
}

#[test]
fn checkpoint_command_writes_note_inside_wrapped_session() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let drafts = tmp.path().join("drafts");
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("checkpoint")
        .arg("--title")
        .arg("Binary checkpoint")
        .arg("Remember helper command coverage.")
        .env("MIMIR_SESSION_DRAFTS_DIR", &drafts)
        .env("MIMIR_SESSION_ID", "session-binary")
        .env("MIMIR_AGENT", "codex")
        .env("MIMIR_PROJECT", "Mimir")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    let path = stdout.trim();
    assert!(path.ends_with("-binary-checkpoint.md"), "path: {path}");
    let note = std::fs::read_to_string(path)?;
    assert!(note.contains("# Binary checkpoint"));
    assert!(note.contains("session_id: session-binary"));
    assert!(note.contains("agent: codex"));
    assert!(note.contains("Remember helper command coverage."));
    Ok(())
}

#[test]
fn checkpoint_command_requires_wrapped_session_env() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("checkpoint")
        .arg("Remember missing env handling.")
        .env_remove("MIMIR_SESSION_DRAFTS_DIR")
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("MIMIR_SESSION_DRAFTS_DIR is not set"));
    Ok(())
}

#[test]
fn setup_agent_installs_statuses_and_removes_claude_project_artifacts(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let setup = tmp.path().join("setup");
    let project = tmp.path().join("project");
    write_setup_artifacts(&setup)?;

    let install = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--from")
        .arg(&setup)
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(install.status.success(), "status: {:?}", install.status);
    let install_stdout = String::from_utf8(install.stdout)?;
    assert!(install_stdout.contains("skill=installed"));
    assert!(install_stdout.contains("hook=installed"));

    let skill_path = project.join(".claude/skills/mimir-checkpoint/SKILL.md");
    assert!(skill_path.is_file());
    let settings_path = project.join(".claude/settings.json");
    let settings = fs::read_to_string(&settings_path)?;
    assert!(settings.contains("SessionStart"));
    assert!(settings.contains("PreCompact"));
    assert!(settings.contains("mimir hook-context"));

    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("status")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(status.status.success(), "status: {:?}", status.status);
    let status_stdout = String::from_utf8(status.stdout)?;
    assert!(status_stdout.contains("skill=installed"));
    assert!(status_stdout.contains("hook=installed"));

    let remove = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("remove")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(remove.status.success(), "status: {:?}", remove.status);
    assert!(!skill_path.exists());
    let settings_after_remove = fs::read_to_string(&settings_path)?;
    assert!(!settings_after_remove.contains("mimir hook-context"));
    Ok(())
}

#[test]
fn setup_agent_status_requires_claude_precompact_hook() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let settings_path = project.join(".claude/settings.json");
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &settings_path,
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "mimir hook-context"
          }
        ]
      }
    ]
  }
}
"#,
    )?;

    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("status")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("hook")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(status.status.success(), "status: {:?}", status.status);
    let stdout = String::from_utf8(status.stdout)?;
    assert!(stdout.contains("hook=partial"));
    assert!(stdout.contains("reason=mimir_precompact_hook_missing"));
    Ok(())
}

#[test]
fn setup_agent_installs_codex_project_skill_hook_and_feature_flag(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let setup = tmp.path().join("setup");
    let project = tmp.path().join("project");
    write_setup_artifacts(&setup)?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("codex")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--from")
        .arg(&setup)
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("skill=installed"));
    assert!(stdout.contains("hook=installed"));
    assert!(project
        .join(".agents/skills/mimir-checkpoint/SKILL.md")
        .is_file());
    assert!(fs::read_to_string(project.join(".codex/hooks.json"))?.contains("mimir hook-context"));
    assert!(fs::read_to_string(project.join(".codex/config.toml"))?.contains("codex_hooks = true"));
    Ok(())
}

#[test]
fn setup_agent_dry_run_previews_without_writing() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let setup = tmp.path().join("setup");
    let project = tmp.path().join("project");
    write_setup_artifacts(&setup)?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--from")
        .arg(&setup)
        .arg("--project-root")
        .arg(&project)
        .arg("--dry-run")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("mode=dry-run"));
    assert!(stdout.contains("skill=missing"));
    assert!(stdout.contains("action=would_install"));
    assert!(stdout.contains("hook=missing"));
    assert!(!project.join(".claude/skills/mimir-checkpoint").exists());
    assert!(!project.join(".claude/settings.json").exists());
    Ok(())
}

#[test]
fn setup_agent_doctor_reports_action_required_without_writing(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let setup = tmp.path().join("setup");

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("doctor")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--project-root")
        .arg(&project)
        .arg("--from")
        .arg(&setup)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("doctor_status=action_required"));
    assert!(stdout.contains("skill=missing"));
    assert!(stdout.contains("hook=missing"));
    assert!(
        stdout.contains("status_command=mimir setup-agent status --agent claude --scope project")
    );
    assert!(stdout.contains(
        "install_command=mimir setup-agent install --agent claude --scope project --from "
    ));
    assert!(stdout.contains("context_command=mimir context --project-root "));
    assert!(stdout.contains("next_action=mimir setup-agent install"));
    assert!(!project.join(".claude").exists());
    Ok(())
}

#[test]
fn setup_agent_doctor_reports_ready_after_codex_install() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let setup = tmp.path().join("setup");
    let project = tmp.path().join("project");
    write_setup_artifacts(&setup)?;

    let install = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("codex")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--from")
        .arg(&setup)
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(install.status.success(), "status: {:?}", install.status);

    let doctor = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("doctor")
        .arg("--agent")
        .arg("codex")
        .arg("--scope")
        .arg("project")
        .arg("--project-root")
        .arg(&project)
        .arg("--from")
        .arg(&setup)
        .output()?;

    assert!(doctor.status.success(), "status: {:?}", doctor.status);
    let stdout = String::from_utf8(doctor.stdout)?;
    assert!(stdout.contains("doctor_status=ready"));
    assert!(stdout.contains("skill=installed"));
    assert!(stdout.contains("hook=installed"));
    assert!(stdout.contains("codex_config_status=enabled"));
    assert!(stdout.contains("next_action=none"));
    Ok(())
}

#[test]
fn setup_agent_status_reports_codex_partial_reason() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let hook_path = project.join(".codex/hooks.json");
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &hook_path,
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "mimir hook-context"
          }
        ]
      }
    ]
  }
}
"#,
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("status")
        .arg("--agent")
        .arg("codex")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("hook")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("hook=partial"));
    assert!(stdout.contains("reason=codex_hooks_feature_missing"));
    assert!(stdout.contains("config_path="));
    Ok(())
}

#[test]
fn setup_agent_install_codex_disabled_hooks_is_atomic() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let setup = tmp.path().join("setup");
    let project = tmp.path().join("project");
    write_setup_artifacts(&setup)?;
    let config_path = project.join(".codex/config.toml");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&config_path, "[features]\ncodex_hooks = false\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("codex")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("all")
        .arg("--from")
        .arg(&setup)
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("codex hooks are explicitly disabled"));
    assert!(!project.join(".codex/hooks.json").exists());
    assert!(!project.join(".agents/skills/mimir-checkpoint").exists());
    Ok(())
}

#[test]
fn setup_agent_remove_refuses_non_mimir_skill() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let skill_path = project.join(".claude/skills/mimir-checkpoint");
    fs::create_dir_all(&skill_path)?;
    fs::write(
        skill_path.join("SKILL.md"),
        "---\nname: local-user-skill\ndescription: do not remove\n---\n",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("remove")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--features")
        .arg("skill")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("refusing to remove non-Mimir skill target"));
    assert!(skill_path.join("SKILL.md").is_file());
    Ok(())
}

#[test]
fn setup_agent_install_requires_setup_source() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("setup-agent")
        .arg("install")
        .arg("--agent")
        .arg("claude")
        .arg("--scope")
        .arg("project")
        .arg("--project-root")
        .arg(tmp.path())
        .env_remove("MIMIR_AGENT_SETUP_DIR")
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("MIMIR_AGENT_SETUP_DIR"));
    Ok(())
}

#[test]
fn config_init_dry_run_previews_without_writing() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("config")
        .arg("init")
        .arg("--project-root")
        .arg(&project)
        .arg("--operator")
        .arg("hasnobeef")
        .arg("--organization")
        .arg("buildepicshit")
        .arg("--remote-url")
        .arg("git@github.com:buildepicshit/mimir-memory.git")
        .arg("--dry-run")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("mode=dry-run"));
    assert!(stdout.contains("path="));
    assert!(stdout.contains("[storage]"));
    assert!(stdout.contains("[remote]"));
    assert!(stdout.contains("git@github.com:buildepicshit/mimir-memory.git"));
    assert!(!project.join(".mimir/config.toml").exists());
    Ok(())
}

#[test]
fn config_init_writes_project_config_with_remote_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("config")
        .arg("init")
        .arg("--project-root")
        .arg(&project)
        .arg("--operator")
        .arg("hasnobeef")
        .arg("--organization")
        .arg("buildepicshit")
        .arg("--remote-url")
        .arg("git@github.com:buildepicshit/mimir-memory.git")
        .arg("--remote-branch")
        .arg("main")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("mode=written"));
    let config_path = project.join(".mimir/config.toml");
    let config = fs::read_to_string(config_path)?;
    assert!(config.contains("data_root = \"state\""));
    assert!(config.contains("operator = \"hasnobeef\""));
    assert!(config.contains("organization = \"buildepicshit\""));
    assert!(config.contains("[remote]"));
    assert!(config.contains("kind = \"git\""));
    assert!(config.contains("url = \"git@github.com:buildepicshit/mimir-memory.git\""));
    assert!(config.contains("branch = \"main\""));
    assert!(config.contains("after_capture = \"process\""));
    Ok(())
}

#[test]
fn config_init_refuses_to_overwrite_existing_config() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let config_path = project.join(".mimir/config.toml");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&config_path, "[storage]\ndata_root = \"state\"\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("config")
        .arg("init")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("refusing to overwrite existing config"));
    assert_eq!(
        fs::read_to_string(config_path)?,
        "[storage]\ndata_root = \"state\"\n"
    );
    Ok(())
}

#[test]
fn status_reports_local_operator_dashboard() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    submit_test_draft(&drafts_dir, "Remember that status surfaces pending drafts.")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("status")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("status=ok"));
    assert!(stdout.contains("config_status=ready"));
    assert!(stdout.contains("bootstrap_status=ready"));
    assert!(stdout.contains("workspace_status=detected"));
    assert!(stdout.contains("drafts_pending=1"));
    assert!(stdout.contains("remote_status=configured"));
    assert!(stdout.contains("native_setup_claude_project=missing"));
    assert!(stdout.contains("native_setup_codex_project=missing"));
    assert!(stdout.contains("next_action=mimir drafts list --state pending"));
    Ok(())
}

#[test]
fn doctor_reports_public_readiness_actions_without_raw_draft_text(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    submit_test_draft(&drafts_dir, "Do not leak this raw doctor draft.")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("doctor")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("doctor_status=ok"));
    assert!(stdout.contains("doctor_schema=mimir.doctor.v1"));
    assert!(stdout.contains("doctor_readiness=action_required"));
    assert!(stdout.contains("drafts_pending=1"));
    assert!(stdout.contains("doctor_check index=0"));
    assert!(stdout.contains("id=pending_drafts"));
    assert!(stdout.contains("mimir drafts list --state pending"));
    assert!(stdout.contains("id=native_setup_claude_project_missing"));
    assert!(!stdout.contains("mimir setup-agent doctor --agent claude"));
    assert!(!stdout.contains("Do not leak this raw doctor draft"));
    Ok(())
}

#[test]
fn doctor_treats_missing_native_project_setup_as_info() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("doctor")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("doctor_readiness=ready"));
    assert!(stdout.contains("doctor_action_count=0"));
    assert!(stdout.contains("id=native_setup_claude_project_missing"));
    assert!(stdout.contains("id=native_setup_codex_project_missing"));
    assert!(!stdout.contains("mimir setup-agent doctor --agent claude"));
    assert!(!stdout.contains("mimir setup-agent doctor --agent codex"));
    Ok(())
}

#[test]
fn doctor_guides_missing_config_to_config_init() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("doctor")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("doctor_status=ok"));
    assert!(stdout.contains("config_status=missing"));
    assert!(stdout.contains("doctor_readiness=action_required"));
    assert!(stdout.contains("id=config_missing"));
    assert!(stdout.contains("mimir config init --project-root"));
    Ok(())
}

#[test]
fn health_reports_memory_readiness_without_raw_draft_text() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    submit_test_draft(
        &drafts_dir,
        "Do not leak this raw draft into health output.",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("health")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("health_status=ok"));
    assert!(stdout.contains("health_overall_zone=amber"));
    assert!(stdout.contains("config_status=ready"));
    assert!(stdout.contains("workspace_log_status=missing"));
    assert!(stdout.contains("drafts_pending=1"));
    assert!(stdout.contains("oldest_pending_draft_age_ms="));
    assert!(stdout.contains("recall_telemetry_status=unavailable"));
    assert!(stdout.contains("next_action=mimir drafts list --state pending"));
    assert!(!stdout.contains("Do not leak this raw draft"));
    Ok(())
}

#[test]
fn context_command_renders_bounded_records_without_raw_draft_text(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let remote = "https://github.com/buildepicshit/Mimir.git";
    fs::create_dir_all(&project)?;
    write_git_origin(&project, remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    submit_test_draft(&drafts_dir, "Do not leak this raw context draft.")?;

    let workspace_id = WorkspaceId::from_git_remote(remote)?;
    {
        let mut store = Store::open_in_workspace(&data_root, workspace_id)?;
        store.commit_batch(
            r#"(sem @project @launch_state "ready" :src @observation :c 0.95000 :v 2024-04-24)"#,
            ClockTime::try_from_millis(1_772_000_000_000)?,
        )?;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("context")
        .arg("--project-root")
        .arg(&project)
        .arg("--limit")
        .arg("1")
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("context_status=ok"));
    assert!(stdout.contains("context_schema=mimir.context.v1"));
    assert!(stdout.contains("drafts_pending=1"));
    assert!(stdout.contains("rehydrated_record_count=1"));
    assert!(stdout.contains("context_record index=0 source=governed_canonical kind=sem"));
    assert!(stdout.contains(r#"lisp=(sem @project @launch_state "ready""#));
    assert!(!stdout.contains("Do not leak this raw context draft"));
    Ok(())
}

#[test]
fn memory_commands_list_show_and_explain_governed_records() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let remote = "https://github.com/buildepicshit/Mimir.git";
    fs::create_dir_all(&project)?;
    write_git_origin(&project, remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(remote)?;
    {
        let mut store = Store::open_in_workspace(&data_root, workspace_id)?;
        store.commit_batch(
            r#"(sem @project @launch_state "ready" :src @observation :c 0.95000 :v 2024-04-24)"#,
            ClockTime::try_from_millis(1_772_000_000_000)?,
        )?;
        store.commit_batch(
            r#"(sem @project @launch_owner "operator" :src @observation :c 0.90000 :v 2024-04-24)"#,
            ClockTime::try_from_millis(1_772_000_000_001)?,
        )?;
    }

    let list = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("list")
        .arg("--project-root")
        .arg(&project)
        .arg("--limit")
        .arg("1")
        .output()?;
    assert!(list.status.success(), "status: {:?}", list.status);
    let stdout = String::from_utf8(list.stdout)?;
    assert!(stdout.contains("memory_status=ok"));
    assert!(stdout.contains("memory_schema=mimir.memory.v1"));
    assert!(stdout.contains("memory_record_count=1"));
    assert!(stdout.contains("memory_record_truncated=true"));
    assert!(stdout.contains("memory_record index=0 id=@__mem_0 source=governed_canonical kind=sem"));
    assert!(stdout.contains(r#"lisp=(sem @project @launch_state "ready""#));

    let show = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("show")
        .arg("--project-root")
        .arg(&project)
        .arg("--id")
        .arg("@__mem_0")
        .output()?;
    assert!(show.status.success(), "status: {:?}", show.status);
    let stdout = String::from_utf8(show.stdout)?;
    assert!(stdout.contains("memory_show_status=ok"));
    assert!(stdout.contains("memory_id=@__mem_0"));
    assert!(stdout.contains("payload_format=canonical_lisp"));
    assert!(stdout.contains(r#"lisp=(sem @project @launch_state "ready""#));

    let explain = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("explain")
        .arg("--project-root")
        .arg(&project)
        .arg("--id")
        .arg("@__mem_0")
        .output()?;
    assert!(explain.status.success(), "status: {:?}", explain.status);
    let stdout = String::from_utf8(explain.stdout)?;
    assert!(stdout.contains("memory_explain_status=ok"));
    assert!(stdout.contains("memory_id=@__mem_0"));
    assert!(stdout.contains("memory_current=true"));
    assert!(stdout.contains("memory_edge_count=0"));
    assert!(stdout.contains("revoke_command=mimir memory revoke --id @__mem_0 --reason "));

    assert_memory_missing_statuses(&project)?;
    Ok(())
}

fn assert_memory_missing_statuses(project: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let missing_show = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("show")
        .arg("--project-root")
        .arg(project)
        .arg("--id")
        .arg("@missing")
        .output()?;
    assert!(
        missing_show.status.success(),
        "status: {:?}",
        missing_show.status
    );
    let stdout = String::from_utf8(missing_show.stdout)?;
    assert!(stdout.contains("memory_show_status=not_found"));
    assert!(!stdout.contains("memory_show_status=ok"));

    let missing_explain = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("explain")
        .arg("--project-root")
        .arg(project)
        .arg("--id")
        .arg("@missing")
        .output()?;
    assert!(
        missing_explain.status.success(),
        "status: {:?}",
        missing_explain.status
    );
    let stdout = String::from_utf8(missing_explain.stdout)?;
    assert!(stdout.contains("memory_explain_status=not_found"));
    assert!(!stdout.contains("memory_explain_status=ok"));
    Ok(())
}

#[test]
fn memory_revoke_stages_librarian_request_without_touching_log(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    let remote = "https://github.com/buildepicshit/Mimir.git";
    fs::create_dir_all(&project)?;
    write_git_origin(&project, remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(remote)?;
    let workspace_hex = full_workspace_hex(workspace_id);
    {
        let mut store = Store::open_in_workspace(&data_root, workspace_id)?;
        store.commit_batch(
            r#"(sem @project @bad_fact "remove me" :src @observation :c 0.50000 :v 2024-04-24)"#,
            ClockTime::try_from_millis(1_772_000_000_000)?,
        )?;
    }
    let log_path = data_root.join(workspace_hex).join("canonical.log");
    let before_len = fs::metadata(&log_path)?.len();

    let revoke = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("memory")
        .arg("revoke")
        .arg("--project-root")
        .arg(&project)
        .arg("--id")
        .arg("@__mem_0")
        .arg("--reason")
        .arg("operator requested forgetting an incorrect fact")
        .output()?;
    assert!(revoke.status.success(), "status: {:?}", revoke.status);
    let stdout = String::from_utf8(revoke.stdout)?;
    assert!(stdout.contains("memory_revoke_status=staged"));
    assert!(stdout.contains("canonical_write=none"));
    assert!(stdout.contains("draft_state=pending"));
    assert_eq!(fs::metadata(&log_path)?.len(), before_len);

    let pending = fs::read_dir(drafts_dir.join("pending"))?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(pending.len(), 1);
    let draft = fs::read_to_string(pending[0].path())?;
    assert!(draft.contains("append-only revocation"));
    assert!(draft.contains("@__mem_0"));
    assert!(draft.contains("operator requested forgetting an incorrect fact"));
    Ok(())
}

#[test]
fn drafts_commands_report_list_and_show_staged_drafts() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    let id = submit_test_draft(
        &drafts_dir,
        "Remember that drafts list gives operators a queue view.",
    )?;

    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("status")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(status.status.success(), "status: {:?}", status.status);
    let status_stdout = String::from_utf8(status.stdout)?;
    assert!(status_stdout.contains("drafts_pending=1"));
    assert!(status_stdout.contains("drafts_accepted=0"));

    let list = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("list")
        .arg("--state")
        .arg("pending")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(list.status.success(), "status: {:?}", list.status);
    let list_stdout = String::from_utf8(list.stdout)?;
    assert!(list_stdout.contains("state=pending"));
    assert!(list_stdout.contains("count=1"));
    assert!(list_stdout.contains(&format!("id={id}")));
    assert!(list_stdout.contains("source_surface=agent_export"));
    assert!(list_stdout.contains("preview=Remember that drafts list gives operators a queue view."));

    let show = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("show")
        .arg(&id)
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(show.status.success(), "status: {:?}", show.status);
    let show_stdout = String::from_utf8(show.stdout)?;
    assert!(show_stdout.contains(&format!("id={id}")));
    assert!(show_stdout.contains("state=pending"));
    assert!(show_stdout.contains("source_agent=codex"));
    assert!(show_stdout.contains("context_tags=status-test"));
    assert!(show_stdout.contains("raw_text:"));
    assert!(show_stdout.contains("Remember that drafts list gives operators a queue view."));
    Ok(())
}

#[test]
fn drafts_next_shows_oldest_staged_draft() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    let older_id = submit_test_draft_at(
        &drafts_dir,
        "Older pending memory should be reviewed first.",
        UNIX_EPOCH + Duration::from_millis(1_772_000_000_000),
    )?;
    let newer_id = submit_test_draft_at(
        &drafts_dir,
        "Newer pending memory should wait behind older drafts.",
        UNIX_EPOCH + Duration::from_millis(1_772_000_001_000),
    )?;

    let next = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("next")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(next.status.success(), "status: {:?}", next.status);
    let stdout = String::from_utf8(next.stdout)?;
    assert!(stdout.contains(&format!("id={older_id}")));
    assert!(!stdout.contains(&format!("id={newer_id}")));
    assert!(stdout.contains("state=pending"));
    assert!(stdout.contains("submitted_at_unix_ms=1772000000000"));
    assert!(stdout.contains("raw_text:"));
    assert!(stdout.contains("Older pending memory should be reviewed first."));
    Ok(())
}

#[test]
fn drafts_display_sanitizes_terminal_control_sequences() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    let raw = "Safe prefix \x1b[31mred\x1b[0m \x1b]0;owned\x07 title \x08backspace safe suffix.";
    let id = submit_test_draft(&drafts_dir, raw)?;

    let list = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("list")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(list.status.success(), "status: {:?}", list.status);
    let list_stdout = String::from_utf8(list.stdout)?;
    assert!(!list_stdout.contains('\x1b'));
    assert!(!list_stdout.contains('\x07'));
    assert!(!list_stdout.contains('\x08'));
    assert!(list_stdout.contains("preview=Safe prefix red title backspace safe suffix."));

    let show = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("show")
        .arg(&id)
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(show.status.success(), "status: {:?}", show.status);
    let show_stdout = String::from_utf8(show.stdout)?;
    assert!(!show_stdout.contains('\x1b'));
    assert!(!show_stdout.contains('\x07'));
    assert!(!show_stdout.contains('\x08'));
    assert!(show_stdout.contains("Safe prefix red"));
    assert!(show_stdout.contains("title"));
    assert!(show_stdout.contains("backspace safe suffix."));

    let stored = fs::read_to_string(drafts_dir.join("pending").join(format!("{id}.json")))?;
    assert!(stored.contains("\\u001b"));
    assert!(stored.contains("\\u0007"));
    assert!(stored.contains("\\b"));
    Ok(())
}

#[test]
fn drafts_skip_and_quarantine_move_pending_drafts_with_review_reasons(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;
    let skip_id = submit_test_draft_at(
        &drafts_dir,
        "Skip this pending memory because it is a duplicate.",
        UNIX_EPOCH + Duration::from_millis(1_772_000_000_000),
    )?;
    let quarantine_id = submit_test_draft_at(
        &drafts_dir,
        "Quarantine this pending memory because the instruction is ambiguous.",
        UNIX_EPOCH + Duration::from_millis(1_772_000_001_000),
    )?;

    let skip = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("skip")
        .arg(&skip_id)
        .arg("--reason")
        .arg("duplicate/noisy")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(skip.status.success(), "status: {:?}", skip.status);
    let skip_stdout = String::from_utf8(skip.stdout)?;
    assert!(skip_stdout.contains(&format!("id={skip_id}")));
    assert!(skip_stdout.contains("from=pending"));
    assert!(skip_stdout.contains("to=skipped"));
    assert!(skip_stdout.contains("canonical_write=false"));
    assert!(drafts_dir
        .join("skipped")
        .join(format!("{skip_id}.json"))
        .is_file());
    assert!(!drafts_dir
        .join("pending")
        .join(format!("{skip_id}.json"))
        .exists());

    let quarantine = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("quarantine")
        .arg(&quarantine_id)
        .arg("--reason")
        .arg("unsafe/ambiguous")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert!(
        quarantine.status.success(),
        "status: {:?}",
        quarantine.status
    );
    let quarantine_stdout = String::from_utf8(quarantine.stdout)?;
    assert!(quarantine_stdout.contains(&format!("id={quarantine_id}")));
    assert!(quarantine_stdout.contains("from=pending"));
    assert!(quarantine_stdout.contains("to=quarantined"));
    assert!(drafts_dir
        .join("quarantined")
        .join(format!("{quarantine_id}.json"))
        .is_file());

    let skip_review = fs::read_to_string(
        drafts_dir
            .join("reviews")
            .join(format!("{skip_id}-skipped.json")),
    )?;
    assert!(skip_review.contains("\"reason\": \"duplicate/noisy\""));
    assert!(skip_review.contains("\"to\": \"skipped\""));
    let quarantine_review = fs::read_to_string(
        drafts_dir
            .join("reviews")
            .join(format!("{quarantine_id}-quarantined.json")),
    )?;
    assert!(quarantine_review.contains("\"reason\": \"unsafe/ambiguous\""));
    assert!(quarantine_review.contains("\"to\": \"quarantined\""));

    let missing_reason = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("drafts")
        .arg("skip")
        .arg(&quarantine_id)
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;
    assert_eq!(missing_reason.status.code(), Some(2));
    let stderr = String::from_utf8(missing_reason.stderr)?;
    assert!(stderr.contains("drafts skip requires --reason"));
    Ok(())
}

#[test]
fn remote_status_reports_explicit_git_sync_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    let remote_url = tmp.path().join("memory.git");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &remote_url,
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("status")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("remote_kind=git"));
    assert!(stdout.contains("remote_branch=main"));
    assert!(stdout.contains("sync_mode=explicit"));
    assert!(stdout.contains("push_command=mimir remote push"));
    assert!(stdout.contains("pull_command=mimir remote pull"));
    assert!(stdout.contains("workspace_log_relation=missing"));
    assert!(stdout.contains("status_snapshot=local_checkout"));
    assert!(stdout.contains("refresh_status=not_requested"));
    assert!(stdout.contains("refresh_command=mimir remote status --refresh"));
    assert!(stdout.contains("next_action=none"));
    Ok(())
}

#[test]
fn remote_status_refresh_updates_owned_checkout_snapshot() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    Store::open_in_workspace(&data_root, workspace_id)?.commit_batch(
        r#"(sem @mimir @remote_sync "first" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;

    let push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(push.status.success(), "status: {:?}", push.status);

    let external = tmp.path().join("external");
    run_git(
        &[
            "clone",
            "--branch",
            "main",
            &bare.display().to_string(),
            &external.display().to_string(),
        ],
        tmp.path(),
    )?;
    run_git(&["config", "user.name", "Mimir Test"], &external)?;
    run_git(
        &["config", "user.email", "mimir@example.invalid"],
        &external,
    )?;
    let remote_log = external
        .join("workspaces")
        .join(full_workspace_hex(workspace_id))
        .join("canonical.log");
    fs::OpenOptions::new()
        .append(true)
        .open(&remote_log)?
        .write_all(b"remote-suffix")?;
    run_git(&["add", "workspaces"], &external)?;
    run_git(&["commit", "-m", "remote append"], &external)?;
    run_git(&["push", "origin", "main"], &external)?;

    let stale = remote_status_stdout(&project)?;
    assert!(stale.contains("status_snapshot=local_checkout"));
    assert!(stale.contains("refresh_status=not_requested"));
    assert!(stale.contains("workspace_log_relation=synced"));

    let refreshed = remote_status_stdout_with_args(&project, &["--refresh"])?;
    assert!(refreshed.contains("status_snapshot=refreshed_checkout"));
    assert!(refreshed.contains("refresh_status=success"));
    assert!(refreshed.contains("workspace_log_relation=remote_ahead"));
    assert!(refreshed.contains("next_action=mimir remote pull"));
    Ok(())
}

#[test]
fn remote_status_reports_log_remediation_recipes() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    let mut store = Store::open_in_workspace(&data_root, workspace_id)?;
    store.commit_batch(
        r#"(sem @mimir @remote_sync "first" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;

    let local_only = remote_status_stdout(&project)?;
    assert!(local_only.contains("workspace_log_relation=local_only"));
    assert!(local_only.contains("next_action=mimir remote push"));
    assert!(
        local_only.contains("remediation=publish local append-only state with `mimir remote push`")
    );

    let push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(push.status.success(), "status: {:?}", push.status);

    let synced = remote_status_stdout(&project)?;
    assert!(synced.contains("workspace_log_relation=synced"));
    assert!(synced.contains("next_action=none"));

    store.commit_batch(
        r#"(sem @mimir @remote_sync "second" :src @observation :c 0.90000 :v 2024-04-25)"#,
        ClockTime::try_from_millis(1_772_000_000_100)?,
    )?;

    let local_ahead = remote_status_stdout(&project)?;
    assert!(local_ahead.contains("workspace_log_relation=local_ahead"));
    assert!(local_ahead.contains("next_action=mimir remote push"));

    let remote_log_path = status_value(&local_ahead, "remote_workspace_log_path")
        .ok_or("status must include remote workspace log path")?;
    fs::write(remote_log_path, b"divergent-remote-log")?;
    let remote_drafts_dir = status_value(&local_ahead, "remote_drafts_dir")
        .ok_or("status must include remote drafts dir")?;
    let local_conflict = drafts_dir.join("pending").join("conflict.json");
    let remote_conflict = Path::new(remote_drafts_dir)
        .join("pending")
        .join("conflict.json");
    fs::create_dir_all(local_conflict.parent().ok_or("local conflict parent")?)?;
    fs::create_dir_all(remote_conflict.parent().ok_or("remote conflict parent")?)?;
    fs::write(&local_conflict, "{\"origin\":\"local\"}\n")?;
    fs::write(&remote_conflict, "{\"origin\":\"remote\"}\n")?;

    let divergent = remote_status_stdout(&project)?;
    assert!(divergent.contains("workspace_log_relation=diverged"));
    assert!(divergent.contains("draft_conflicts=1"));
    assert!(divergent.contains("draft_remediation=draft file names conflict; rename or quarantine one side before push/pull because draft sync is copy-only"));
    assert!(divergent.contains("next_action=manual_resolution_required"));
    assert!(divergent.contains("remediation=canonical logs diverged; preserve both files, decode both histories, and resolve through the librarian instead of overwriting canonical.log"));
    Ok(())
}

#[test]
fn remote_status_reports_service_boundary_without_git_sync(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    write_git_origin(&project, "https://github.com/buildepicshit/Mimir.git")?;
    write_service_remote_config(
        &project.join(".mimir/config.toml"),
        &tmp.path().join("mimir-data"),
        "https://memory.example.invalid/mimir",
    )?;

    let status = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("status")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(status.status.success(), "status: {:?}", status.status);
    let stdout = String::from_utf8(status.stdout)?;
    assert!(stdout.contains("remote_kind=service"));
    assert!(stdout.contains("remote_url=https://memory.example.invalid/mimir"));
    assert!(stdout.contains("sync_mode=unsupported"));
    assert!(stdout.contains("service_contract_version=1"));
    assert!(stdout.contains("service_status=adapter_not_implemented"));
    assert!(stdout.contains("push_dry_run_command=mimir remote push --dry-run"));
    assert!(stdout.contains("pull_dry_run_command=mimir remote pull --dry-run"));

    let dry_push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--dry-run")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(dry_push.status.success(), "status: {:?}", dry_push.status);
    let dry_push_stdout = String::from_utf8(dry_push.stdout)?;
    assert!(dry_push_stdout.contains("mode=dry-run"));
    assert!(dry_push_stdout.contains("direction=push"));
    assert!(dry_push_stdout.contains("remote_kind=service"));
    assert!(dry_push_stdout.contains("sync_mode=service_adapter_boundary"));
    assert!(dry_push_stdout.contains("service_contract_version=1"));
    assert!(dry_push_stdout.contains("service_status=adapter_not_implemented"));
    assert!(dry_push_stdout.contains("requires_append_only_log_prefix_check=true"));
    assert!(dry_push_stdout.contains("requires_copy_only_draft_sync=true"));
    assert!(dry_push_stdout.contains("requires_librarian_governed_writes=true"));
    assert!(dry_push_stdout.contains("network_request=not_sent"));

    let dry_pull = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("pull")
        .arg("--dry-run")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(dry_pull.status.success(), "status: {:?}", dry_pull.status);
    let dry_pull_stdout = String::from_utf8(dry_pull.stdout)?;
    assert!(dry_pull_stdout.contains("direction=pull"));
    assert!(dry_pull_stdout.contains("service_operation=pull_workspace_state"));

    let push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert_eq!(push.status.code(), Some(2));
    let stderr = String::from_utf8(push.stderr)?;
    assert!(stderr.contains("service remote sync is not implemented"));
    Ok(())
}

#[test]
fn remote_push_copies_governed_log_and_drafts_to_git_remote(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    Store::open_in_workspace(&data_root, workspace_id)?.commit_batch(
        r#"(sem @mimir @remote_sync "explicit" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;
    let pending_dir = drafts_dir.join("pending");
    fs::create_dir_all(&pending_dir)?;
    fs::write(pending_dir.join("draft.json"), "{\"id\":\"draft\"}\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .env_remove("MIMIR_CONFIG_PATH")
        .env_remove("MIMIR_DRAFTS_DIR")
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("direction=push"));
    assert!(stdout.contains("status=synced"));
    assert!(stdout.contains("workspace_log_verified=true"));
    assert!(stdout.contains("drafts_copied=1"));
    let workspace_hex = full_workspace_hex(workspace_id);
    assert_remote_status_reports_synced_checkout(&project)?;
    assert_remote_clone_contains_workspace_state(tmp.path(), &bare, &workspace_hex)?;
    assert_remote_pull_restores_workspace_state(tmp.path(), &bare, project_remote, &workspace_hex)?;
    assert_remote_pull_refuses_divergent_log(tmp.path(), &bare, project_remote, &workspace_hex)?;
    Ok(())
}

#[test]
fn remote_push_rejects_corrupt_workspace_log_before_mirroring(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    Store::open_in_workspace(&data_root, workspace_id)?.commit_batch(
        r#"(sem @mimir @bcdr_backup "remote push verifies source log" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;
    let workspace_hex = full_workspace_hex(workspace_id);
    fs::OpenOptions::new()
        .append(true)
        .open(data_root.join(&workspace_hex).join("canonical.log"))?
        .write_all(b"\xff")?;

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("remote sync integrity check failed"));
    assert!(stderr.contains("verify reported corrupt canonical-log tail"));

    let verify = tmp.path().join("verify-corrupt-rejection");
    run_git(
        &[
            "clone",
            "--branch",
            "main",
            &bare.display().to_string(),
            &verify.display().to_string(),
        ],
        tmp.path(),
    )?;
    assert!(!verify.join("workspaces").join(workspace_hex).exists());
    Ok(())
}

#[test]
fn remote_pull_rejects_corrupt_remote_log_before_restore() -> Result<(), Box<dyn std::error::Error>>
{
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    Store::open_in_workspace(&data_root, workspace_id)?.commit_batch(
        r#"(sem @mimir @bcdr_restore "remote pull verifies source log" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;
    let push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(push.status.success(), "status: {:?}", push.status);

    let workspace_hex = full_workspace_hex(workspace_id);
    let external = tmp.path().join("external-corrupt");
    run_git(
        &[
            "clone",
            "--branch",
            "main",
            &bare.display().to_string(),
            &external.display().to_string(),
        ],
        tmp.path(),
    )?;
    run_git(&["config", "user.name", "Mimir Test"], &external)?;
    run_git(
        &["config", "user.email", "mimir@example.invalid"],
        &external,
    )?;
    fs::OpenOptions::new()
        .append(true)
        .open(
            external
                .join("workspaces")
                .join(&workspace_hex)
                .join("canonical.log"),
        )?
        .write_all(b"\xff")?;
    run_git(&["add", "workspaces"], &external)?;
    run_git(&["commit", "-m", "corrupt remote log"], &external)?;
    run_git(&["push", "origin", "main"], &external)?;

    let restored_project = tmp.path().join("restored-project-corrupt");
    fs::create_dir_all(&restored_project)?;
    write_git_origin(&restored_project, project_remote)?;
    let restored_data = tmp.path().join("restored-data-corrupt");
    let restored_drafts = tmp.path().join("restored-drafts-corrupt");
    write_remote_config(
        &restored_project.join(".mimir/config.toml"),
        &restored_data,
        &restored_drafts,
        &bare,
    )?;

    let pull = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("pull")
        .arg("--project-root")
        .arg(&restored_project)
        .output()?;

    assert_eq!(pull.status.code(), Some(2));
    let stderr = String::from_utf8(pull.stderr)?;
    assert!(stderr.contains("remote sync integrity check failed"));
    assert!(stderr.contains("verify reported corrupt canonical-log tail"));
    assert!(!restored_data
        .join(workspace_hex)
        .join("canonical.log")
        .exists());
    Ok(())
}

#[test]
fn remote_drill_restores_verifies_and_sanity_queries_git_backup(
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let bare = prepare_bare_remote(tmp.path())?;
    let project = tmp.path().join("project");
    fs::create_dir_all(&project)?;
    let project_remote = "https://github.com/buildepicshit/Mimir.git";
    write_git_origin(&project, project_remote)?;
    let data_root = tmp.path().join("mimir-data");
    let drafts_dir = tmp.path().join("mimir-drafts");
    write_remote_config(
        &project.join(".mimir/config.toml"),
        &data_root,
        &drafts_dir,
        &bare,
    )?;

    let workspace_id = WorkspaceId::from_git_remote(project_remote)?;
    Store::open_in_workspace(&data_root, workspace_id)?.commit_batch(
        r#"(sem @mimir @bcdr_restore "remote drill restores governed memory" :src @observation :c 0.90000 :v 2024-04-24)"#,
        ClockTime::try_from_millis(1_772_000_000_000)?,
    )?;
    let pending_dir = drafts_dir.join("pending");
    fs::create_dir_all(&pending_dir)?;
    fs::write(pending_dir.join("draft.json"), "{\"id\":\"draft\"}\n")?;

    let push = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("push")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(push.status.success(), "status: {:?}", push.status);

    let refused = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("drill")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert_eq!(refused.status.code(), Some(2));
    let stderr = String::from_utf8(refused.stderr)?;
    assert!(stderr.contains("rerun with --destructive or --dry-run"));

    let dry_run = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("drill")
        .arg("--dry-run")
        .arg("--project-root")
        .arg(&project)
        .output()?;
    assert!(dry_run.status.success(), "status: {:?}", dry_run.status);
    let dry_run_stdout = String::from_utf8(dry_run.stdout)?;
    assert!(dry_run_stdout.contains("mode=dry-run"));
    assert!(dry_run_stdout.contains("destructive_required=true"));
    assert!(dry_run_stdout.contains("sanity_query=(query :limit 1)"));

    let output = Command::new(env!("CARGO_BIN_EXE_mimir"))
        .arg("remote")
        .arg("drill")
        .arg("--destructive")
        .arg("--project-root")
        .arg(&project)
        .output()?;

    assert!(output.status.success(), "status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("direction=drill"));
    assert!(stdout.contains("status=passed"));
    assert!(stdout.contains("deleted_local_log=true"));
    assert!(stdout.contains("workspace_log_copied=true"));
    assert!(stdout.contains("workspace_log_verified=true"));
    assert!(stdout.contains("verify_tail=clean"));
    assert!(stdout.contains("verify_memory_records=1"));
    assert!(stdout.contains("sanity_query=(query :limit 1)"));
    assert!(stdout.contains("sanity_query_records=1"));

    let workspace_hex = full_workspace_hex(workspace_id);
    assert!(data_root
        .join(workspace_hex)
        .join("canonical.log")
        .is_file());
    assert!(drafts_dir.join("pending").join("draft.json").is_file());
    Ok(())
}
