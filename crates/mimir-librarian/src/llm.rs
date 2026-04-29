//! `LlmInvoker` — the trait over "ask an active agent adapter to
//! structure this prose as canonical Mimir Lisp."
//!
//! Wrapped as a trait so tests can mock the LLM without spawning
//! subprocesses or hitting the operator's native CLI auth. Production
//! impls shell out to native non-interactive adapter surfaces.
//!
//! # Invocation shape
//!
//! [`ClaudeCliInvoker::invoke`] runs:
//!
//! ```text
//! <binary_path> -p --no-session-persistence --model <model>
//!               --system-prompt <system_prompt> <user_message>
//! ```
//!
//! with `stdin` closed, `stdout + stderr` piped, and a
//! [`wait_timeout::ChildExt::wait_timeout`]-bounded wait. Error
//! classification (spawn failure, timeout, non-zero exit, empty
//! stdout) maps to [`LibrarianError::LlmInvocationFailed`] with
//! a specific `message` on every failure path; the caller can
//! match on the enum variant while logs and operator-facing
//! surfaces get the diagnostic message.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wait_timeout::ChildExt as _;

use crate::{LibrarianError, DEFAULT_LLM_TIMEOUT_SECS};

/// Default binary name searched on `PATH`.
const DEFAULT_CLAUDE_BINARY: &str = "claude";
const DEFAULT_CODEX_BINARY: &str = "codex";
const DEFAULT_COPILOT_BINARY: &str = "gh";

/// Maximum number of characters of `stderr` retained in error
/// messages on non-zero exit. Bounded, sufficient for debugging,
/// avoids carrying unbounded operator-controlled data in logs.
const STDERR_TAIL_CHARS: usize = 400;
const TEXT_FILE_BUSY_OS_ERROR: i32 = 26;
const SPAWN_RETRY_COUNT: usize = 3;
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);
static CODEX_RESPONSE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Supported LLM adapter surfaces for librarian draft processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmAdapter {
    /// Claude Code non-interactive print mode.
    Claude,
    /// Codex CLI non-interactive exec mode.
    Codex,
    /// GitHub Copilot CLI prompt mode, normally invoked through `gh copilot`.
    Copilot,
}

impl LlmAdapter {
    /// Parse a user/config adapter name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "copilot" => Some(Self::Copilot),
            _ => None,
        }
    }

    /// Stable adapter name for summaries and config.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
        }
    }

    /// Default executable searched on `PATH` for this adapter.
    #[must_use]
    pub const fn default_binary(self) -> &'static str {
        match self {
            Self::Claude => DEFAULT_CLAUDE_BINARY,
            Self::Codex => DEFAULT_CODEX_BINARY,
            Self::Copilot => DEFAULT_COPILOT_BINARY,
        }
    }
}

/// Ask the LLM to produce a JSON response for a prose draft.
///
/// The `system_prompt` sets the librarian's role and the output
/// schema; the `user_message` carries the wrapped prose draft
/// (typically `<draft>...</draft>` — the envelope is the caller's
/// responsibility). The returned `String` is the LLM's raw stdout,
/// expected (but not verified by this trait) to be a JSON object
/// matching the librarian system prompt's output contract.
///
/// Implementations must be `Send + Sync` so callers can use them
/// from multi-threaded runners.
pub trait LlmInvoker: Send + Sync + std::fmt::Debug {
    /// Run one LLM invocation and return its stdout.
    ///
    /// # Errors
    ///
    /// - [`LibrarianError::LlmInvocationFailed`] if the invocation
    ///   mechanism (typically a subprocess) failed to produce
    ///   usable output. The attached `message` distinguishes spawn
    ///   failure, timeout, non-zero exit, and empty output.
    fn invoke(&self, system_prompt: &str, user_message: &str) -> Result<String, LibrarianError>;
}

/// Production `LlmInvoker` that shells out to the `claude` CLI in
/// non-interactive mode.
///
/// Uses whatever auth the operator's `claude` CLI already has —
/// no `ANTHROPIC_API_KEY` required. See the
/// `feedback_no_api_blocker.md` memory.
///
/// Construction is via [`ClaudeCliInvoker::new`] (takes a model
/// alias; binary defaults to `claude` on `PATH`) with optional
/// [`Self::with_timeout`] and [`Self::with_binary_path`] builders.
#[derive(Debug, Clone)]
pub struct ClaudeCliInvoker {
    model: String,
    timeout: Duration,
    binary_path: PathBuf,
}

impl ClaudeCliInvoker {
    /// Construct with the given Claude model alias (e.g.
    /// `"claude-sonnet-4-6"` or `"claude-opus-4-7"`). Binary
    /// defaults to `claude` resolved via `PATH`; override with
    /// [`Self::with_binary_path`]. Timeout defaults to
    /// [`DEFAULT_LLM_TIMEOUT_SECS`].
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            timeout: Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS),
            binary_path: PathBuf::from(DEFAULT_CLAUDE_BINARY),
        }
    }

    /// Override the invocation timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the path to the `claude` binary. Accepts any
    /// path-convertible type — operators can pin a specific binary
    /// version; tests can point at a shim.
    #[must_use]
    pub fn with_binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = path.into();
        self
    }

    /// The model alias this invoker calls.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The per-invocation timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The binary path this invoker will execute.
    #[must_use]
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    /// Build the argv (excluding argv\[0\], which is the binary
    /// path) for a given invocation. Extracted as a pure function
    /// so the precise flag layout is unit-testable without spawning
    /// a subprocess.
    fn build_argv(&self, system_prompt: &str, user_message: &str) -> Vec<String> {
        vec![
            "-p".to_string(),
            "--no-session-persistence".to_string(),
            "--model".to_string(),
            self.model.clone(),
            "--system-prompt".to_string(),
            system_prompt.to_string(),
            user_message.to_string(),
        ]
    }
}

impl Default for ClaudeCliInvoker {
    /// Default model is Sonnet 4.6 — matches the librarian-prototype
    /// run configuration.
    fn default() -> Self {
        Self::new("claude-sonnet-4-6")
    }
}

/// Production `LlmInvoker` that shells out to Codex CLI in
/// non-interactive mode.
///
/// [`CodexCliInvoker::invoke`] runs:
///
/// ```text
/// <binary_path> exec [--model <model>] --output-last-message <tmpfile> -
/// ```
///
/// with the combined system/user prompt written to stdin. Codex writes
/// its final answer to the temporary `--output-last-message` path,
/// which is then returned as the LLM response.
#[derive(Debug, Clone)]
pub struct CodexCliInvoker {
    model: Option<String>,
    timeout: Duration,
    binary_path: PathBuf,
}

/// Production `LlmInvoker` that shells out to GitHub Copilot CLI.
///
/// By default this invokes `gh copilot -p <prompt>`, matching the
/// supported wrapper available in GitHub CLI. If `binary_path` is set
/// to a non-`gh` executable, it invokes that binary directly as
/// `<binary_path> -p <prompt>` for operators with a standalone
/// Copilot CLI on `PATH`.
#[derive(Debug, Clone)]
pub struct CopilotCliInvoker {
    model: Option<String>,
    timeout: Duration,
    binary_path: PathBuf,
}

impl CopilotCliInvoker {
    /// Construct a Copilot adapter invocation. GitHub Copilot CLI does
    /// not currently expose a stable model flag through `gh copilot`;
    /// the model value is retained for summaries and future direct
    /// binary support.
    #[must_use]
    pub fn new(model: Option<String>) -> Self {
        Self {
            model,
            timeout: Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS),
            binary_path: PathBuf::from(DEFAULT_COPILOT_BINARY),
        }
    }

    /// Override the invocation timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the path to the `gh` or standalone Copilot binary.
    #[must_use]
    pub fn with_binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = path.into();
        self
    }

    /// Configured model, if one was requested.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The per-invocation timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The binary path this invoker will execute.
    #[must_use]
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    fn build_argv(&self, prompt: &str) -> Vec<String> {
        let mut argv = Vec::new();
        if binary_basename(&self.binary_path) == Some("gh") {
            argv.push("copilot".to_string());
        }
        argv.push("-p".to_string());
        argv.push(prompt.to_string());
        argv
    }
}

impl CodexCliInvoker {
    /// Construct a Codex adapter invocation. If `model` is `None`,
    /// Codex uses its configured/default model.
    #[must_use]
    pub fn new(model: Option<String>) -> Self {
        Self {
            model,
            timeout: Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS),
            binary_path: PathBuf::from(DEFAULT_CODEX_BINARY),
        }
    }

    /// Override the invocation timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the path to the `codex` binary.
    #[must_use]
    pub fn with_binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = path.into();
        self
    }

    /// Configured model, if one should be passed explicitly.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The per-invocation timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The binary path this invoker will execute.
    #[must_use]
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    fn build_argv(&self, response_path: &Path) -> Vec<String> {
        let mut argv = vec!["exec".to_string()];
        if let Some(model) = &self.model {
            argv.push("--model".to_string());
            argv.push(model.clone());
        }
        argv.push("--output-last-message".to_string());
        argv.push(response_path.display().to_string());
        argv.push("-".to_string());
        argv
    }
}

impl LlmInvoker for CopilotCliInvoker {
    #[tracing::instrument(
        name = "mimir.librarian.llm.invoke",
        skip_all,
        fields(
            adapter = "copilot",
            model = self.model.as_deref().unwrap_or("configured_default"),
            prompt_bytes = system_prompt.len() + user_message.len(),
            response_bytes = tracing::field::Empty,
            exit_code = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
        ),
    )]
    fn invoke(&self, system_prompt: &str, user_message: &str) -> Result<String, LibrarianError> {
        let started = Instant::now();
        let prompt = format!("{system_prompt}\n\n{user_message}");
        let argv = self.build_argv(&prompt);

        let mut command = Command::new(&self.binary_path);
        command
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_with_retry(&mut command).map_err(|io_err| {
            LibrarianError::LlmInvocationFailed {
                message: format!("failed to spawn {}: {io_err}", self.binary_path.display()),
            }
        })?;

        let wait_outcome = child.wait_timeout(self.timeout);
        let status = match wait_outcome {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("invocation timed out after {}s", self.timeout.as_secs()),
                });
            }
            Err(io_err) => {
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("wait error: {io_err}"),
                });
            }
        };

        let mut stdout = String::new();
        if let Some(mut handle) = child.stdout.take() {
            let _ = handle.read_to_string(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }

        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let span = tracing::Span::current();
        span.record("response_bytes", stdout.len());
        span.record("duration_ms", duration_ms);

        if !status.success() {
            let exit_label = status
                .code()
                .map_or_else(|| "signalled".to_string(), |c| c.to_string());
            span.record("exit_code", exit_label.as_str());
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!(
                    "{} exited {exit_label}: {}",
                    self.binary_path.display(),
                    tail_chars(stderr.trim())
                ),
            });
        }
        span.record("exit_code", 0);

        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!("{} exited 0 with empty stdout", self.binary_path.display()),
            });
        }

        Ok(trimmed.to_string())
    }
}

/// Return at most the last `STDERR_TAIL_CHARS` characters of `s`,
/// as a new `String`. UTF-8-safe (operates on `char` boundaries).
fn tail_chars(s: &str) -> String {
    let char_count = s.chars().count();
    if char_count <= STDERR_TAIL_CHARS {
        return s.to_string();
    }
    let skip = char_count - STDERR_TAIL_CHARS;
    s.chars().skip(skip).collect()
}

fn binary_basename(path: &Path) -> Option<&str> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.to_str())
}

fn remove_response_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn kill_child_and_remove_response(child: &mut Child, response_path: &Path) {
    let _ = child.kill();
    let _ = child.wait();
    remove_response_file(response_path);
}

fn write_prompt_to_child_stdin(
    child: &mut Child,
    prompt: &str,
    binary_path: &Path,
    response_path: &Path,
) -> Result<(), LibrarianError> {
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(io_err) = stdin.write_all(prompt.as_bytes()) {
            kill_child_and_remove_response(child, response_path);
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!(
                    "failed to write prompt to {}: {io_err}",
                    binary_path.display()
                ),
            });
        }
        return Ok(());
    }

    kill_child_and_remove_response(child, response_path);
    Err(LibrarianError::LlmInvocationFailed {
        message: format!("{} stdin was unavailable", binary_path.display()),
    })
}

impl LlmInvoker for CodexCliInvoker {
    #[tracing::instrument(
        name = "mimir.librarian.llm.invoke",
        skip_all,
        fields(
            adapter = "codex",
            model = self.model.as_deref().unwrap_or("configured_default"),
            prompt_bytes = system_prompt.len() + user_message.len(),
            response_bytes = tracing::field::Empty,
            exit_code = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
        ),
    )]
    fn invoke(&self, system_prompt: &str, user_message: &str) -> Result<String, LibrarianError> {
        let started = Instant::now();
        let response_path = next_codex_response_path();
        let argv = self.build_argv(&response_path);
        let prompt = format!("{system_prompt}\n\n{user_message}");

        let mut command = Command::new(&self.binary_path);
        command
            .args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match spawn_with_retry(&mut command) {
            Ok(child) => child,
            Err(io_err) => {
                let _ = fs::remove_file(&response_path);
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("failed to spawn {}: {io_err}", self.binary_path.display()),
                });
            }
        };

        write_prompt_to_child_stdin(&mut child, &prompt, &self.binary_path, &response_path)?;

        let wait_outcome = child.wait_timeout(self.timeout);
        let status = match wait_outcome {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&response_path);
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("invocation timed out after {}s", self.timeout.as_secs()),
                });
            }
            Err(io_err) => {
                let _ = fs::remove_file(&response_path);
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("wait error: {io_err}"),
                });
            }
        };

        let mut stdout = String::new();
        if let Some(mut handle) = child.stdout.take() {
            let _ = handle.read_to_string(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }

        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let span = tracing::Span::current();
        span.record("duration_ms", duration_ms);

        if !status.success() {
            let exit_label = status
                .code()
                .map_or_else(|| "signalled".to_string(), |c| c.to_string());
            span.record("exit_code", exit_label.as_str());
            let _ = fs::remove_file(&response_path);
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!(
                    "{} exited {exit_label}: {}",
                    self.binary_path.display(),
                    tail_chars(stderr.trim())
                ),
            });
        }
        span.record("exit_code", 0);

        let response = match fs::read_to_string(&response_path) {
            Ok(response) => response,
            Err(io_err) if stdout.trim().is_empty() => {
                let _ = fs::remove_file(&response_path);
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!(
                        "{} exited 0 but response file {} could not be read: {io_err}",
                        self.binary_path.display(),
                        response_path.display()
                    ),
                });
            }
            Err(_) => stdout,
        };
        let _ = fs::remove_file(&response_path);
        span.record("response_bytes", response.len());

        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!(
                    "{} exited 0 with empty response",
                    self.binary_path.display()
                ),
            });
        }

        Ok(trimmed.to_string())
    }
}

fn next_codex_response_path() -> PathBuf {
    let counter = CODEX_RESPONSE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mimir-codex-llm-{}-{counter}.txt",
        std::process::id()
    ))
}

impl LlmInvoker for ClaudeCliInvoker {
    #[tracing::instrument(
        name = "mimir.librarian.llm.invoke",
        skip_all,
        fields(
            model = %self.model,
            prompt_bytes = system_prompt.len() + user_message.len(),
            response_bytes = tracing::field::Empty,
            exit_code = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
        ),
    )]
    fn invoke(&self, system_prompt: &str, user_message: &str) -> Result<String, LibrarianError> {
        let started = Instant::now();
        let argv = self.build_argv(system_prompt, user_message);

        let mut command = Command::new(&self.binary_path);
        command
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_with_retry(&mut command).map_err(|io_err| {
            LibrarianError::LlmInvocationFailed {
                message: format!("failed to spawn {}: {io_err}", self.binary_path.display()),
            }
        })?;

        let wait_outcome = child.wait_timeout(self.timeout);
        let status = match wait_outcome {
            Ok(Some(status)) => status,
            Ok(None) => {
                // Timeout — reap the child so we don't leave a zombie.
                let _ = child.kill();
                let _ = child.wait();
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("invocation timed out after {}s", self.timeout.as_secs()),
                });
            }
            Err(io_err) => {
                return Err(LibrarianError::LlmInvocationFailed {
                    message: format!("wait error: {io_err}"),
                });
            }
        };

        // Drain stdout and stderr. The prompt + response sizes we
        // encounter in practice (~10 KB total) are comfortably below
        // typical 64 KB pipe buffers, so reading after wait is safe.
        // If that invariant ever tightens (much larger prompts), a
        // follow-up can switch to threaded reading.
        let mut stdout = String::new();
        if let Some(mut handle) = child.stdout.take() {
            let _ = handle.read_to_string(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }

        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let span = tracing::Span::current();
        span.record("response_bytes", stdout.len());
        span.record("duration_ms", duration_ms);

        if !status.success() {
            let exit_label = status
                .code()
                .map_or_else(|| "signalled".to_string(), |c| c.to_string());
            span.record("exit_code", exit_label.as_str());
            tracing::warn!(
                target: "mimir.librarian.llm.nonzero_exit",
                exit = exit_label.as_str(),
            );
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!(
                    "{} exited {exit_label}: {}",
                    self.binary_path.display(),
                    tail_chars(stderr.trim())
                ),
            });
        }
        span.record("exit_code", 0);

        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Err(LibrarianError::LlmInvocationFailed {
                message: format!("{} exited 0 with empty stdout", self.binary_path.display()),
            });
        }

        Ok(trimmed.to_string())
    }
}

fn spawn_with_retry(command: &mut Command) -> Result<Child, std::io::Error> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(err)
                if err.raw_os_error() == Some(TEXT_FILE_BUSY_OS_ERROR)
                    && attempt < SPAWN_RETRY_COUNT =>
            {
                attempt += 1;
                thread::sleep(SPAWN_RETRY_DELAY);
            }
            result => return result,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::TempDir;

    /// Write an executable shell-script shim to a fresh tempdir.
    /// The shim simulates `claude` for integration tests. Returns
    /// the tempdir (kept alive for the test) and the shim path.
    #[cfg(unix)]
    fn make_shim(script_body: &str) -> (TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("claude");
        let tmp_path = dir.path().join(".claude.tmp");
        let mut file = fs::File::create(&tmp_path).expect("create shim");
        file.write_all(script_body.as_bytes()).expect("write shim");
        file.sync_all().expect("sync shim");
        drop(file);
        let mut perms = fs::metadata(&tmp_path).expect("stat shim").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp_path, perms).expect("chmod shim");
        fs::rename(&tmp_path, &path).expect("publish shim");
        (dir, path)
    }

    /// Write a shell-script shim through a Windows command wrapper.
    /// GitHub's Windows runners provide Git Bash `sh`; the wrapper
    /// keeps the test fixture bodies identical across platforms.
    #[cfg(windows)]
    fn make_shim(script_body: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let script_path = dir.path().join("claude.sh");
        let path = dir.path().join("claude.cmd");
        let tmp_script_path = dir.path().join(".claude.sh.tmp");
        let tmp_path = dir.path().join(".claude.cmd.tmp");
        let mut script = fs::File::create(&tmp_script_path).expect("create shim script");
        script
            .write_all(script_body.as_bytes())
            .expect("write shim script");
        script.sync_all().expect("sync shim script");
        drop(script);
        let mut command = fs::File::create(&tmp_path).expect("create shim command");
        command
            .write_all(b"@echo off\r\nsh \"%~dp0claude.sh\" %*\r\n")
            .expect("write shim command");
        command.sync_all().expect("sync shim command");
        drop(command);
        fs::rename(&tmp_script_path, &script_path).expect("publish shim script");
        fs::rename(&tmp_path, &path).expect("publish shim command");
        (dir, path)
    }

    #[test]
    fn construction() {
        let invoker = ClaudeCliInvoker::new("claude-opus-4-7");
        assert_eq!(invoker.model(), "claude-opus-4-7");
        assert_eq!(
            invoker.timeout(),
            Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS)
        );
        assert_eq!(invoker.binary_path(), Path::new("claude"));
    }

    #[test]
    fn default_is_sonnet_4_6() {
        let invoker = ClaudeCliInvoker::default();
        assert_eq!(invoker.model(), "claude-sonnet-4-6");
    }

    #[test]
    fn builders_override_defaults() {
        let invoker = ClaudeCliInvoker::new("m")
            .with_timeout(Duration::from_secs(7))
            .with_binary_path("/tmp/fake-claude");
        assert_eq!(invoker.timeout(), Duration::from_secs(7));
        assert_eq!(invoker.binary_path(), Path::new("/tmp/fake-claude"));
    }

    #[test]
    fn argv_shape_is_exact() {
        let invoker = ClaudeCliInvoker::new("claude-opus-4-7");
        let argv = invoker.build_argv("SYS PROMPT", "USER MSG");
        assert_eq!(
            argv,
            vec![
                "-p",
                "--no-session-persistence",
                "--model",
                "claude-opus-4-7",
                "--system-prompt",
                "SYS PROMPT",
                "USER MSG",
            ]
        );
    }

    #[test]
    fn codex_argv_shape_is_exact() {
        let invoker = CodexCliInvoker::new(Some("gpt-5.4".to_string()));
        let argv = invoker.build_argv(Path::new("/tmp/mimir-codex-response.txt"));
        assert_eq!(
            argv,
            vec![
                "exec",
                "--model",
                "gpt-5.4",
                "--output-last-message",
                "/tmp/mimir-codex-response.txt",
                "-",
            ]
        );
    }

    #[test]
    fn copilot_argv_shape_uses_gh_wrapper_by_default() {
        let invoker = CopilotCliInvoker::new(None);
        let argv = invoker.build_argv("SYS\n\nUSER");
        assert_eq!(argv, vec!["copilot", "-p", "SYS\n\nUSER"]);
    }

    #[test]
    fn copilot_argv_shape_supports_direct_binary() {
        let invoker = CopilotCliInvoker::new(None).with_binary_path("/tmp/copilot");
        let argv = invoker.build_argv("SYS\n\nUSER");
        assert_eq!(argv, vec!["-p", "SYS\n\nUSER"]);
    }

    #[test]
    fn adapter_parse_supports_processing_surfaces() {
        assert_eq!(LlmAdapter::parse("claude"), Some(LlmAdapter::Claude));
        assert_eq!(LlmAdapter::parse("codex"), Some(LlmAdapter::Codex));
        assert_eq!(LlmAdapter::parse("copilot"), Some(LlmAdapter::Copilot));
        assert_eq!(LlmAdapter::parse("defer"), None);
    }

    #[test]
    fn tail_chars_returns_whole_short_string() {
        assert_eq!(tail_chars("hello"), "hello");
    }

    #[test]
    fn tail_chars_truncates_long_string_to_last_n() {
        let long = "x".repeat(STDERR_TAIL_CHARS + 100);
        let tail = tail_chars(&long);
        assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS);
    }

    #[test]
    fn tail_chars_preserves_utf8() {
        let s = "日".repeat(STDERR_TAIL_CHARS + 10);
        let tail = tail_chars(&s);
        assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS);
        assert!(tail.chars().all(|c| c == '日'));
    }

    // ---- Integration tests via shim binary ----

    #[test]
    fn invoke_success_returns_trimmed_stdout() {
        let (_dir, shim) = make_shim("#!/bin/sh\necho '{\"records\":[],\"notes\":\"ok\"}'\n");
        let invoker = ClaudeCliInvoker::default().with_binary_path(&shim);
        let result = invoker.invoke("sys", "usr").expect("shim always succeeds");
        assert_eq!(result, r#"{"records":[],"notes":"ok"}"#);
    }

    #[test]
    fn codex_invoke_reads_output_last_message_file() {
        let (_dir, shim) = make_shim(
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"records\":[],\"notes\":\"codex\"}' > \"$3\"\n",
        );
        let invoker = CodexCliInvoker::new(None).with_binary_path(&shim);
        let result = invoker.invoke("sys", "usr").expect("shim always succeeds");
        assert_eq!(result, r#"{"records":[],"notes":"codex"}"#);
    }

    #[test]
    fn copilot_invoke_returns_trimmed_stdout() {
        let (_dir, shim) = make_shim("#!/bin/sh\necho '{\"records\":[],\"notes\":\"copilot\"}'\n");
        let invoker = CopilotCliInvoker::new(None).with_binary_path(&shim);
        let result = invoker.invoke("sys", "usr").expect("shim always succeeds");
        assert_eq!(result, r#"{"records":[],"notes":"copilot"}"#);
    }

    #[test]
    fn invoke_nonzero_exit_is_classified() {
        let (_dir, shim) = make_shim("#!/bin/sh\necho 'something broke' >&2\nexit 7\n");
        let invoker = ClaudeCliInvoker::default().with_binary_path(&shim);
        let err = invoker
            .invoke("sys", "usr")
            .expect_err("shim always exits 7");
        let LibrarianError::LlmInvocationFailed { message } = err else {
            panic!("expected LlmInvocationFailed, got {err:?}");
        };
        assert!(message.contains("exited 7"), "message was: {message}");
        assert!(
            message.contains("something broke"),
            "stderr tail missing: {message}"
        );
    }

    #[test]
    fn invoke_empty_stdout_is_rejected() {
        let (_dir, shim) = make_shim("#!/bin/sh\nexit 0\n");
        let invoker = ClaudeCliInvoker::default().with_binary_path(&shim);
        let err = invoker.invoke("sys", "usr").expect_err("empty stdout");
        let LibrarianError::LlmInvocationFailed { message } = err else {
            panic!("expected LlmInvocationFailed, got {err:?}");
        };
        assert!(message.contains("empty stdout"), "message was: {message}");
    }

    #[test]
    fn invoke_timeout_kills_child_and_reports() {
        let (_dir, shim) = make_shim("#!/bin/sh\nsleep 5\n");
        let invoker = ClaudeCliInvoker::default()
            .with_binary_path(&shim)
            .with_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let err = invoker.invoke("sys", "usr").expect_err("must time out");
        // Timeout path must return promptly; give generous slack for
        // slow CI hosts but bound below the shim's 5 s sleep.
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "timeout path took too long: {:?}",
            started.elapsed()
        );
        let LibrarianError::LlmInvocationFailed { message } = err else {
            panic!("expected LlmInvocationFailed, got {err:?}");
        };
        assert!(message.contains("timed out"), "message was: {message}");
    }

    #[test]
    fn invoke_missing_binary_returns_spawn_error() {
        let invoker = ClaudeCliInvoker::default()
            .with_binary_path("/nonexistent/definitely-not-a-claude-binary");
        let err = invoker.invoke("sys", "usr").expect_err("binary is missing");
        let LibrarianError::LlmInvocationFailed { message } = err else {
            panic!("expected LlmInvocationFailed, got {err:?}");
        };
        assert!(
            message.contains("failed to spawn"),
            "message was: {message}"
        );
    }

    /// Demonstrates the trait is mockable — used by every future test
    /// that exercises the librarian without spawning `claude`.
    #[test]
    fn trait_is_mockable() {
        #[derive(Debug)]
        struct MockInvoker {
            canned_response: String,
        }
        impl LlmInvoker for MockInvoker {
            fn invoke(&self, _sys: &str, _usr: &str) -> Result<String, LibrarianError> {
                Ok(self.canned_response.clone())
            }
        }
        let mock = MockInvoker {
            canned_response: r#"{"records": [], "notes": "mock"}"#.to_string(),
        };
        let out = mock.invoke("sys", "usr").expect("mock never errors");
        assert!(out.contains("mock"));
    }
}
