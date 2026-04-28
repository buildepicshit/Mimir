//! `mimir-librarian` binary — CLI entry point.
//!
//! Six subcommands:
//!
//! - `submit` — stage one scope-aware draft into `pending/`.
//! - `sweep` — stage explicit memory files/directories into `pending/`.
//! - `run` — one-shot: process all drafts in `pending/`, exit.
//! - `watch` — long-running: watch `pending/` for new drafts and
//!   process them as they arrive.
//! - `quorum` — record file-backed quorum episodes and participant
//!   outputs for later synthesis.
//! - `copilot` — read Copilot CLI's local session store as untrusted
//!   recall and optional draft input.
//!
//! # Status
//!
//! `run` currently performs lifecycle-safe one-shot processing with
//! bounded LLM validation retry and durable canonical commit. `watch`
//! repeats that same run path on a configurable polling cadence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mimir_core::ClockTime;
use mimir_librarian::{
    run_once, ClaudeCliInvoker, ConsensusLevel, DecisionStatus, DedupPolicy,
    DeferredDraftProcessor, Draft, DraftMetadata, DraftRunSummary, DraftSourceSurface, DraftStore,
    LibrarianConfig, LibrarianError, ParticipantVote, QuorumAdapterRequest, QuorumEpisode,
    QuorumEpisodeState, QuorumParticipant, QuorumParticipantOutput, QuorumResult, QuorumRound,
    QuorumStore, RawArchiveDraftProcessor, RetryingDraftProcessor, SupersessionConflictPolicy,
    VoteChoice, QUORUM_SCHEMA_VERSION,
};
use wait_timeout::ChildExt as _;

mod copilot_session_store;

const DEFAULT_WATCH_POLL_SECS: u64 = 30;
const DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS: u64 = 300;

const USAGE: &str = "\
mimir-librarian — prose drafts → canonical log.

Usage:
    mimir-librarian submit   --text TEXT [--drafts-dir PATH]
                              [--source-surface cli|claude-memory|codex-memory|mcp|directory|repo-handoff|agent-export|consensus-quorum|copilot-session-store]
                              [--agent NAME] [--project NAME] [--operator NAME]
                              [--provenance URI] [--tag TAG]...
    mimir-librarian sweep    --path PATH [--drafts-dir PATH]
                              --source-surface claude-memory|codex-memory|directory|repo-handoff|agent-export
                              [--agent NAME] [--project NAME] [--operator NAME]
                              [--tag TAG]...
    mimir-librarian run      [--drafts-dir PATH] [--workspace PATH]
                              [--stale-processing-secs N]
                              [--max-retries N] [--llm-timeout-secs N]
                              [--dedup-valid-at-window-secs N]
                              [--review-conflicts] [--archive-raw] [--defer]
    mimir-librarian watch    [--drafts-dir PATH] [--workspace PATH]
                              [--poll-secs N] [--iterations N]
                              [run flags...]
    mimir-librarian quorum   create --quorum-dir PATH --id ID
                              --requester NAME --question TEXT
                              --participant ID:ADAPTER:PERSONA[:MODEL]...
                              [--requested-at-unix-ms N]
                              [--target-project NAME] [--target-scope SCOPE]
                              [--evidence-policy TEXT] [--provenance URI]
    mimir-librarian quorum   pilot-plan --quorum-dir PATH --episode-id ID
                              --through-round independent|critique|revision
                              --out-dir PATH --drafts-dir PATH
                              --synthesizer claude|codex
                              [--adapter-binary claude=PATH] [--adapter-binary codex=PATH]
                              [--synthesizer-binary PATH] [--timeout-secs N]
                              [--require-proposed-drafts N]
                              [--project NAME] [--operator NAME] [--tag TAG]...
    mimir-librarian quorum   pilot-status --manifest-file PATH
    mimir-librarian quorum   pilot-run --manifest-file PATH
    mimir-librarian quorum   pilot-review --manifest-file PATH
                              --reviewer ID --decision pass|needs-work|fail
                              --summary TEXT [--finding info|warning|blocker:TEXT]...
                              [--next-action TEXT]...
    mimir-librarian quorum   pilot-summary --manifest-file PATH
    mimir-librarian quorum   append-output --quorum-dir PATH
                              --episode-id ID --output-id ID --participant-id ID
                              --round independent|critique|revision
                              (--prompt TEXT | --prompt-file PATH)
                              (--response TEXT | --response-file PATH)
                              [--submitted-at-unix-ms N]
                              [--visible-prior-output-id ID]... [--evidence URI]...
    mimir-librarian quorum   append-status-output --status-file PATH
                              [--submitted-at-unix-ms N]
    mimir-librarian quorum   outputs --quorum-dir PATH --episode-id ID
                              --round independent|critique|revision
    mimir-librarian quorum   visible --quorum-dir PATH --episode-id ID
                              --round independent|critique|revision
    mimir-librarian quorum   adapter-request --quorum-dir PATH --episode-id ID
                              --participant-id ID --round independent|critique|revision
    mimir-librarian quorum   adapter-plan --quorum-dir PATH --episode-id ID
                              --participant-id ID --round independent|critique|revision
                              --adapter claude|codex --out-dir PATH
                              [--binary NAME] [--output-id ID]
    mimir-librarian quorum   adapter-run --quorum-dir PATH --episode-id ID
                              --participant-id ID --round independent|critique|revision
                              --adapter claude|codex --out-dir PATH
                              [--binary NAME] [--output-id ID] [--timeout-secs N]
    mimir-librarian quorum   adapter-run-round --quorum-dir PATH --episode-id ID
                              --round independent|critique|revision --out-dir PATH
                              [--adapter-binary claude=PATH] [--adapter-binary codex=PATH]
                              [--timeout-secs N]
    mimir-librarian quorum   adapter-run-rounds --quorum-dir PATH --episode-id ID
                              --through-round independent|critique|revision --out-dir PATH
                              [--adapter-binary claude=PATH] [--adapter-binary codex=PATH]
                              [--timeout-secs N] [--submitted-at-unix-ms N]
    mimir-librarian quorum   synthesize-plan --quorum-dir PATH --episode-id ID
                              --adapter claude|codex --out-dir PATH
                              [--binary NAME]
    mimir-librarian quorum   synthesize-run --quorum-dir PATH --episode-id ID
                              --adapter claude|codex --out-dir PATH
                              [--binary NAME] [--timeout-secs N]
    mimir-librarian quorum   accept-synthesis --quorum-dir PATH
                              (--result-file PATH | --status-file PATH)
                              [--episode-id ID]
    mimir-librarian quorum   synthesize --quorum-dir PATH --episode-id ID
                              --recommendation TEXT
                              --decision-status recommend|split|needs-evidence|reject|unsafe
                              --consensus-level unanimous|strong-majority|weak-majority|contested|abstained
                              --confidence N
                              [--supporting-point TEXT]... [--dissenting-point TEXT]...
                              [--unresolved-question TEXT]... [--evidence URI]...
                              [--participant-vote PARTICIPANT:agree|disagree|abstain:CONFIDENCE:RATIONALE]...
                              [--proposed-memory-draft TEXT]...
    mimir-librarian quorum   submit-drafts --quorum-dir PATH --drafts-dir PATH
                              --episode-id ID
                              [--project NAME] [--operator NAME] [--tag TAG]...
    mimir-librarian copilot  schema-check [--db PATH]
    mimir-librarian copilot  recent|files|checkpoints [--db PATH]
                              [--repo OWNER/REPO|all] [--repo-root PATH] [--limit N]
    mimir-librarian copilot  search --query TEXT [--db PATH]
                              [--repo OWNER/REPO|all] [--repo-root PATH] [--limit N]
    mimir-librarian copilot  submit-drafts --drafts-dir PATH [--db PATH]
                              [--repo OWNER/REPO|all] [--repo-root PATH] [--limit N]
                              [--project NAME] [--operator NAME] [--tag TAG]...
    mimir-librarian --help

`submit` is wired and writes a v2 draft JSON envelope to `pending/`.
`sweep` imports explicit file/directory paths as untrusted pending drafts.
`run` is lifecycle-safe and validates LLM output with bounded retry.
`--dedup-valid-at-window-secs` controls same-fact valid_at duplicate skipping.
`--review-conflicts` writes supersession conflicts to `drafts/conflicts/` and quarantines them.
`--archive-raw` deterministically commits raw drafts as pending-verification evidence without LLM work.
`--defer` claims and returns drafts without invoking the LLM.
`watch` repeats `run` on a polling cadence; `--iterations` bounds it for scripts/tests.
`quorum` records file-backed quorum artifacts; `pilot-plan` writes a replayable
manifest, and adapter execution still requires explicit status/result gates.
`copilot` opens Copilot CLI's local SQLite session store read-only, validates schema
before every query, and stages only untrusted drafts through the librarian path.
";

fn main() -> ExitCode {
    init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("submit") => cmd_submit(&args[1..], SystemTime::now()),
        Some("sweep") => cmd_sweep(&args[1..], SystemTime::now()),
        Some("run") => cmd_run(&args[1..], SystemTime::now()),
        Some("watch") => cmd_watch(&args[1..]),
        Some("quorum") => cmd_quorum(&args[1..]),
        Some("copilot") => cmd_copilot(&args[1..], SystemTime::now()),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("mimir-librarian: unknown subcommand '{other}'\n");
            eprintln!("{USAGE}");
            ExitCode::from(64) // EX_USAGE
        }
        None => {
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
    }
}

fn init_tracing() {
    // Stderr subscriber with `info` default; `RUST_LOG` overrides.
    // Install failures are swallowed — another subscriber may be
    // installed by an embedder; we never want tracing setup to
    // take the binary down.
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn cmd_run(args: &[String], now: SystemTime) -> ExitCode {
    match run_from_args(args, now) {
        Ok(outcome) => match serde_json::to_string(&outcome) {
            Ok(json) => {
                println!("{json}");
                if outcome.summary.deferred > 0 {
                    ExitCode::from(70)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("mimir-librarian run failed to render JSON: {err}");
                ExitCode::from(70)
            }
        },
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian run: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian run failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian run failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn cmd_watch(args: &[String]) -> ExitCode {
    match watch_from_args(args, |outcome| match serde_json::to_string(outcome) {
        Ok(json) => println!("{json}"),
        Err(err) => eprintln!("mimir-librarian watch failed to render JSON: {err}"),
    }) {
        Ok(_) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian watch: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian watch failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian watch failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn cmd_submit(args: &[String], submitted_at: SystemTime) -> ExitCode {
    match submit_from_args(args, submitted_at) {
        Ok(outcome) => {
            println!(
                "{}",
                serde_json::json!({
                    "id": outcome.id,
                    "path": outcome.path,
                })
            );
            ExitCode::SUCCESS
        }
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian submit: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian submit failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian submit failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn cmd_sweep(args: &[String], submitted_at: SystemTime) -> ExitCode {
    match sweep_from_args(args, submitted_at) {
        Ok(outcome) => {
            println!(
                "{}",
                serde_json::json!({
                    "submitted": outcome.submitted,
                    "skipped_empty": outcome.skipped_empty,
                    "drafts": outcome.drafts,
                })
            );
            ExitCode::SUCCESS
        }
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian sweep: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian sweep failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian sweep failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn cmd_quorum(args: &[String]) -> ExitCode {
    match quorum_from_args(args, SystemTime::now()) {
        Ok(outcome) => match serde_json::to_string(&outcome) {
            Ok(json) => {
                let exit_code = quorum_outcome_exit_code(&outcome);
                println!("{json}");
                exit_code
            }
            Err(err) => {
                eprintln!("mimir-librarian quorum failed to render JSON: {err}");
                ExitCode::from(70)
            }
        },
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian quorum: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian quorum failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian quorum failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn cmd_copilot(args: &[String], submitted_at: SystemTime) -> ExitCode {
    match copilot_session_store::copilot_session_store_from_args(args, submitted_at) {
        Ok(outcome) => match serde_json::to_string(&outcome) {
            Ok(json) => {
                println!("{json}");
                if outcome.is_failure() {
                    ExitCode::from(70)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("mimir-librarian copilot failed to render JSON: {err}");
                ExitCode::from(70)
            }
        },
        Err(CliError::Usage(message)) => {
            eprintln!("mimir-librarian copilot: {message}\n");
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
        Err(CliError::Librarian(err)) => {
            eprintln!("mimir-librarian copilot failed: {err}");
            ExitCode::from(70)
        }
        Err(CliError::Copilot(err)) => {
            eprintln!("mimir-librarian copilot failed: {err}");
            ExitCode::from(70)
        }
    }
}

fn quorum_outcome_exit_code(outcome: &QuorumCliOutcome) -> ExitCode {
    match outcome {
        QuorumCliOutcome::PilotRun { run } if !run.success => ExitCode::from(70),
        _ => ExitCode::SUCCESS,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubmitOutcome {
    id: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SweepOutcome {
    submitted: usize,
    skipped_empty: usize,
    drafts: Vec<SweepDraftOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct SweepDraftOutcome {
    id: String,
    path: String,
    provenance_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct RunOutcome {
    processor: &'static str,
    summary: DraftRunSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchOutcome {
    iterations: u64,
    last: Option<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QuorumCliOutcome {
    EpisodeCreated {
        id: String,
        path: String,
    },
    OutputAppended {
        episode_id: String,
        output_id: String,
        path: String,
    },
    StatusOutputsAppended {
        status_path: String,
        appended: usize,
        outputs: Vec<QuorumStatusOutputAppend>,
    },
    OutputsLoaded {
        episode_id: String,
        round: String,
        outputs: Vec<QuorumParticipantOutput>,
    },
    VisibleOutputs {
        episode_id: String,
        round: String,
        outputs: Vec<QuorumParticipantOutput>,
    },
    AdapterRequest {
        request: Box<QuorumAdapterRequest>,
    },
    AdapterPlan {
        plan: Box<QuorumAdapterPlan>,
    },
    AdapterRun {
        run: Box<QuorumAdapterRun>,
    },
    AdapterRoundRun {
        round_run: Box<QuorumAdapterRoundRun>,
    },
    AdapterRoundsRun {
        rounds_run: Box<QuorumAdapterRoundsRun>,
    },
    PilotPlan {
        plan: Box<QuorumPilotPlan>,
    },
    PilotStatus {
        status: Box<QuorumPilotStatus>,
    },
    PilotRun {
        run: Box<QuorumPilotRun>,
    },
    PilotReview {
        review: Box<QuorumPilotReview>,
    },
    PilotSummary {
        summary: Box<QuorumPilotSummary>,
    },
    SynthesisPlan {
        plan: Box<QuorumSynthesisPlan>,
    },
    SynthesisRun {
        run: Box<QuorumSynthesisRun>,
    },
    ResultSaved {
        episode_id: String,
        path: String,
    },
    DraftsSubmitted {
        episode_id: String,
        submitted: usize,
        drafts: Vec<SweepDraftOutcome>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumStatusOutputAppend {
    episode_id: String,
    output_id: String,
    participant_id: String,
    round: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumPilotPlan {
    schema_version: u32,
    episode_id: String,
    quorum_dir: String,
    out_dir: String,
    drafts_dir: String,
    through_round: String,
    synthesizer_adapter: String,
    timeout_secs: u64,
    #[serde(default)]
    required_proposed_drafts: usize,
    participants: Vec<QuorumParticipant>,
    manifest_path: String,
    artifacts: QuorumPilotArtifacts,
    steps: Vec<QuorumPilotStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumPilotArtifacts {
    round_statuses: Vec<QuorumPilotRoundArtifact>,
    synthesis_status_path: String,
    synthesis_result_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumPilotRoundArtifact {
    round: String,
    status_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumPilotStep {
    name: String,
    description: String,
    argv: Vec<String>,
    writes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum QuorumPilotGateStatus {
    Pending,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumPilotStatus {
    schema_version: u32,
    episode_id: String,
    manifest_path: String,
    complete: bool,
    overall_status: QuorumPilotGateStatus,
    gates: Vec<QuorumPilotGate>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumPilotGate {
    name: String,
    status: QuorumPilotGateStatus,
    detail: String,
    artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumPilotRun {
    schema_version: u32,
    episode_id: String,
    manifest_path: String,
    success: bool,
    executed_steps: Vec<String>,
    skipped_steps: Vec<String>,
    failed_step: Option<String>,
    error: Option<String>,
    final_status: QuorumPilotStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuorumPilotReviewDecision {
    Pass,
    NeedsWork,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumPilotReviewFinding {
    severity: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumPilotReview {
    schema_version: u32,
    episode_id: String,
    manifest_path: String,
    review_path: String,
    reviewed_at_unix_ms: u64,
    reviewer: String,
    decision: QuorumPilotReviewDecision,
    summary: String,
    findings: Vec<QuorumPilotReviewFinding>,
    next_actions: Vec<String>,
    status_at_review: QuorumPilotStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumPilotSummary {
    schema_version: u32,
    episode_id: String,
    manifest_path: String,
    complete: bool,
    overall_status: QuorumPilotGateStatus,
    required_proposed_drafts: usize,
    proposed_memory_drafts: usize,
    submitted_drafts: usize,
    result_status: String,
    result_path: Option<String>,
    review_status: String,
    review_path: String,
    review_decision: Option<QuorumPilotReviewDecision>,
    next_action: String,
    gates: Vec<QuorumPilotGate>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumAdapterPlan {
    schema_version: u32,
    adapter: String,
    episode_id: String,
    participant_id: String,
    round: String,
    request_path: String,
    prompt_path: String,
    response_path: String,
    status_path: String,
    stdin_path: String,
    stdout_path: Option<String>,
    stdout_capture_path: String,
    stderr_capture_path: String,
    argv: Vec<String>,
    append_output_argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumAdapterRun {
    schema_version: u32,
    status: QuorumAdapterRunStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumAdapterRunStatus {
    schema_version: u32,
    adapter: String,
    episode_id: String,
    participant_id: String,
    round: String,
    status_path: String,
    request_path: String,
    prompt_path: String,
    response_path: String,
    stdout_capture_path: String,
    stderr_capture_path: String,
    success: bool,
    timed_out: bool,
    exit_code: Option<i32>,
    duration_ms: u64,
    response_bytes: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    append_output_argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumAdapterRoundRun {
    schema_version: u32,
    episode_id: String,
    round: String,
    status_path: String,
    success: bool,
    completed: usize,
    failed: usize,
    timed_out: usize,
    statuses: Vec<QuorumAdapterRunStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumAdapterRoundsRun {
    schema_version: u32,
    episode_id: String,
    through_round: String,
    success: bool,
    rounds: Vec<QuorumAdapterRoundRun>,
    appended: Vec<QuorumStatusOutputAppend>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumSynthesisPlan {
    schema_version: u32,
    adapter: String,
    quorum_dir: String,
    episode_id: String,
    transcript_path: String,
    prompt_path: String,
    result_path: String,
    status_path: String,
    stdin_path: String,
    stdout_path: Option<String>,
    stdout_capture_path: String,
    stderr_capture_path: String,
    argv: Vec<String>,
    accept_synthesis_argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QuorumSynthesisRun {
    schema_version: u32,
    status: QuorumSynthesisRunStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuorumProcessStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct QuorumSynthesisRunStatus {
    schema_version: u32,
    adapter: String,
    episode_id: String,
    status_path: String,
    transcript_path: String,
    prompt_path: String,
    result_path: String,
    stdout_capture_path: String,
    stderr_capture_path: String,
    success: bool,
    #[serde(default)]
    process_status: Option<QuorumProcessStatus>,
    timed_out: bool,
    exit_code: Option<i32>,
    duration_ms: u64,
    #[serde(default)]
    result_valid: bool,
    #[serde(default)]
    validation_error: Option<String>,
    result_bytes: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    accept_synthesis_argv: Vec<String>,
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    Librarian(LibrarianError),
    Copilot(copilot_session_store::CopilotSessionStoreError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "{message}"),
            Self::Librarian(err) => write!(formatter, "{err}"),
            Self::Copilot(err) => write!(formatter, "{err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<LibrarianError> for CliError {
    fn from(value: LibrarianError) -> Self {
        Self::Librarian(value)
    }
}

fn run_from_args(args: &[String], now: SystemTime) -> Result<RunOutcome, CliError> {
    let mut cfg = LibrarianConfig::default();
    let mut defer = false;
    let mut archive_raw = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--drafts-dir" => {
                cfg.drafts_dir = take_value(args, &mut i, "--drafts-dir")?.into();
            }
            "--workspace" => {
                cfg.workspace_log = take_value(args, &mut i, "--workspace")?.into();
            }
            "--stale-processing-secs" => {
                let value = take_value(args, &mut i, "--stale-processing-secs")?;
                let secs = value.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!(
                        "--stale-processing-secs must be an integer: {value}"
                    ))
                })?;
                cfg.processing_stale_after = Duration::from_secs(secs);
            }
            "--max-retries" => {
                let value = take_value(args, &mut i, "--max-retries")?;
                cfg.max_retries_per_record = value.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--max-retries must be an integer: {value}"))
                })?;
            }
            "--llm-timeout-secs" => {
                let value = take_value(args, &mut i, "--llm-timeout-secs")?;
                let secs = value.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--llm-timeout-secs must be an integer: {value}"))
                })?;
                cfg.llm_timeout = Duration::from_secs(secs);
            }
            "--dedup-valid-at-window-secs" => {
                let value = take_value(args, &mut i, "--dedup-valid-at-window-secs")?;
                let secs = value.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!(
                        "--dedup-valid-at-window-secs must be an integer: {value}"
                    ))
                })?;
                cfg.dedup_valid_at_window = Duration::from_secs(secs);
            }
            "--defer" => {
                defer = true;
            }
            "--archive-raw" => {
                archive_raw = true;
            }
            "--review-conflicts" => {
                cfg.review_conflicts = true;
            }
            other => {
                return Err(CliError::Usage(format!("unknown option '{other}'")));
            }
        }
        i += 1;
    }

    if defer && archive_raw {
        return Err(CliError::Usage(
            "--archive-raw cannot be combined with --defer".to_string(),
        ));
    }

    let store = DraftStore::new(&cfg.drafts_dir);
    if defer {
        let mut processor = DeferredDraftProcessor;
        let summary = run_once(&store, &mut processor, now, cfg.processing_stale_after)?;
        return Ok(RunOutcome {
            processor: "deferred",
            summary,
        });
    }
    if archive_raw {
        let clock = clock_time_from_system_time(now)?;
        let mut processor = RawArchiveDraftProcessor::new_at(clock, &cfg.workspace_log)?;
        let summary = run_once(&store, &mut processor, now, cfg.processing_stale_after)?;
        return Ok(RunOutcome {
            processor: "archive_raw",
            summary,
        });
    }

    let invoker = ClaudeCliInvoker::default().with_timeout(cfg.llm_timeout);
    let mut processor =
        RetryingDraftProcessor::new(invoker, cfg.max_retries_per_record, &cfg.workspace_log)?
            .with_dedup_policy(DedupPolicy {
                valid_at_window: cfg.dedup_valid_at_window,
            });
    if cfg.review_conflicts {
        processor = processor.with_conflict_policy(SupersessionConflictPolicy::Review {
            dir: cfg.drafts_dir.join("conflicts"),
        });
    }
    let summary = run_once(&store, &mut processor, now, cfg.processing_stale_after)?;
    Ok(RunOutcome {
        processor: "retrying_llm",
        summary,
    })
}

fn watch_from_args<F>(args: &[String], mut on_outcome: F) -> Result<WatchOutcome, CliError>
where
    F: FnMut(&RunOutcome),
{
    let mut run_args = Vec::new();
    let mut poll_interval = Duration::from_secs(DEFAULT_WATCH_POLL_SECS);
    let mut max_iterations: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--poll-secs" => {
                let value = take_value(args, &mut i, "--poll-secs")?;
                let secs = parse_positive_u64("--poll-secs", &value)?;
                poll_interval = Duration::from_secs(secs);
            }
            "--iterations" => {
                let value = take_value(args, &mut i, "--iterations")?;
                max_iterations = Some(parse_positive_u64("--iterations", &value)?);
            }
            other => {
                run_args.push(other.to_string());
            }
        }
        i += 1;
    }

    let limit = max_iterations.unwrap_or(u64::MAX);
    let mut iterations = 0;
    let mut last = None;
    while iterations < limit {
        let outcome = run_from_args(&run_args, SystemTime::now())?;
        on_outcome(&outcome);
        last = Some(outcome);
        iterations += 1;
        if iterations < limit {
            std::thread::sleep(poll_interval);
        }
    }

    Ok(WatchOutcome { iterations, last })
}

fn submit_from_args(args: &[String], submitted_at: SystemTime) -> Result<SubmitOutcome, CliError> {
    let default_cfg = LibrarianConfig::default();
    let mut drafts_dir = default_cfg.drafts_dir;
    let mut raw_text: Option<String> = None;
    let mut metadata = DraftMetadata::new(DraftSourceSurface::Cli, submitted_at);
    metadata.provenance_uri = Some("cli://mimir-librarian/submit".to_string());

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--text" => {
                raw_text = Some(take_value(args, &mut i, "--text")?);
            }
            "--drafts-dir" => {
                drafts_dir = take_value(args, &mut i, "--drafts-dir")?.into();
            }
            "--source-surface" => {
                let value = take_value(args, &mut i, "--source-surface")?;
                metadata.source_surface = DraftSourceSurface::parse(&value)
                    .ok_or_else(|| CliError::Usage(format!("unknown source surface '{value}'")))?;
            }
            "--agent" => {
                metadata.source_agent = Some(take_value(args, &mut i, "--agent")?);
            }
            "--project" => {
                metadata.source_project = Some(take_value(args, &mut i, "--project")?);
            }
            "--operator" => {
                metadata.operator = Some(take_value(args, &mut i, "--operator")?);
            }
            "--provenance" => {
                metadata.provenance_uri = Some(take_value(args, &mut i, "--provenance")?);
            }
            "--tag" => {
                metadata
                    .context_tags
                    .push(take_value(args, &mut i, "--tag")?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown option '{other}'")));
            }
        }
        i += 1;
    }

    let raw_text = raw_text.ok_or_else(|| CliError::Usage("--text is required".to_string()))?;
    if raw_text.trim().is_empty() {
        return Err(CliError::Usage("--text must not be empty".to_string()));
    }

    let draft = Draft::with_metadata(raw_text, metadata);
    let path = DraftStore::new(drafts_dir).submit(&draft)?;
    Ok(SubmitOutcome {
        id: draft.id().to_hex(),
        path: path.display().to_string(),
    })
}

fn sweep_from_args(args: &[String], submitted_at: SystemTime) -> Result<SweepOutcome, CliError> {
    let default_cfg = LibrarianConfig::default();
    let mut drafts_dir = default_cfg.drafts_dir;
    let mut source_path: Option<PathBuf> = None;
    let mut metadata = DraftMetadata::new(DraftSourceSurface::Directory, submitted_at);
    let mut source_surface_seen = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--path" => {
                source_path = Some(take_value(args, &mut i, "--path")?.into());
            }
            "--drafts-dir" => {
                drafts_dir = take_value(args, &mut i, "--drafts-dir")?.into();
            }
            "--source-surface" => {
                let value = take_value(args, &mut i, "--source-surface")?;
                let source_surface = DraftSourceSurface::parse(&value)
                    .ok_or_else(|| CliError::Usage(format!("unknown source surface '{value}'")))?;
                if !is_sweep_source_surface(source_surface) {
                    return Err(CliError::Usage(format!(
                        "source surface '{value}' is not valid for sweep"
                    )));
                }
                metadata.source_surface = source_surface;
                source_surface_seen = true;
            }
            "--agent" => {
                metadata.source_agent = Some(take_value(args, &mut i, "--agent")?);
            }
            "--project" => {
                metadata.source_project = Some(take_value(args, &mut i, "--project")?);
            }
            "--operator" => {
                metadata.operator = Some(take_value(args, &mut i, "--operator")?);
            }
            "--tag" => {
                metadata
                    .context_tags
                    .push(take_value(args, &mut i, "--tag")?);
            }
            "--provenance" => {
                return Err(CliError::Usage(
                    "--provenance is per swept file; use --path for sweep provenance".to_string(),
                ));
            }
            other => {
                return Err(CliError::Usage(format!("unknown option '{other}'")));
            }
        }
        i += 1;
    }

    let source_path =
        source_path.ok_or_else(|| CliError::Usage("--path is required".to_string()))?;
    if !source_surface_seen {
        return Err(CliError::Usage(
            "--source-surface is required for sweep".to_string(),
        ));
    }
    fill_default_agent(&mut metadata);

    let files = collect_sweep_files(&source_path)?;
    let store = DraftStore::new(drafts_dir);
    let mut outcome = SweepOutcome {
        submitted: 0,
        skipped_empty: 0,
        drafts: Vec::new(),
    };

    for file in files {
        let raw_text = fs::read_to_string(&file).map_err(LibrarianError::from)?;
        if raw_text.trim().is_empty() {
            outcome.skipped_empty += 1;
            continue;
        }

        let mut file_metadata = metadata.clone();
        let provenance_uri = file_uri(&file);
        file_metadata.provenance_uri = Some(provenance_uri.clone());
        let draft = Draft::with_metadata(raw_text, file_metadata);
        let path = store.submit(&draft)?;
        outcome.submitted += 1;
        outcome.drafts.push(SweepDraftOutcome {
            id: draft.id().to_hex(),
            path: path.display().to_string(),
            provenance_uri,
        });
    }

    Ok(outcome)
}

fn quorum_from_args(args: &[String], now: SystemTime) -> Result<QuorumCliOutcome, CliError> {
    let Some(command) = args.first() else {
        return Err(CliError::Usage("quorum subcommand is required".to_string()));
    };
    match command.as_str() {
        "create" => quorum_create_from_args(&args[1..], now),
        "pilot-plan" => quorum_pilot_plan_from_args(&args[1..]),
        "pilot-status" => quorum_pilot_status_from_args(&args[1..]),
        "pilot-run" => quorum_pilot_run_from_args(&args[1..], now),
        "pilot-review" => quorum_pilot_review_from_args(&args[1..], now),
        "pilot-summary" => quorum_pilot_summary_from_args(&args[1..]),
        "append-output" => quorum_append_output_from_args(&args[1..], now),
        "append-status-output" => quorum_append_status_output_from_args(&args[1..], now),
        "outputs" => quorum_outputs_from_args(&args[1..]),
        "visible" => quorum_visible_from_args(&args[1..]),
        "adapter-request" => quorum_adapter_request_from_args(&args[1..]),
        "adapter-plan" => quorum_adapter_plan_from_args(&args[1..]),
        "adapter-run" => quorum_adapter_run_from_args(&args[1..]),
        "adapter-run-round" => quorum_adapter_run_round_from_args(&args[1..]),
        "adapter-run-rounds" => quorum_adapter_run_rounds_from_args(&args[1..], now),
        "synthesize-plan" => quorum_synthesize_plan_from_args(&args[1..]),
        "synthesize-run" => quorum_synthesize_run_from_args(&args[1..]),
        "accept-synthesis" => quorum_accept_synthesis_from_args(&args[1..]),
        "synthesize" => quorum_synthesize_from_args(&args[1..]),
        "submit-drafts" => quorum_submit_drafts_from_args(&args[1..], now),
        other => Err(CliError::Usage(format!(
            "unknown quorum subcommand '{other}'"
        ))),
    }
}

fn quorum_create_from_args(args: &[String], now: SystemTime) -> Result<QuorumCliOutcome, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut id: Option<String> = None;
    let mut requested_at_unix_ms: Option<u64> = None;
    let mut requester: Option<String> = None;
    let mut question: Option<String> = None;
    let mut target_project: Option<String> = None;
    let mut target_scope: Option<String> = None;
    let mut evidence_policy = "operator_supplied".to_string();
    let mut provenance_uri: Option<String> = None;
    let mut participants = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--id" => {
                id = Some(take_value(args, &mut i, "--id")?);
            }
            "--requested-at-unix-ms" => {
                let value = take_value(args, &mut i, "--requested-at-unix-ms")?;
                requested_at_unix_ms = Some(parse_u64("--requested-at-unix-ms", &value)?);
            }
            "--requester" => {
                requester = Some(take_value(args, &mut i, "--requester")?);
            }
            "--question" => {
                question = Some(take_value(args, &mut i, "--question")?);
            }
            "--target-project" => {
                target_project = Some(take_value(args, &mut i, "--target-project")?);
            }
            "--target-scope" => {
                target_scope = Some(take_value(args, &mut i, "--target-scope")?);
            }
            "--evidence-policy" => {
                evidence_policy = take_value(args, &mut i, "--evidence-policy")?;
            }
            "--provenance" => {
                provenance_uri = Some(take_value(args, &mut i, "--provenance")?);
            }
            "--participant" => {
                let value = take_value(args, &mut i, "--participant")?;
                participants.push(parse_quorum_participant(&value)?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum create option '{other}'"
                )));
            }
        }
        i += 1;
    }

    let quorum_dir = require_path(quorum_dir, "--quorum-dir")?;
    let id = require_non_empty(id, "--id")?;
    let requester = require_non_empty(requester, "--requester")?;
    let question = require_non_empty(question, "--question")?;
    if participants.is_empty() {
        return Err(CliError::Usage(
            "--participant is required at least once".to_string(),
        ));
    }

    let episode = QuorumEpisode {
        schema_version: QUORUM_SCHEMA_VERSION,
        requested_at_unix_ms: requested_at_unix_ms.unwrap_or_else(|| system_time_to_unix_ms(now)),
        provenance_uri: provenance_uri.unwrap_or_else(|| format!("quorum://episode/{id}")),
        id: id.clone(),
        requester,
        question,
        target_project,
        target_scope,
        evidence_policy,
        state: QuorumEpisodeState::Requested,
        participants,
    };
    let path = QuorumStore::new(quorum_dir).create_episode(&episode)?;
    Ok(QuorumCliOutcome::EpisodeCreated {
        id,
        path: path.display().to_string(),
    })
}

struct QuorumPilotPlanArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    through_round: QuorumRound,
    out_dir: PathBuf,
    drafts_dir: PathBuf,
    synthesizer_adapter: String,
    adapter_binaries: BTreeMap<String, String>,
    synthesizer_binary: Option<String>,
    timeout_secs: u64,
    required_proposed_drafts: usize,
    project: Option<String>,
    operator: Option<String>,
    tags: Vec<String>,
}

struct QuorumPilotReviewArgs {
    manifest_file: PathBuf,
    reviewer: String,
    decision: QuorumPilotReviewDecision,
    summary: String,
    findings: Vec<QuorumPilotReviewFinding>,
    next_actions: Vec<String>,
}

fn quorum_pilot_plan_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_pilot_plan_args(args)?;
    Ok(QuorumCliOutcome::PilotPlan {
        plan: Box::new(build_quorum_pilot_plan(&parsed)?),
    })
}

fn parse_quorum_pilot_plan_args(args: &[String]) -> Result<QuorumPilotPlanArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut through_round: Option<QuorumRound> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut drafts_dir: Option<PathBuf> = None;
    let mut synthesizer_adapter: Option<String> = None;
    let mut adapter_binaries = BTreeMap::new();
    let mut synthesizer_binary: Option<String> = None;
    let mut timeout_secs: Option<u64> = None;
    let mut required_proposed_drafts: Option<usize> = None;
    let mut project: Option<String> = None;
    let mut operator: Option<String> = None;
    let mut tags = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--through-round" => {
                let value = take_value(args, &mut i, "--through-round")?;
                through_round = Some(parse_quorum_round(&value)?);
            }
            "--out-dir" => {
                out_dir = Some(take_value(args, &mut i, "--out-dir")?.into());
            }
            "--drafts-dir" => {
                drafts_dir = Some(take_value(args, &mut i, "--drafts-dir")?.into());
            }
            "--synthesizer" => {
                let value = take_value(args, &mut i, "--synthesizer")?;
                synthesizer_adapter = Some(parse_quorum_adapter(&value)?);
            }
            "--adapter-binary" => {
                let value = take_value(args, &mut i, "--adapter-binary")?;
                let (adapter, binary) = parse_adapter_binary(&value)?;
                if adapter_binaries.insert(adapter.clone(), binary).is_some() {
                    return Err(CliError::Usage(format!(
                        "--adapter-binary supplied more than once for adapter '{adapter}'"
                    )));
                }
            }
            "--synthesizer-binary" => {
                synthesizer_binary = Some(take_value(args, &mut i, "--synthesizer-binary")?);
            }
            "--timeout-secs" => {
                let value = take_value(args, &mut i, "--timeout-secs")?;
                timeout_secs = Some(parse_positive_u64("--timeout-secs", &value)?);
            }
            "--require-proposed-drafts" => {
                let value = take_value(args, &mut i, "--require-proposed-drafts")?;
                required_proposed_drafts = Some(parse_usize("--require-proposed-drafts", &value)?);
            }
            "--project" => {
                project = Some(take_value(args, &mut i, "--project")?);
            }
            "--operator" => {
                operator = Some(take_value(args, &mut i, "--operator")?);
            }
            "--tag" => {
                tags.push(take_value(args, &mut i, "--tag")?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum pilot-plan option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumPilotPlanArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        through_round: through_round
            .ok_or_else(|| CliError::Usage("--through-round is required".to_string()))?,
        out_dir: require_path(out_dir, "--out-dir")?,
        drafts_dir: require_path(drafts_dir, "--drafts-dir")?,
        synthesizer_adapter: require_non_empty(synthesizer_adapter, "--synthesizer")?,
        adapter_binaries,
        synthesizer_binary,
        timeout_secs: timeout_secs.unwrap_or(DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS),
        required_proposed_drafts: required_proposed_drafts.unwrap_or(0),
        project,
        operator,
        tags,
    })
}

fn build_quorum_pilot_plan(parsed: &QuorumPilotPlanArgs) -> Result<QuorumPilotPlan, CliError> {
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&parsed.episode_id)?;
    let artifacts = build_quorum_pilot_artifacts(parsed);
    let steps = build_quorum_pilot_steps(parsed, &artifacts);
    let manifest_path = artifacts_manifest_path(&parsed.out_dir, &parsed.episode_id);
    fs::create_dir_all(&parsed.out_dir).map_err(LibrarianError::from)?;

    let plan = QuorumPilotPlan {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: parsed.episode_id.clone(),
        quorum_dir: parsed.quorum_dir.display().to_string(),
        out_dir: parsed.out_dir.display().to_string(),
        drafts_dir: parsed.drafts_dir.display().to_string(),
        through_round: quorum_round_name(parsed.through_round).to_string(),
        synthesizer_adapter: parsed.synthesizer_adapter.clone(),
        timeout_secs: parsed.timeout_secs,
        required_proposed_drafts: parsed.required_proposed_drafts,
        participants: episode.participants,
        manifest_path: manifest_path.display().to_string(),
        artifacts,
        steps,
    };
    write_quorum_pilot_manifest(&plan)?;
    Ok(plan)
}

fn build_quorum_pilot_artifacts(parsed: &QuorumPilotPlanArgs) -> QuorumPilotArtifacts {
    let synthesis_paths = synthesis_plan_paths(&parsed.out_dir, &parsed.episode_id);
    QuorumPilotArtifacts {
        round_statuses: quorum_rounds_through(parsed.through_round)
            .into_iter()
            .map(|round| QuorumPilotRoundArtifact {
                round: quorum_round_name(round).to_string(),
                status_path: adapter_round_status_path(&parsed.out_dir, &parsed.episode_id, round)
                    .display()
                    .to_string(),
            })
            .collect(),
        synthesis_status_path: synthesis_paths.status.display().to_string(),
        synthesis_result_path: synthesis_paths.result.display().to_string(),
    }
}

fn build_quorum_pilot_steps(
    parsed: &QuorumPilotPlanArgs,
    artifacts: &QuorumPilotArtifacts,
) -> Vec<QuorumPilotStep> {
    vec![
        QuorumPilotStep {
            name: "run_rounds".to_string(),
            description: "Execute participant rounds and append only successful status artifacts."
                .to_string(),
            argv: pilot_run_rounds_argv(parsed),
            writes: artifacts
                .round_statuses
                .iter()
                .map(|artifact| artifact.status_path.clone())
                .collect(),
        },
        QuorumPilotStep {
            name: "run_synthesis".to_string(),
            description: "Run the synthesizer into a proposed result artifact.".to_string(),
            argv: pilot_run_synthesis_argv(parsed),
            writes: vec![
                artifacts.synthesis_status_path.clone(),
                artifacts.synthesis_result_path.clone(),
            ],
        },
        QuorumPilotStep {
            name: "accept_synthesis".to_string(),
            description: "Validate the successful synthesis status before saving result.json."
                .to_string(),
            argv: pilot_accept_synthesis_argv(parsed, artifacts),
            writes: vec![QuorumStore::new(&parsed.quorum_dir)
                .result_artifact_path(&parsed.episode_id)
                .display()
                .to_string()],
        },
        QuorumPilotStep {
            name: "submit_drafts".to_string(),
            description: "Submit accepted proposed memories as consensus_quorum drafts."
                .to_string(),
            argv: pilot_submit_drafts_argv(parsed),
            writes: vec![parsed.drafts_dir.display().to_string()],
        },
    ]
}

fn pilot_run_rounds_argv(parsed: &QuorumPilotPlanArgs) -> Vec<String> {
    let mut argv = vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "adapter-run-rounds".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
        "--through-round".to_string(),
        quorum_round_name(parsed.through_round).to_string(),
        "--out-dir".to_string(),
        parsed.out_dir.display().to_string(),
    ];
    for (adapter, binary) in &parsed.adapter_binaries {
        argv.push("--adapter-binary".to_string());
        argv.push(format!("{adapter}={binary}"));
    }
    argv.push("--timeout-secs".to_string());
    argv.push(parsed.timeout_secs.to_string());
    argv
}

fn pilot_run_synthesis_argv(parsed: &QuorumPilotPlanArgs) -> Vec<String> {
    let mut argv = vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "synthesize-run".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
        "--adapter".to_string(),
        parsed.synthesizer_adapter.clone(),
        "--out-dir".to_string(),
        parsed.out_dir.display().to_string(),
    ];
    if let Some(binary) = parsed
        .synthesizer_binary
        .as_ref()
        .or_else(|| parsed.adapter_binaries.get(&parsed.synthesizer_adapter))
    {
        argv.push("--binary".to_string());
        argv.push(binary.clone());
    }
    argv.push("--timeout-secs".to_string());
    argv.push(parsed.timeout_secs.to_string());
    argv
}

fn pilot_accept_synthesis_argv(
    parsed: &QuorumPilotPlanArgs,
    artifacts: &QuorumPilotArtifacts,
) -> Vec<String> {
    vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "accept-synthesis".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
        "--status-file".to_string(),
        artifacts.synthesis_status_path.clone(),
    ]
}

fn pilot_submit_drafts_argv(parsed: &QuorumPilotPlanArgs) -> Vec<String> {
    let mut argv = vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "submit-drafts".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--drafts-dir".to_string(),
        parsed.drafts_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
    ];
    if let Some(project) = &parsed.project {
        argv.push("--project".to_string());
        argv.push(project.clone());
    }
    if let Some(operator) = &parsed.operator {
        argv.push("--operator".to_string());
        argv.push(operator.clone());
    }
    for tag in &parsed.tags {
        argv.push("--tag".to_string());
        argv.push(tag.clone());
    }
    argv
}

fn artifacts_manifest_path(out_dir: &Path, episode_id: &str) -> PathBuf {
    out_dir.join(format!("{}-pilot-plan.json", safe_file_stem(episode_id)))
}

fn write_quorum_pilot_manifest(plan: &QuorumPilotPlan) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(plan).map_err(LibrarianError::from)?;
    fs::write(&plan.manifest_path, bytes).map_err(LibrarianError::from)?;
    Ok(())
}

fn quorum_pilot_status_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let manifest_file = parse_quorum_pilot_status_args(args)?;
    let plan = load_quorum_pilot_manifest(&manifest_file)?;
    Ok(QuorumCliOutcome::PilotStatus {
        status: Box::new(build_quorum_pilot_status(&manifest_file, &plan)),
    })
}

fn quorum_pilot_run_from_args(
    args: &[String],
    now: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let manifest_file = parse_quorum_pilot_status_args(args)?;
    let plan = load_quorum_pilot_manifest(&manifest_file)?;
    Ok(QuorumCliOutcome::PilotRun {
        run: Box::new(execute_quorum_pilot_manifest(&manifest_file, &plan, now)?),
    })
}

fn quorum_pilot_review_from_args(
    args: &[String],
    now: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_pilot_review_args(args)?;
    let plan = load_quorum_pilot_manifest(&parsed.manifest_file)?;
    Ok(QuorumCliOutcome::PilotReview {
        review: Box::new(write_quorum_pilot_review(&parsed, &plan, now)?),
    })
}

fn quorum_pilot_summary_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let manifest_file = parse_quorum_pilot_manifest_args(args, "pilot-summary")?;
    let plan = load_quorum_pilot_manifest(&manifest_file)?;
    Ok(QuorumCliOutcome::PilotSummary {
        summary: Box::new(build_quorum_pilot_summary(&manifest_file, &plan)?),
    })
}

fn parse_quorum_pilot_status_args(args: &[String]) -> Result<PathBuf, CliError> {
    parse_quorum_pilot_manifest_args(args, "pilot-status")
}

fn parse_quorum_pilot_manifest_args(args: &[String], command: &str) -> Result<PathBuf, CliError> {
    let mut manifest_file: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest-file" => {
                manifest_file = Some(take_value(args, &mut i, "--manifest-file")?.into());
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum {command} option '{other}'"
                )));
            }
        }
        i += 1;
    }
    require_path(manifest_file, "--manifest-file")
}

fn parse_quorum_pilot_review_args(args: &[String]) -> Result<QuorumPilotReviewArgs, CliError> {
    let mut manifest_file: Option<PathBuf> = None;
    let mut reviewer: Option<String> = None;
    let mut decision: Option<QuorumPilotReviewDecision> = None;
    let mut summary: Option<String> = None;
    let mut findings = Vec::new();
    let mut next_actions = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest-file" => {
                manifest_file = Some(take_value(args, &mut i, "--manifest-file")?.into());
            }
            "--reviewer" => {
                reviewer = Some(take_value(args, &mut i, "--reviewer")?);
            }
            "--decision" => {
                let value = take_value(args, &mut i, "--decision")?;
                decision = Some(parse_quorum_pilot_review_decision(&value)?);
            }
            "--summary" => {
                summary = Some(take_value(args, &mut i, "--summary")?);
            }
            "--finding" => {
                let value = take_value(args, &mut i, "--finding")?;
                findings.push(parse_quorum_pilot_review_finding(&value)?);
            }
            "--next-action" => {
                next_actions.push(take_value(args, &mut i, "--next-action")?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum pilot-review option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumPilotReviewArgs {
        manifest_file: require_path(manifest_file, "--manifest-file")?,
        reviewer: require_non_empty(reviewer, "--reviewer")?,
        decision: decision.ok_or_else(|| CliError::Usage("--decision is required".to_string()))?,
        summary: require_non_empty(summary, "--summary")?,
        findings,
        next_actions,
    })
}

fn load_quorum_pilot_manifest(path: &Path) -> Result<QuorumPilotPlan, CliError> {
    let bytes = fs::read(path).map_err(LibrarianError::from)?;
    let plan: QuorumPilotPlan = serde_json::from_slice(&bytes).map_err(LibrarianError::from)?;
    if plan.schema_version != QUORUM_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported pilot manifest schema version {}; expected {QUORUM_SCHEMA_VERSION}",
            plan.schema_version
        )));
    }
    Ok(plan)
}

fn write_quorum_pilot_review(
    parsed: &QuorumPilotReviewArgs,
    plan: &QuorumPilotPlan,
    now: SystemTime,
) -> Result<QuorumPilotReview, CliError> {
    let status = build_quorum_pilot_status(&parsed.manifest_file, plan);
    if parsed.decision == QuorumPilotReviewDecision::Pass && !status.complete {
        return Err(CliError::Usage(
            "--decision pass requires a complete pilot-status".to_string(),
        ));
    }
    let review_path = pilot_review_path(plan);
    if let Some(parent) = review_path.parent() {
        fs::create_dir_all(parent).map_err(LibrarianError::from)?;
    }
    let review = QuorumPilotReview {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: plan.episode_id.clone(),
        manifest_path: parsed.manifest_file.display().to_string(),
        review_path: review_path.display().to_string(),
        reviewed_at_unix_ms: system_time_to_unix_ms(now),
        reviewer: parsed.reviewer.clone(),
        decision: parsed.decision,
        summary: parsed.summary.clone(),
        findings: parsed.findings.clone(),
        next_actions: parsed.next_actions.clone(),
        status_at_review: status,
    };
    let bytes = serde_json::to_vec_pretty(&review).map_err(LibrarianError::from)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&review_path)
        .map_err(LibrarianError::from)?;
    file.write_all(&bytes).map_err(LibrarianError::from)?;
    Ok(review)
}

fn build_quorum_pilot_summary(
    manifest_file: &Path,
    plan: &QuorumPilotPlan,
) -> Result<QuorumPilotSummary, CliError> {
    let status = build_quorum_pilot_status(manifest_file, plan);
    let store = QuorumStore::new(&plan.quorum_dir);
    let result_path = store.result_artifact_path(&plan.episode_id);
    let result = store.load_result(&plan.episode_id).ok();
    let review_path = pilot_review_path(plan);
    let review_decision = load_quorum_pilot_review_decision(&review_path)?;
    let submitted_drafts = count_quorum_drafts(Path::new(&plan.drafts_dir), &plan.episode_id)?;
    let next_action = pilot_summary_next_action(&status, review_decision, &review_path);

    Ok(QuorumPilotSummary {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: plan.episode_id.clone(),
        manifest_path: manifest_file.display().to_string(),
        complete: status.complete,
        overall_status: status.overall_status,
        required_proposed_drafts: plan.required_proposed_drafts,
        proposed_memory_drafts: result
            .as_ref()
            .map_or(0, |result| result.proposed_memory_drafts.len()),
        submitted_drafts,
        result_status: if result_path.is_file() {
            "present".to_string()
        } else {
            "missing".to_string()
        },
        result_path: result_path
            .is_file()
            .then(|| result_path.display().to_string()),
        review_status: if review_path.is_file() {
            "present".to_string()
        } else {
            "missing".to_string()
        },
        review_path: review_path.display().to_string(),
        review_decision,
        next_action,
        gates: status.gates,
    })
}

fn load_quorum_pilot_review_decision(
    review_path: &Path,
) -> Result<Option<QuorumPilotReviewDecision>, CliError> {
    if !review_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(review_path).map_err(LibrarianError::from)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(LibrarianError::from)?;
    let Some(decision) = value.get("decision").and_then(serde_json::Value::as_str) else {
        return Err(CliError::Usage(format!(
            "pilot review artifact '{}' is missing a decision",
            review_path.display()
        )));
    };
    parse_quorum_pilot_review_decision(decision).map(Some)
}

fn pilot_summary_next_action(
    status: &QuorumPilotStatus,
    review_decision: Option<QuorumPilotReviewDecision>,
    review_path: &Path,
) -> String {
    if !status.complete {
        let gate = pilot_first_incomplete_gate_name(status);
        return format!(
            "complete pilot gate `{gate}` with `mimir-librarian quorum pilot-run --manifest-file {}`",
            status.manifest_path
        );
    }
    if review_decision.is_none() {
        return format!(
            "certify pilot with `mimir-librarian quorum pilot-review --manifest-file {} --reviewer ID --decision pass --summary TEXT`",
            status.manifest_path
        );
    }
    if review_decision == Some(QuorumPilotReviewDecision::Pass) {
        "none".to_string()
    } else {
        format!("address pilot review findings in {}", review_path.display())
    }
}

fn pilot_review_path(plan: &QuorumPilotPlan) -> PathBuf {
    PathBuf::from(&plan.out_dir).join(format!(
        "{}-pilot-review.json",
        safe_file_stem(&plan.episode_id)
    ))
}

fn build_quorum_pilot_status(manifest_file: &Path, plan: &QuorumPilotPlan) -> QuorumPilotStatus {
    let store = QuorumStore::new(&plan.quorum_dir);
    let gates = vec![
        pilot_rounds_gate(plan, &store),
        pilot_synthesis_gate(plan),
        pilot_acceptance_gate(plan, &store),
        pilot_submit_drafts_gate(plan, &store),
    ];
    let overall_status = pilot_overall_status(&gates);
    QuorumPilotStatus {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: plan.episode_id.clone(),
        manifest_path: manifest_file.display().to_string(),
        complete: overall_status == QuorumPilotGateStatus::Complete,
        overall_status,
        gates,
    }
}

fn execute_quorum_pilot_manifest(
    manifest_file: &Path,
    plan: &QuorumPilotPlan,
    now: SystemTime,
) -> Result<QuorumPilotRun, CliError> {
    validate_quorum_pilot_manifest_steps(plan)?;
    let mut executed_steps = Vec::new();
    let mut skipped_steps = Vec::new();
    for step in &plan.steps {
        let current_status = build_quorum_pilot_status(manifest_file, plan);
        if pilot_step_is_complete(&current_status, &step.name) {
            skipped_steps.push(step.name.clone());
            continue;
        }
        if current_status.overall_status == QuorumPilotGateStatus::Failed {
            let failed_step = pilot_first_incomplete_gate_name(&current_status);
            return Ok(QuorumPilotRun {
                schema_version: QUORUM_SCHEMA_VERSION,
                episode_id: plan.episode_id.clone(),
                manifest_path: manifest_file.display().to_string(),
                success: false,
                executed_steps,
                skipped_steps,
                failed_step: Some(failed_step.clone()),
                error: Some(format!(
                    "pilot manifest cannot continue because gate '{failed_step}' failed"
                )),
                final_status: current_status,
            });
        }
        if let Err(err) = execute_quorum_pilot_step(step, now) {
            let final_status = build_quorum_pilot_status(manifest_file, plan);
            return Ok(QuorumPilotRun {
                schema_version: QUORUM_SCHEMA_VERSION,
                episode_id: plan.episode_id.clone(),
                manifest_path: manifest_file.display().to_string(),
                success: false,
                executed_steps,
                skipped_steps,
                failed_step: Some(step.name.clone()),
                error: Some(format!("pilot step '{}' failed: {err}", step.name)),
                final_status,
            });
        }
        executed_steps.push(step.name.clone());
    }
    let final_status = build_quorum_pilot_status(manifest_file, plan);
    let success = final_status.complete;
    let failed_step = (!success).then(|| pilot_first_incomplete_gate_name(&final_status));
    let error = failed_step.as_ref().map(|step| {
        format!(
            "pilot manifest finished but final status is {:?}; first incomplete gate: {step}",
            final_status.overall_status
        )
    });
    Ok(QuorumPilotRun {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: plan.episode_id.clone(),
        manifest_path: manifest_file.display().to_string(),
        success,
        executed_steps,
        skipped_steps,
        failed_step,
        error,
        final_status,
    })
}

fn execute_quorum_pilot_step(step: &QuorumPilotStep, now: SystemTime) -> Result<(), CliError> {
    let outcome = quorum_from_args(&step.argv[2..], now)?;
    match (step.name.as_str(), outcome) {
        ("run_rounds", QuorumCliOutcome::AdapterRoundsRun { rounds_run }) if rounds_run.success => {
            Ok(())
        }
        ("run_synthesis", QuorumCliOutcome::SynthesisRun { run }) if run.status.success => Ok(()),
        ("accept_synthesis", QuorumCliOutcome::ResultSaved { .. })
        | ("submit_drafts", QuorumCliOutcome::DraftsSubmitted { .. }) => Ok(()),
        (name, outcome) => Err(CliError::Usage(format!(
            "pilot step '{name}' did not complete successfully: {outcome:?}"
        ))),
    }
}

fn validate_quorum_pilot_manifest_steps(plan: &QuorumPilotPlan) -> Result<(), CliError> {
    for step in &plan.steps {
        validate_quorum_pilot_step_command(step)?;
    }
    Ok(())
}

fn validate_quorum_pilot_step_command(step: &QuorumPilotStep) -> Result<(), CliError> {
    if step.argv.len() < 3 || step.argv[0] != "mimir-librarian" || step.argv[1] != "quorum" {
        return Err(CliError::Usage(format!(
            "pilot step '{}' does not carry a valid mimir-librarian quorum argv",
            step.name
        )));
    }
    let expected = match step.name.as_str() {
        "run_rounds" => "adapter-run-rounds",
        "run_synthesis" => "synthesize-run",
        "accept_synthesis" => "accept-synthesis",
        "submit_drafts" => "submit-drafts",
        other => {
            return Err(CliError::Usage(format!(
                "unknown pilot manifest step '{other}'"
            )));
        }
    };
    if step.argv[2] != expected {
        return Err(CliError::Usage(format!(
            "pilot step '{}' expected command '{expected}', got '{}'",
            step.name, step.argv[2]
        )));
    }
    Ok(())
}

fn pilot_step_is_complete(status: &QuorumPilotStatus, step_name: &str) -> bool {
    status
        .gates
        .iter()
        .any(|gate| gate.name == step_name && gate.status == QuorumPilotGateStatus::Complete)
}

fn pilot_first_incomplete_gate_name(status: &QuorumPilotStatus) -> String {
    status
        .gates
        .iter()
        .find(|gate| gate.status != QuorumPilotGateStatus::Complete)
        .map_or_else(|| "<unknown>".to_string(), |gate| gate.name.clone())
}

fn pilot_overall_status(gates: &[QuorumPilotGate]) -> QuorumPilotGateStatus {
    if gates
        .iter()
        .any(|gate| gate.status == QuorumPilotGateStatus::Failed)
    {
        return QuorumPilotGateStatus::Failed;
    }
    if gates
        .iter()
        .all(|gate| gate.status == QuorumPilotGateStatus::Complete)
    {
        return QuorumPilotGateStatus::Complete;
    }
    QuorumPilotGateStatus::Pending
}

fn pilot_rounds_gate(plan: &QuorumPilotPlan, store: &QuorumStore) -> QuorumPilotGate {
    let artifacts = plan
        .artifacts
        .round_statuses
        .iter()
        .map(|artifact| artifact.status_path.clone())
        .collect::<Vec<_>>();
    for artifact in &plan.artifacts.round_statuses {
        let path = Path::new(&artifact.status_path);
        if !path.exists() {
            return pilot_gate(
                "run_rounds",
                QuorumPilotGateStatus::Pending,
                "round status artifact is missing",
                artifacts,
            );
        }
        if let Err(err) = pilot_validate_round_artifact(plan, store, artifact) {
            return pilot_gate("run_rounds", err.0, &err.1, artifacts);
        }
    }
    pilot_gate(
        "run_rounds",
        QuorumPilotGateStatus::Complete,
        "all round statuses are successful and participant outputs are recorded",
        artifacts,
    )
}

fn pilot_validate_round_artifact(
    plan: &QuorumPilotPlan,
    store: &QuorumStore,
    artifact: &QuorumPilotRoundArtifact,
) -> Result<(), (QuorumPilotGateStatus, String)> {
    let statuses = load_adapter_status_artifact(Path::new(&artifact.status_path))
        .map_err(|err| (QuorumPilotGateStatus::Failed, err.to_string()))?;
    for status in &statuses {
        validate_adapter_status_for_append(status)
            .map_err(|err| (QuorumPilotGateStatus::Failed, err.to_string()))?;
    }
    let round = parse_quorum_round(&artifact.round)
        .map_err(|err| (QuorumPilotGateStatus::Failed, err.to_string()))?;
    let outputs = store
        .load_round_outputs(&plan.episode_id, round)
        .map_err(|err| (QuorumPilotGateStatus::Failed, err.to_string()))?;
    if outputs.len() < plan.participants.len() {
        return Err((
            QuorumPilotGateStatus::Pending,
            "round status exists but participant outputs have not all been appended".to_string(),
        ));
    }
    Ok(())
}

fn pilot_synthesis_gate(plan: &QuorumPilotPlan) -> QuorumPilotGate {
    let artifacts = vec![
        plan.artifacts.synthesis_status_path.clone(),
        plan.artifacts.synthesis_result_path.clone(),
    ];
    let status_file = Path::new(&plan.artifacts.synthesis_status_path);
    if !status_file.exists() {
        return pilot_gate(
            "run_synthesis",
            QuorumPilotGateStatus::Pending,
            "synthesis status artifact is missing",
            artifacts,
        );
    }
    match pilot_validate_synthesis_status(plan, status_file) {
        Ok(()) => pilot_gate(
            "run_synthesis",
            QuorumPilotGateStatus::Complete,
            "synthesis status and proposed result are valid",
            artifacts,
        ),
        Err(message) => pilot_gate(
            "run_synthesis",
            QuorumPilotGateStatus::Failed,
            &message,
            artifacts,
        ),
    }
}

fn pilot_validate_synthesis_status(
    plan: &QuorumPilotPlan,
    status_file: &Path,
) -> Result<(), String> {
    let status = load_synthesis_status_artifact(status_file).map_err(|err| err.to_string())?;
    let parsed = QuorumAcceptSynthesisArgs {
        quorum_dir: PathBuf::from(&plan.quorum_dir),
        episode_id: Some(plan.episode_id.clone()),
        result_file: None,
        status_file: Some(status_file.to_path_buf()),
    };
    validate_synthesis_status_for_accept(&parsed, status_file, &status)
        .map_err(|err| err.to_string())
}

fn pilot_acceptance_gate(plan: &QuorumPilotPlan, store: &QuorumStore) -> QuorumPilotGate {
    let result_path = store.result_artifact_path(&plan.episode_id);
    let artifacts = vec![result_path.display().to_string()];
    if !result_path.exists() {
        return pilot_gate(
            "accept_synthesis",
            QuorumPilotGateStatus::Pending,
            "accepted quorum result is missing",
            artifacts,
        );
    }
    match store.load_result(&plan.episode_id) {
        Ok(result)
            if result.schema_version == QUORUM_SCHEMA_VERSION
                && result.episode_id == plan.episode_id =>
        {
            if result.proposed_memory_drafts.len() < plan.required_proposed_drafts {
                return pilot_gate(
                    "accept_synthesis",
                    QuorumPilotGateStatus::Failed,
                    &format!(
                        "accepted quorum result has {} proposed memory drafts; pilot requires at least {}",
                        result.proposed_memory_drafts.len(),
                        plan.required_proposed_drafts
                    ),
                    artifacts,
                );
            }
            pilot_gate(
                "accept_synthesis",
                QuorumPilotGateStatus::Complete,
                "accepted quorum result exists",
                artifacts,
            )
        }
        Ok(_) => pilot_gate(
            "accept_synthesis",
            QuorumPilotGateStatus::Failed,
            "accepted quorum result does not match the pilot episode",
            artifacts,
        ),
        Err(err) => pilot_gate(
            "accept_synthesis",
            QuorumPilotGateStatus::Failed,
            &err.to_string(),
            artifacts,
        ),
    }
}

fn pilot_submit_drafts_gate(plan: &QuorumPilotPlan, store: &QuorumStore) -> QuorumPilotGate {
    let artifacts = vec![plan.drafts_dir.clone()];
    let Ok(result) = store.load_result(&plan.episode_id) else {
        return pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Pending,
            "accepted quorum result is missing",
            artifacts,
        );
    };
    let expected = result.proposed_memory_drafts.len();
    if expected < plan.required_proposed_drafts {
        return pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Failed,
            &format!(
                "accepted quorum result has {expected} proposed memory drafts; pilot requires at least {}",
                plan.required_proposed_drafts
            ),
            artifacts,
        );
    }
    if expected == 0 {
        return pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Complete,
            "accepted result has no proposed memory drafts",
            artifacts,
        );
    }
    match count_quorum_drafts(Path::new(&plan.drafts_dir), &plan.episode_id) {
        Ok(count) if count >= expected => pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Complete,
            "proposed memory drafts have been submitted",
            artifacts,
        ),
        Ok(_) => pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Pending,
            "proposed memory drafts have not all been submitted",
            artifacts,
        ),
        Err(err) => pilot_gate(
            "submit_drafts",
            QuorumPilotGateStatus::Failed,
            &err.to_string(),
            artifacts,
        ),
    }
}

fn count_quorum_drafts(drafts_dir: &Path, episode_id: &str) -> Result<usize, CliError> {
    let provenance = format!("quorum://episode/{episode_id}");
    let mut count = 0;
    for state_dir in [
        "pending",
        "processing",
        "accepted",
        "skipped",
        "failed",
        "quarantined",
    ] {
        count += count_matching_drafts_in_dir(&drafts_dir.join(state_dir), &provenance)?;
    }
    Ok(count)
}

fn count_matching_drafts_in_dir(dir: &Path, provenance: &str) -> Result<usize, CliError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(LibrarianError::from(err).into()),
    };
    let mut count = 0;
    for entry in entries {
        let path = entry.map_err(LibrarianError::from)?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(path).map_err(LibrarianError::from)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(LibrarianError::from)?;
        if value
            .get("source_surface")
            .and_then(serde_json::Value::as_str)
            == Some("consensus_quorum")
            && value
                .get("provenance_uri")
                .and_then(serde_json::Value::as_str)
                == Some(provenance)
        {
            count += 1;
        }
    }
    Ok(count)
}

fn pilot_gate(
    name: &str,
    status: QuorumPilotGateStatus,
    detail: &str,
    artifacts: Vec<String>,
) -> QuorumPilotGate {
    QuorumPilotGate {
        name: name.to_string(),
        status,
        detail: detail.to_string(),
        artifacts,
    }
}

struct QuorumAppendOutputArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    output_id: String,
    participant_id: String,
    round: QuorumRound,
    submitted_at_unix_ms: Option<u64>,
    prompt: String,
    response: String,
    visible_prior_output_ids: Vec<String>,
    evidence_used: Vec<String>,
}

struct QuorumAppendStatusOutputArgs {
    status_file: PathBuf,
    submitted_at_unix_ms: Option<u64>,
}

fn quorum_append_output_from_args(
    args: &[String],
    now: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_append_output_args(args)?;
    let output = QuorumParticipantOutput {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: parsed.episode_id.clone(),
        output_id: parsed.output_id.clone(),
        participant_id: parsed.participant_id,
        round: parsed.round,
        submitted_at_unix_ms: parsed
            .submitted_at_unix_ms
            .unwrap_or_else(|| system_time_to_unix_ms(now)),
        prompt: parsed.prompt,
        response: parsed.response,
        visible_prior_output_ids: parsed.visible_prior_output_ids,
        evidence_used: parsed.evidence_used,
    };
    let path = QuorumStore::new(parsed.quorum_dir).append_participant_output(&output)?;
    Ok(QuorumCliOutcome::OutputAppended {
        episode_id: parsed.episode_id,
        output_id: parsed.output_id,
        path: path.display().to_string(),
    })
}

fn quorum_append_status_output_from_args(
    args: &[String],
    now: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_append_status_output_args(args)?;
    let statuses = load_adapter_status_artifact(&parsed.status_file)?;
    let append_args = statuses
        .iter()
        .map(validated_append_output_args_from_status)
        .collect::<Result<Vec<_>, _>>()?;
    let mut outputs = Vec::with_capacity(append_args.len());

    for mut args in append_args {
        if let Some(submitted_at_unix_ms) = parsed.submitted_at_unix_ms {
            if flag_value(&args, "--submitted-at-unix-ms").is_some() {
                return Err(CliError::Usage(
                    "status append command already includes --submitted-at-unix-ms".to_string(),
                ));
            }
            args.push("--submitted-at-unix-ms".to_string());
            args.push(submitted_at_unix_ms.to_string());
        }
        let status = matching_status_for_append_args(&statuses, &args)?;
        let outcome = quorum_from_args(&args, now)?;
        match outcome {
            QuorumCliOutcome::OutputAppended {
                episode_id,
                output_id,
                path,
            } => outputs.push(QuorumStatusOutputAppend {
                episode_id,
                output_id,
                participant_id: status.participant_id.clone(),
                round: status.round.clone(),
                path,
            }),
            other => {
                return Err(CliError::Usage(format!(
                    "append-status-output expected append-output result, got {other:?}"
                )));
            }
        }
    }

    Ok(QuorumCliOutcome::StatusOutputsAppended {
        status_path: parsed.status_file.display().to_string(),
        appended: outputs.len(),
        outputs,
    })
}

fn parse_quorum_append_status_output_args(
    args: &[String],
) -> Result<QuorumAppendStatusOutputArgs, CliError> {
    let mut status_file: Option<PathBuf> = None;
    let mut submitted_at_unix_ms: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--status-file" => {
                status_file = Some(take_value(args, &mut i, "--status-file")?.into());
            }
            "--submitted-at-unix-ms" => {
                let value = take_value(args, &mut i, "--submitted-at-unix-ms")?;
                submitted_at_unix_ms = Some(parse_u64("--submitted-at-unix-ms", &value)?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum append-status-output option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumAppendStatusOutputArgs {
        status_file: require_path(status_file, "--status-file")?,
        submitted_at_unix_ms,
    })
}

fn parse_quorum_append_output_args(args: &[String]) -> Result<QuorumAppendOutputArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut output_id: Option<String> = None;
    let mut participant_id: Option<String> = None;
    let mut round: Option<QuorumRound> = None;
    let mut submitted_at_unix_ms: Option<u64> = None;
    let mut prompt: Option<String> = None;
    let mut prompt_file: Option<PathBuf> = None;
    let mut response: Option<String> = None;
    let mut response_file: Option<PathBuf> = None;
    let mut visible_prior_output_ids = Vec::new();
    let mut evidence_used = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--output-id" => {
                output_id = Some(take_value(args, &mut i, "--output-id")?);
            }
            "--participant-id" => {
                participant_id = Some(take_value(args, &mut i, "--participant-id")?);
            }
            "--round" => {
                let value = take_value(args, &mut i, "--round")?;
                round = Some(parse_quorum_round(&value)?);
            }
            "--submitted-at-unix-ms" => {
                let value = take_value(args, &mut i, "--submitted-at-unix-ms")?;
                submitted_at_unix_ms = Some(parse_u64("--submitted-at-unix-ms", &value)?);
            }
            "--prompt" => {
                if prompt_file.is_some() {
                    return Err(CliError::Usage(
                        "--prompt and --prompt-file are mutually exclusive".to_string(),
                    ));
                }
                prompt = Some(take_value(args, &mut i, "--prompt")?);
            }
            "--prompt-file" => {
                if prompt.is_some() {
                    return Err(CliError::Usage(
                        "--prompt and --prompt-file are mutually exclusive".to_string(),
                    ));
                }
                prompt_file = Some(take_value(args, &mut i, "--prompt-file")?.into());
            }
            "--response" => {
                if response_file.is_some() {
                    return Err(CliError::Usage(
                        "--response and --response-file are mutually exclusive".to_string(),
                    ));
                }
                response = Some(take_value(args, &mut i, "--response")?);
            }
            "--response-file" => {
                if response.is_some() {
                    return Err(CliError::Usage(
                        "--response and --response-file are mutually exclusive".to_string(),
                    ));
                }
                response_file = Some(take_value(args, &mut i, "--response-file")?.into());
            }
            "--visible-prior-output-id" => {
                visible_prior_output_ids.push(take_value(
                    args,
                    &mut i,
                    "--visible-prior-output-id",
                )?);
            }
            "--evidence" => {
                evidence_used.push(take_value(args, &mut i, "--evidence")?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum append-output option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumAppendOutputArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        output_id: require_non_empty(output_id, "--output-id")?,
        participant_id: require_non_empty(participant_id, "--participant-id")?,
        round: round.ok_or_else(|| CliError::Usage("--round is required".to_string()))?,
        submitted_at_unix_ms,
        prompt: require_text_or_file(prompt, prompt_file, "--prompt", "--prompt-file")?,
        response: require_text_or_file(response, response_file, "--response", "--response-file")?,
        visible_prior_output_ids,
        evidence_used,
    })
}

fn quorum_outputs_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let (quorum_dir, episode_id, round) = parse_quorum_read_args(args, "outputs")?;
    let outputs = QuorumStore::new(quorum_dir).load_round_outputs(&episode_id, round)?;
    Ok(QuorumCliOutcome::OutputsLoaded {
        episode_id,
        round: quorum_round_name(round).to_string(),
        outputs,
    })
}

fn quorum_visible_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let (quorum_dir, episode_id, round) = parse_quorum_read_args(args, "visible")?;
    let outputs = QuorumStore::new(quorum_dir).visible_outputs_for_round(&episode_id, round)?;
    Ok(QuorumCliOutcome::VisibleOutputs {
        episode_id,
        round: quorum_round_name(round).to_string(),
        outputs,
    })
}

fn quorum_adapter_request_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut participant_id: Option<String> = None;
    let mut round: Option<QuorumRound> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--participant-id" => {
                participant_id = Some(take_value(args, &mut i, "--participant-id")?);
            }
            "--round" => {
                let value = take_value(args, &mut i, "--round")?;
                round = Some(parse_quorum_round(&value)?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum adapter-request option '{other}'"
                )));
            }
        }
        i += 1;
    }

    let request = QuorumStore::new(require_path(quorum_dir, "--quorum-dir")?)
        .build_adapter_request(
            &require_non_empty(episode_id, "--episode-id")?,
            &require_non_empty(participant_id, "--participant-id")?,
            round.ok_or_else(|| CliError::Usage("--round is required".to_string()))?,
        )?;
    Ok(QuorumCliOutcome::AdapterRequest {
        request: Box::new(request),
    })
}

#[derive(Debug, Clone)]
struct QuorumAdapterPlanArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    participant_id: String,
    round: QuorumRound,
    adapter: String,
    out_dir: PathBuf,
    binary: Option<String>,
    output_id: Option<String>,
}

struct QuorumAdapterPlanPaths {
    request: PathBuf,
    prompt: PathBuf,
    response: PathBuf,
    status: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

struct QuorumAdapterRunArgs {
    plan: QuorumAdapterPlanArgs,
    timeout: Duration,
}

struct QuorumAdapterRunRoundArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    round: QuorumRound,
    out_dir: PathBuf,
    adapter_binaries: BTreeMap<String, String>,
    timeout: Duration,
}

struct QuorumAdapterRunRoundsArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    through_round: QuorumRound,
    out_dir: PathBuf,
    adapter_binaries: BTreeMap<String, String>,
    timeout: Duration,
    submitted_at_unix_ms: Option<u64>,
}

fn quorum_adapter_plan_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_adapter_plan_args(args)?;
    Ok(QuorumCliOutcome::AdapterPlan {
        plan: Box::new(build_quorum_adapter_plan(&parsed)?),
    })
}

fn quorum_adapter_run_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_adapter_run_args(args)?;
    let plan = build_quorum_adapter_plan(&parsed.plan)?;
    let status = execute_quorum_adapter_plan(&plan, parsed.timeout)?;
    Ok(QuorumCliOutcome::AdapterRun {
        run: Box::new(QuorumAdapterRun {
            schema_version: QUORUM_SCHEMA_VERSION,
            status,
        }),
    })
}

fn quorum_adapter_run_round_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_adapter_run_round_args(args)?;
    let round_run = execute_quorum_adapter_round(&parsed)?;
    Ok(QuorumCliOutcome::AdapterRoundRun {
        round_run: Box::new(round_run),
    })
}

fn quorum_adapter_run_rounds_from_args(
    args: &[String],
    now: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_adapter_run_rounds_args(args)?;
    let rounds_run = execute_quorum_adapter_rounds(&parsed, now)?;
    Ok(QuorumCliOutcome::AdapterRoundsRun {
        rounds_run: Box::new(rounds_run),
    })
}

fn build_quorum_adapter_plan(
    parsed: &QuorumAdapterPlanArgs,
) -> Result<QuorumAdapterPlan, CliError> {
    let store = QuorumStore::new(&parsed.quorum_dir);
    let request =
        store.build_adapter_request(&parsed.episode_id, &parsed.participant_id, parsed.round)?;
    validate_adapter_plan_request(&request, &parsed.adapter)?;
    let paths = adapter_plan_paths(
        &parsed.out_dir,
        &parsed.episode_id,
        &parsed.participant_id,
        parsed.round,
    );
    fs::create_dir_all(&parsed.out_dir).map_err(LibrarianError::from)?;
    write_adapter_plan_files(&paths, &request)?;

    let response_path = paths.response.display().to_string();
    let status_path = paths.status.display().to_string();
    let prompt_path = paths.prompt.display().to_string();
    let command_argv =
        adapter_plan_argv(&parsed.adapter, parsed.binary.as_deref(), &response_path)?;
    let stdout_path = adapter_plan_stdout_path(&parsed.adapter, &response_path)?;
    let stdout_capture_path = stdout_path
        .clone()
        .unwrap_or_else(|| paths.stdout.display().to_string());
    let stderr_capture_path = paths.stderr.display().to_string();
    let output_id = parsed.output_id.clone().unwrap_or_else(|| {
        format!(
            "out-{}-{}",
            quorum_round_name(parsed.round),
            parsed.participant_id
        )
    });
    let append_output_argv =
        append_output_plan_argv(parsed, &request, &output_id, &prompt_path, &response_path);

    Ok(QuorumAdapterPlan {
        schema_version: QUORUM_SCHEMA_VERSION,
        adapter: parsed.adapter.clone(),
        episode_id: parsed.episode_id.clone(),
        participant_id: parsed.participant_id.clone(),
        round: quorum_round_name(parsed.round).to_string(),
        request_path: paths.request.display().to_string(),
        prompt_path: prompt_path.clone(),
        response_path: response_path.clone(),
        status_path,
        stdin_path: prompt_path,
        stdout_path,
        stdout_capture_path,
        stderr_capture_path,
        argv: command_argv,
        append_output_argv,
    })
}

fn parse_quorum_adapter_plan_args(args: &[String]) -> Result<QuorumAdapterPlanArgs, CliError> {
    parse_quorum_adapter_args(args, "adapter-plan", false).map(|parsed| parsed.plan)
}

fn parse_quorum_adapter_run_args(args: &[String]) -> Result<QuorumAdapterRunArgs, CliError> {
    let mut parsed = parse_quorum_adapter_args(args, "adapter-run", true)?;
    let timeout = parsed
        .timeout
        .take()
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS));
    Ok(QuorumAdapterRunArgs {
        plan: parsed.plan,
        timeout,
    })
}

fn parse_quorum_adapter_run_round_args(
    args: &[String],
) -> Result<QuorumAdapterRunRoundArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut round: Option<QuorumRound> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut adapter_binaries = BTreeMap::new();
    let mut timeout: Option<Duration> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--round" => {
                let value = take_value(args, &mut i, "--round")?;
                round = Some(parse_quorum_round(&value)?);
            }
            "--out-dir" => {
                out_dir = Some(take_value(args, &mut i, "--out-dir")?.into());
            }
            "--adapter-binary" => {
                let value = take_value(args, &mut i, "--adapter-binary")?;
                let (adapter, binary) = parse_adapter_binary(&value)?;
                if adapter_binaries.insert(adapter.clone(), binary).is_some() {
                    return Err(CliError::Usage(format!(
                        "--adapter-binary supplied more than once for adapter '{adapter}'"
                    )));
                }
            }
            "--timeout-secs" => {
                let value = take_value(args, &mut i, "--timeout-secs")?;
                timeout = Some(Duration::from_secs(parse_positive_u64(
                    "--timeout-secs",
                    &value,
                )?));
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum adapter-run-round option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumAdapterRunRoundArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        round: round.ok_or_else(|| CliError::Usage("--round is required".to_string()))?,
        out_dir: require_path(out_dir, "--out-dir")?,
        adapter_binaries,
        timeout: timeout
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS)),
    })
}

fn parse_quorum_adapter_run_rounds_args(
    args: &[String],
) -> Result<QuorumAdapterRunRoundsArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut through_round: Option<QuorumRound> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut adapter_binaries = BTreeMap::new();
    let mut timeout: Option<Duration> = None;
    let mut submitted_at_unix_ms: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--through-round" => {
                let value = take_value(args, &mut i, "--through-round")?;
                through_round = Some(parse_quorum_round(&value)?);
            }
            "--out-dir" => {
                out_dir = Some(take_value(args, &mut i, "--out-dir")?.into());
            }
            "--adapter-binary" => {
                let value = take_value(args, &mut i, "--adapter-binary")?;
                let (adapter, binary) = parse_adapter_binary(&value)?;
                if adapter_binaries.insert(adapter.clone(), binary).is_some() {
                    return Err(CliError::Usage(format!(
                        "--adapter-binary supplied more than once for adapter '{adapter}'"
                    )));
                }
            }
            "--timeout-secs" => {
                let value = take_value(args, &mut i, "--timeout-secs")?;
                timeout = Some(Duration::from_secs(parse_positive_u64(
                    "--timeout-secs",
                    &value,
                )?));
            }
            "--submitted-at-unix-ms" => {
                let value = take_value(args, &mut i, "--submitted-at-unix-ms")?;
                submitted_at_unix_ms = Some(parse_u64("--submitted-at-unix-ms", &value)?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum adapter-run-rounds option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumAdapterRunRoundsArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        through_round: through_round
            .ok_or_else(|| CliError::Usage("--through-round is required".to_string()))?,
        out_dir: require_path(out_dir, "--out-dir")?,
        adapter_binaries,
        timeout: timeout
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS)),
        submitted_at_unix_ms,
    })
}

fn parse_adapter_binary(value: &str) -> Result<(String, String), CliError> {
    let Some((adapter, binary)) = value.split_once('=') else {
        return Err(CliError::Usage(
            "--adapter-binary must be ADAPTER=PATH".to_string(),
        ));
    };
    let adapter = parse_quorum_adapter(adapter)?;
    let binary = require_non_empty(Some(binary.to_string()), "--adapter-binary PATH")?;
    Ok((adapter, binary))
}

struct ParsedQuorumAdapterArgs {
    plan: QuorumAdapterPlanArgs,
    timeout: Option<Duration>,
}

fn parse_quorum_adapter_args(
    args: &[String],
    command: &str,
    allow_timeout: bool,
) -> Result<ParsedQuorumAdapterArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut participant_id: Option<String> = None;
    let mut round: Option<QuorumRound> = None;
    let mut adapter: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut binary: Option<String> = None;
    let mut output_id: Option<String> = None;
    let mut timeout: Option<Duration> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--participant-id" => {
                participant_id = Some(take_value(args, &mut i, "--participant-id")?);
            }
            "--round" => {
                let value = take_value(args, &mut i, "--round")?;
                round = Some(parse_quorum_round(&value)?);
            }
            "--adapter" => {
                adapter = Some(parse_quorum_adapter(&take_value(
                    args,
                    &mut i,
                    "--adapter",
                )?)?);
            }
            "--out-dir" => {
                out_dir = Some(take_value(args, &mut i, "--out-dir")?.into());
            }
            "--binary" => {
                binary = Some(take_value(args, &mut i, "--binary")?);
            }
            "--output-id" => {
                output_id = Some(take_value(args, &mut i, "--output-id")?);
            }
            "--timeout-secs" if allow_timeout => {
                let value = take_value(args, &mut i, "--timeout-secs")?;
                timeout = Some(Duration::from_secs(parse_positive_u64(
                    "--timeout-secs",
                    &value,
                )?));
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum {command} option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(ParsedQuorumAdapterArgs {
        plan: QuorumAdapterPlanArgs {
            quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
            episode_id: require_non_empty(episode_id, "--episode-id")?,
            participant_id: require_non_empty(participant_id, "--participant-id")?,
            round: round.ok_or_else(|| CliError::Usage("--round is required".to_string()))?,
            adapter: require_non_empty(adapter, "--adapter")?,
            out_dir: require_path(out_dir, "--out-dir")?,
            binary,
            output_id,
        },
        timeout,
    })
}

fn validate_adapter_plan_request(
    request: &QuorumAdapterRequest,
    adapter: &str,
) -> Result<(), CliError> {
    if request.participant.adapter == adapter {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "--adapter {adapter} does not match participant '{}' adapter '{}'",
        request.participant.id, request.participant.adapter
    )))
}

fn adapter_plan_paths(
    out_dir: &Path,
    episode_id: &str,
    participant_id: &str,
    round: QuorumRound,
) -> QuorumAdapterPlanPaths {
    let stem = format!(
        "{}-{}-{}",
        safe_file_stem(episode_id),
        safe_file_stem(participant_id),
        quorum_round_name(round)
    );
    QuorumAdapterPlanPaths {
        request: out_dir.join(format!("{stem}-request.json")),
        prompt: out_dir.join(format!("{stem}-prompt.md")),
        response: out_dir.join(format!("{stem}-response.md")),
        status: out_dir.join(format!("{stem}-status.json")),
        stdout: out_dir.join(format!("{stem}-stdout.log")),
        stderr: out_dir.join(format!("{stem}-stderr.log")),
    }
}

fn write_adapter_plan_files(
    paths: &QuorumAdapterPlanPaths,
    request: &QuorumAdapterRequest,
) -> Result<(), CliError> {
    let request_json = serde_json::to_vec_pretty(request).map_err(LibrarianError::from)?;
    let prompt = build_adapter_prompt(request);
    fs::write(&paths.request, request_json).map_err(LibrarianError::from)?;
    fs::write(&paths.prompt, prompt).map_err(LibrarianError::from)?;
    Ok(())
}

fn execute_quorum_adapter_plan(
    plan: &QuorumAdapterPlan,
    timeout: Duration,
) -> Result<QuorumAdapterRunStatus, CliError> {
    let program = plan
        .argv
        .first()
        .ok_or_else(|| CliError::Usage("adapter plan argv must not be empty".to_string()))?;
    let started = Instant::now();
    let mut command = Command::new(program);
    command.args(&plan.argv[1..]);
    command.stdin(Stdio::from(
        fs::File::open(&plan.stdin_path).map_err(LibrarianError::from)?,
    ));
    command.stdout(Stdio::from(
        fs::File::create(&plan.stdout_capture_path).map_err(LibrarianError::from)?,
    ));
    command.stderr(Stdio::from(
        fs::File::create(&plan.stderr_capture_path).map_err(LibrarianError::from)?,
    ));

    let mut child = command.spawn().map_err(LibrarianError::from)?;
    let wait_result = child.wait_timeout(timeout);
    let (timed_out, exit_code, process_success) = match wait_result {
        Ok(Some(status)) => (false, status.code(), status.success()),
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            (true, None, false)
        }
        Err(err) => return Err(LibrarianError::from(err).into()),
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let status = QuorumAdapterRunStatus {
        schema_version: QUORUM_SCHEMA_VERSION,
        adapter: plan.adapter.clone(),
        episode_id: plan.episode_id.clone(),
        participant_id: plan.participant_id.clone(),
        round: plan.round.clone(),
        status_path: plan.status_path.clone(),
        request_path: plan.request_path.clone(),
        prompt_path: plan.prompt_path.clone(),
        response_path: plan.response_path.clone(),
        stdout_capture_path: plan.stdout_capture_path.clone(),
        stderr_capture_path: plan.stderr_capture_path.clone(),
        success: process_success && !timed_out,
        timed_out,
        exit_code,
        duration_ms,
        response_bytes: file_len(&plan.response_path)?,
        stdout_bytes: file_len(&plan.stdout_capture_path)?,
        stderr_bytes: file_len(&plan.stderr_capture_path)?,
        append_output_argv: plan.append_output_argv.clone(),
    };
    write_adapter_run_status(&status)?;
    Ok(status)
}

fn execute_quorum_adapter_round(
    parsed: &QuorumAdapterRunRoundArgs,
) -> Result<QuorumAdapterRoundRun, CliError> {
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&parsed.episode_id)?;
    let mut statuses = Vec::with_capacity(episode.participants.len());
    for participant in &episode.participants {
        let plan_args = QuorumAdapterPlanArgs {
            quorum_dir: parsed.quorum_dir.clone(),
            episode_id: parsed.episode_id.clone(),
            participant_id: participant.id.clone(),
            round: parsed.round,
            adapter: participant.adapter.clone(),
            out_dir: parsed.out_dir.clone(),
            binary: parsed.adapter_binaries.get(&participant.adapter).cloned(),
            output_id: None,
        };
        let plan = build_quorum_adapter_plan(&plan_args)?;
        statuses.push(execute_quorum_adapter_plan(&plan, parsed.timeout)?);
    }
    let status_path = adapter_round_status_path(&parsed.out_dir, &parsed.episode_id, parsed.round);
    let round_run = QuorumAdapterRoundRun {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: parsed.episode_id.clone(),
        round: quorum_round_name(parsed.round).to_string(),
        status_path: status_path.display().to_string(),
        success: statuses.iter().all(|status| status.success),
        completed: statuses.len(),
        failed: statuses.iter().filter(|status| !status.success).count(),
        timed_out: statuses.iter().filter(|status| status.timed_out).count(),
        statuses,
    };
    write_adapter_round_status(&round_run)?;
    Ok(round_run)
}

fn execute_quorum_adapter_rounds(
    parsed: &QuorumAdapterRunRoundsArgs,
    now: SystemTime,
) -> Result<QuorumAdapterRoundsRun, CliError> {
    let mut rounds = Vec::new();
    let mut appended = Vec::new();
    for round in quorum_rounds_through(parsed.through_round) {
        let round_args = QuorumAdapterRunRoundArgs {
            quorum_dir: parsed.quorum_dir.clone(),
            episode_id: parsed.episode_id.clone(),
            round,
            out_dir: parsed.out_dir.clone(),
            adapter_binaries: parsed.adapter_binaries.clone(),
            timeout: parsed.timeout,
        };
        let round_run = execute_quorum_adapter_round(&round_args)?;
        let mut append_args = vec!["--status-file".to_string(), round_run.status_path.clone()];
        if let Some(submitted_at_unix_ms) = parsed.submitted_at_unix_ms {
            append_args.push("--submitted-at-unix-ms".to_string());
            append_args.push(submitted_at_unix_ms.to_string());
        }
        match quorum_append_status_output_from_args(&append_args, now)? {
            QuorumCliOutcome::StatusOutputsAppended {
                outputs: round_appended,
                ..
            } => appended.extend(round_appended),
            other => {
                return Err(CliError::Usage(format!(
                    "adapter-run-rounds expected status append result, got {other:?}"
                )));
            }
        }
        rounds.push(round_run);
    }
    Ok(QuorumAdapterRoundsRun {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: parsed.episode_id.clone(),
        through_round: quorum_round_name(parsed.through_round).to_string(),
        success: rounds.iter().all(|round| round.success),
        rounds,
        appended,
    })
}

fn quorum_rounds_through(through_round: QuorumRound) -> Vec<QuorumRound> {
    match through_round {
        QuorumRound::Independent => vec![QuorumRound::Independent],
        QuorumRound::Critique => vec![QuorumRound::Independent, QuorumRound::Critique],
        QuorumRound::Revision => vec![
            QuorumRound::Independent,
            QuorumRound::Critique,
            QuorumRound::Revision,
        ],
    }
}

fn write_adapter_run_status(status: &QuorumAdapterRunStatus) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(status).map_err(LibrarianError::from)?;
    fs::write(&status.status_path, bytes).map_err(LibrarianError::from)?;
    Ok(())
}

fn write_adapter_round_status(status: &QuorumAdapterRoundRun) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(status).map_err(LibrarianError::from)?;
    fs::write(&status.status_path, bytes).map_err(LibrarianError::from)?;
    Ok(())
}

fn adapter_round_status_path(out_dir: &Path, episode_id: &str, round: QuorumRound) -> PathBuf {
    out_dir.join(format!(
        "{}-{}-round-status.json",
        safe_file_stem(episode_id),
        quorum_round_name(round)
    ))
}

fn load_adapter_status_artifact(path: &Path) -> Result<Vec<QuorumAdapterRunStatus>, CliError> {
    let bytes = fs::read(path).map_err(LibrarianError::from)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(LibrarianError::from)?;
    if value.get("statuses").is_some() {
        let round_run: QuorumAdapterRoundRun =
            serde_json::from_value(value).map_err(LibrarianError::from)?;
        validate_adapter_round_status_for_append(&round_run)?;
        return Ok(round_run.statuses);
    }
    let status: QuorumAdapterRunStatus =
        serde_json::from_value(value).map_err(LibrarianError::from)?;
    Ok(vec![status])
}

fn validate_adapter_round_status_for_append(
    round_run: &QuorumAdapterRoundRun,
) -> Result<(), CliError> {
    if round_run.schema_version != QUORUM_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported adapter round status schema version {}; expected {QUORUM_SCHEMA_VERSION}",
            round_run.schema_version
        )));
    }
    if !round_run.success {
        return Err(CliError::Usage(format!(
            "adapter round status for episode '{}' round '{}' is not successful",
            round_run.episode_id, round_run.round
        )));
    }
    if round_run.statuses.is_empty() {
        return Err(CliError::Usage(
            "adapter round status contains no participant statuses".to_string(),
        ));
    }
    let failed = round_run
        .statuses
        .iter()
        .filter(|status| !status.success)
        .count();
    let timed_out = round_run
        .statuses
        .iter()
        .filter(|status| status.timed_out)
        .count();
    if round_run.failed != failed || round_run.timed_out != timed_out {
        return Err(CliError::Usage(
            "adapter round status counters do not match participant statuses".to_string(),
        ));
    }
    for status in &round_run.statuses {
        if status.episode_id != round_run.episode_id || status.round != round_run.round {
            return Err(CliError::Usage(format!(
                "adapter round status contains mismatched participant status for episode '{}' round '{}'",
                status.episode_id, status.round
            )));
        }
    }
    Ok(())
}

fn validated_append_output_args_from_status(
    status: &QuorumAdapterRunStatus,
) -> Result<Vec<String>, CliError> {
    validate_adapter_status_for_append(status)?;
    let args = append_output_args_from_status(status)?;
    require_append_arg_eq(&args, "--episode-id", &status.episode_id)?;
    require_append_arg_eq(&args, "--participant-id", &status.participant_id)?;
    require_append_arg_eq(&args, "--round", &status.round)?;
    require_append_arg_eq(&args, "--prompt-file", &status.prompt_path)?;
    require_append_arg_eq(&args, "--response-file", &status.response_path)?;
    Ok(args)
}

fn validate_adapter_status_for_append(status: &QuorumAdapterRunStatus) -> Result<(), CliError> {
    if status.schema_version != QUORUM_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported adapter status schema version {}; expected {QUORUM_SCHEMA_VERSION}",
            status.schema_version
        )));
    }
    if !status.success {
        return Err(CliError::Usage(format!(
            "adapter status for participant '{}' round '{}' is not successful",
            status.participant_id, status.round
        )));
    }
    if status.timed_out {
        return Err(CliError::Usage(format!(
            "adapter status for participant '{}' round '{}' timed out",
            status.participant_id, status.round
        )));
    }
    if file_len(&status.prompt_path)? == 0 {
        return Err(CliError::Usage(format!(
            "adapter status prompt artifact is missing or empty: {}",
            status.prompt_path
        )));
    }
    if status.response_bytes == 0 {
        return Err(CliError::Usage(format!(
            "adapter status response artifact is empty: {}",
            status.response_path
        )));
    }
    let actual_response_bytes = file_len(&status.response_path)?;
    if actual_response_bytes != status.response_bytes {
        return Err(CliError::Usage(format!(
            "adapter status response byte count mismatch for {}; status says {}, file has {}",
            status.response_path, status.response_bytes, actual_response_bytes
        )));
    }
    Ok(())
}

fn append_output_args_from_status(
    status: &QuorumAdapterRunStatus,
) -> Result<Vec<String>, CliError> {
    if status.append_output_argv.len() < 3
        || status.append_output_argv[0] != "mimir-librarian"
        || status.append_output_argv[1] != "quorum"
        || status.append_output_argv[2] != "append-output"
    {
        return Err(CliError::Usage(format!(
            "adapter status for participant '{}' does not carry a valid append-output command",
            status.participant_id
        )));
    }
    Ok(status.append_output_argv.iter().skip(2).cloned().collect())
}

fn matching_status_for_append_args<'a>(
    statuses: &'a [QuorumAdapterRunStatus],
    args: &[String],
) -> Result<&'a QuorumAdapterRunStatus, CliError> {
    let episode_id = required_flag_value(args, "--episode-id")?;
    let participant_id = required_flag_value(args, "--participant-id")?;
    let round = required_flag_value(args, "--round")?;
    statuses
        .iter()
        .find(|status| {
            status.episode_id == episode_id
                && status.participant_id == participant_id
                && status.round == round
        })
        .ok_or_else(|| {
            CliError::Usage(format!(
                "append-output command does not match any loaded adapter status for participant '{participant_id}'"
            ))
        })
}

fn require_append_arg_eq(args: &[String], flag: &str, expected: &str) -> Result<(), CliError> {
    require_cli_arg_eq(args, "adapter status append command", flag, expected)
}

fn require_cli_arg_eq(
    args: &[String],
    command: &str,
    flag: &str,
    expected: &str,
) -> Result<(), CliError> {
    let actual = required_flag_value(args, flag)?;
    if actual == expected {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "{command} {flag} mismatch; expected '{expected}', got '{actual}'"
    )))
}

fn file_len(path: &str) -> Result<u64, CliError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(LibrarianError::from(err).into()),
    }
}

fn build_adapter_prompt(request: &QuorumAdapterRequest) -> String {
    let prior_outputs = format_visible_prior_outputs(&request.visible_prior_outputs);
    format!(
        "# Mimir quorum participant request\n\n\
You are responding as a Mimir quorum participant.\n\
episode_id: {}\n\
participant_id: {}\n\
adapter: {}\n\
persona: {}\n\
model: {}\n\
round: {}\n\n\
Question:\n{}\n\n\
Evidence policy:\n{}\n\n\
Target project: {}\n\
Target scope: {}\n\n\
Visible prior outputs:\n{}\n\n\
Write only your participant response prose. Do not write canonical Mimir memory, \
do not edit project files, and do not call Mimir write commands. Mimir will record \
your response through `mimir-librarian quorum append-output`.\n",
        request.episode_id,
        request.participant.id,
        request.participant.adapter,
        request.participant.persona,
        request
            .participant
            .model
            .as_deref()
            .unwrap_or("<unspecified>"),
        quorum_round_name(request.round),
        request.question,
        request.evidence_policy,
        request.target_project.as_deref().unwrap_or("<none>"),
        request.target_scope.as_deref().unwrap_or("<none>"),
        prior_outputs,
    )
}

fn format_visible_prior_outputs(outputs: &[QuorumParticipantOutput]) -> String {
    if outputs.is_empty() {
        return "- <none>".to_string();
    }
    outputs
        .iter()
        .map(|output| {
            format!(
                "- output_id: {}\n  participant_id: {}\n  round: {}\n  response: {}",
                output.output_id,
                output.participant_id,
                quorum_round_name(output.round),
                output.response
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn adapter_plan_argv(
    adapter: &str,
    binary: Option<&str>,
    response_path: &str,
) -> Result<Vec<String>, CliError> {
    match adapter {
        "claude" => Ok(vec![
            binary.unwrap_or("claude").to_string(),
            "-p".to_string(),
        ]),
        "codex" => Ok(vec![
            binary.unwrap_or("codex").to_string(),
            "exec".to_string(),
            "--output-last-message".to_string(),
            response_path.to_string(),
            "-".to_string(),
        ]),
        other => Err(CliError::Usage(format!("unknown quorum adapter '{other}'"))),
    }
}

fn adapter_plan_stdout_path(
    adapter: &str,
    response_path: &str,
) -> Result<Option<String>, CliError> {
    match adapter {
        "claude" => Ok(Some(response_path.to_string())),
        "codex" => Ok(None),
        other => Err(CliError::Usage(format!("unknown quorum adapter '{other}'"))),
    }
}

fn append_output_plan_argv(
    parsed: &QuorumAdapterPlanArgs,
    request: &QuorumAdapterRequest,
    output_id: &str,
    prompt_path: &str,
    response_path: &str,
) -> Vec<String> {
    let mut argv = vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "append-output".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
        "--output-id".to_string(),
        output_id.to_string(),
        "--participant-id".to_string(),
        parsed.participant_id.clone(),
        "--round".to_string(),
        quorum_round_name(parsed.round).to_string(),
        "--prompt-file".to_string(),
        prompt_path.to_string(),
        "--response-file".to_string(),
        response_path.to_string(),
    ];
    for visible_id in &request.visible_prior_output_ids {
        argv.push("--visible-prior-output-id".to_string());
        argv.push(visible_id.clone());
    }
    argv
}

#[derive(Debug, Clone)]
struct QuorumSynthesisPlanArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    adapter: String,
    out_dir: PathBuf,
    binary: Option<String>,
}

struct QuorumSynthesisPlanPaths {
    transcript: PathBuf,
    prompt: PathBuf,
    result: PathBuf,
    status: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

struct QuorumSynthesisRunArgs {
    plan: QuorumSynthesisPlanArgs,
    timeout: Duration,
}

struct ParsedQuorumSynthesisArgs {
    plan: QuorumSynthesisPlanArgs,
    timeout: Option<Duration>,
}

struct QuorumAcceptSynthesisArgs {
    quorum_dir: PathBuf,
    episode_id: Option<String>,
    result_file: Option<PathBuf>,
    status_file: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct QuorumProposedSynthesis {
    schema_version: u32,
    episode_id: String,
    recommendation: String,
    decision_status: DecisionStatus,
    consensus_level: ConsensusLevel,
    confidence: f32,
    #[serde(default)]
    supporting_points: Vec<String>,
    #[serde(default)]
    dissenting_points: Vec<String>,
    #[serde(default)]
    unresolved_questions: Vec<String>,
    #[serde(default)]
    evidence_used: Vec<String>,
    #[serde(default)]
    participant_votes: Vec<ParticipantVote>,
    #[serde(default)]
    proposed_memory_drafts: Vec<String>,
}

fn quorum_synthesize_plan_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_synthesis_plan_args(args)?;
    Ok(QuorumCliOutcome::SynthesisPlan {
        plan: Box::new(build_quorum_synthesis_plan(&parsed)?),
    })
}

fn quorum_synthesize_run_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_synthesis_run_args(args)?;
    let plan = build_quorum_synthesis_plan(&parsed.plan)?;
    let status = execute_quorum_synthesis_plan(&plan, parsed.timeout)?;
    Ok(QuorumCliOutcome::SynthesisRun {
        run: Box::new(QuorumSynthesisRun {
            schema_version: QUORUM_SCHEMA_VERSION,
            status,
        }),
    })
}

fn quorum_accept_synthesis_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_accept_synthesis_args(args)?;
    let result_file = synthesis_result_file_for_accept(&parsed)?;
    let proposed = load_proposed_synthesis(&result_file)?;
    if let Some(expected_episode_id) = parsed.episode_id.as_deref() {
        if proposed.episode_id != expected_episode_id {
            return Err(CliError::Usage(format!(
                "synthesis result episode_id '{}' does not match --episode-id '{expected_episode_id}'",
                proposed.episode_id
            )));
        }
    }
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&proposed.episode_id)?;
    validate_proposed_synthesis(&proposed, &episode)?;
    let synthesize_args = synthesize_args_from_proposed(&parsed.quorum_dir, proposed);
    quorum_synthesize_from_args(&synthesize_args)
}

fn synthesis_result_file_for_accept(
    parsed: &QuorumAcceptSynthesisArgs,
) -> Result<PathBuf, CliError> {
    match (&parsed.result_file, &parsed.status_file) {
        (Some(_), Some(_)) => Err(CliError::Usage(
            "--result-file and --status-file are mutually exclusive".to_string(),
        )),
        (Some(result_file), None) => Ok(result_file.clone()),
        (None, Some(status_file)) => {
            let status = load_synthesis_status_artifact(status_file)?;
            validate_synthesis_status_for_accept(parsed, status_file, &status)?;
            Ok(PathBuf::from(status.result_path))
        }
        (None, None) => Err(CliError::Usage(
            "--result-file or --status-file is required".to_string(),
        )),
    }
}

fn build_quorum_synthesis_plan(
    parsed: &QuorumSynthesisPlanArgs,
) -> Result<QuorumSynthesisPlan, CliError> {
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&parsed.episode_id)?;
    let outputs = load_all_quorum_outputs(&store, &parsed.episode_id)?;
    let paths = synthesis_plan_paths(&parsed.out_dir, &parsed.episode_id);
    fs::create_dir_all(&parsed.out_dir).map_err(LibrarianError::from)?;
    write_synthesis_plan_files(&paths, &episode, &outputs)?;

    let result_path = paths.result.display().to_string();
    let status_path = paths.status.display().to_string();
    let prompt_path = paths.prompt.display().to_string();
    let command_argv = adapter_plan_argv(&parsed.adapter, parsed.binary.as_deref(), &result_path)?;
    let stdout_path = adapter_plan_stdout_path(&parsed.adapter, &result_path)?;
    let stdout_capture_path = stdout_path
        .clone()
        .unwrap_or_else(|| paths.stdout.display().to_string());
    let stderr_capture_path = paths.stderr.display().to_string();
    let accept_synthesis_argv = accept_synthesis_plan_argv(parsed, &result_path);

    Ok(QuorumSynthesisPlan {
        schema_version: QUORUM_SCHEMA_VERSION,
        adapter: parsed.adapter.clone(),
        quorum_dir: parsed.quorum_dir.display().to_string(),
        episode_id: parsed.episode_id.clone(),
        transcript_path: paths.transcript.display().to_string(),
        prompt_path: prompt_path.clone(),
        result_path: result_path.clone(),
        status_path,
        stdin_path: prompt_path,
        stdout_path,
        stdout_capture_path,
        stderr_capture_path,
        argv: command_argv,
        accept_synthesis_argv,
    })
}

fn execute_quorum_synthesis_plan(
    plan: &QuorumSynthesisPlan,
    timeout: Duration,
) -> Result<QuorumSynthesisRunStatus, CliError> {
    let program = plan
        .argv
        .first()
        .ok_or_else(|| CliError::Usage("synthesis plan argv must not be empty".to_string()))?;
    let started = Instant::now();
    let mut command = Command::new(program);
    command.args(&plan.argv[1..]);
    command.stdin(Stdio::from(
        fs::File::open(&plan.stdin_path).map_err(LibrarianError::from)?,
    ));
    command.stdout(Stdio::from(
        fs::File::create(&plan.stdout_capture_path).map_err(LibrarianError::from)?,
    ));
    command.stderr(Stdio::from(
        fs::File::create(&plan.stderr_capture_path).map_err(LibrarianError::from)?,
    ));

    let mut child = command.spawn().map_err(LibrarianError::from)?;
    let wait_result = child.wait_timeout(timeout);
    let (timed_out, exit_code, process_success) = match wait_result {
        Ok(Some(status)) => (false, status.code(), status.success()),
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            (true, None, false)
        }
        Err(err) => return Err(LibrarianError::from(err).into()),
    };
    let process_status = if timed_out {
        QuorumProcessStatus::TimedOut
    } else if process_success {
        QuorumProcessStatus::Succeeded
    } else {
        QuorumProcessStatus::Failed
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (result_valid, validation_error) = if process_success && !timed_out {
        match validate_synthesis_result_file(plan) {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        }
    } else {
        (false, None)
    };
    let status = QuorumSynthesisRunStatus {
        schema_version: QUORUM_SCHEMA_VERSION,
        adapter: plan.adapter.clone(),
        episode_id: plan.episode_id.clone(),
        status_path: plan.status_path.clone(),
        transcript_path: plan.transcript_path.clone(),
        prompt_path: plan.prompt_path.clone(),
        result_path: plan.result_path.clone(),
        stdout_capture_path: plan.stdout_capture_path.clone(),
        stderr_capture_path: plan.stderr_capture_path.clone(),
        success: process_success && !timed_out && result_valid,
        process_status: Some(process_status),
        timed_out,
        exit_code,
        duration_ms,
        result_valid,
        validation_error,
        result_bytes: file_len(&plan.result_path)?,
        stdout_bytes: file_len(&plan.stdout_capture_path)?,
        stderr_bytes: file_len(&plan.stderr_capture_path)?,
        accept_synthesis_argv: plan.accept_synthesis_argv.clone(),
    };
    write_synthesis_run_status(&status)?;
    Ok(status)
}

fn validate_synthesis_result_file(plan: &QuorumSynthesisPlan) -> Result<(), CliError> {
    let proposed = load_proposed_synthesis(Path::new(&plan.result_path))?;
    let store = QuorumStore::new(Path::new(&plan.quorum_dir));
    let episode = store.load_episode(&plan.episode_id)?;
    validate_proposed_synthesis(&proposed, &episode)
}

fn write_synthesis_run_status(status: &QuorumSynthesisRunStatus) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(status).map_err(LibrarianError::from)?;
    fs::write(&status.status_path, bytes).map_err(LibrarianError::from)?;
    Ok(())
}

fn load_synthesis_status_artifact(path: &Path) -> Result<QuorumSynthesisRunStatus, CliError> {
    let bytes = fs::read(path).map_err(LibrarianError::from)?;
    serde_json::from_slice(&bytes)
        .map_err(LibrarianError::from)
        .map_err(CliError::from)
}

fn validate_synthesis_status_for_accept(
    parsed: &QuorumAcceptSynthesisArgs,
    status_file: &Path,
    status: &QuorumSynthesisRunStatus,
) -> Result<(), CliError> {
    if status.schema_version != QUORUM_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported synthesis status schema version {}; expected {QUORUM_SCHEMA_VERSION}",
            status.schema_version
        )));
    }
    if let Some(expected_episode_id) = parsed.episode_id.as_deref() {
        if status.episode_id != expected_episode_id {
            return Err(CliError::Usage(format!(
                "synthesis status episode_id '{}' does not match --episode-id '{expected_episode_id}'",
                status.episode_id
            )));
        }
    }
    if !status.success {
        return Err(CliError::Usage(format!(
            "synthesis status for episode '{}' is not successful",
            status.episode_id
        )));
    }
    if status.timed_out || status.process_status != Some(QuorumProcessStatus::Succeeded) {
        return Err(CliError::Usage(format!(
            "synthesis status for episode '{}' did not finish successfully",
            status.episode_id
        )));
    }
    if !status.result_valid {
        return Err(CliError::Usage(format!(
            "synthesis status for episode '{}' has invalid proposed result",
            status.episode_id
        )));
    }
    if status.validation_error.is_some() {
        return Err(CliError::Usage(format!(
            "synthesis status for episode '{}' carries a validation error",
            status.episode_id
        )));
    }
    if status.result_bytes == 0 {
        return Err(CliError::Usage(format!(
            "synthesis status result artifact is empty: {}",
            status.result_path
        )));
    }
    let actual_result_bytes = file_len(&status.result_path)?;
    if actual_result_bytes != status.result_bytes {
        return Err(CliError::Usage(format!(
            "synthesis status result byte count mismatch for {}; status says {}, file has {}",
            status.result_path, status.result_bytes, actual_result_bytes
        )));
    }
    if status.status_path != status_file.display().to_string() {
        return Err(CliError::Usage(format!(
            "synthesis status path mismatch; expected '{}', got '{}'",
            status_file.display(),
            status.status_path
        )));
    }
    let args = accept_synthesis_args_from_status(status)?;
    require_cli_arg_eq(
        &args,
        "synthesis status accept command",
        "--quorum-dir",
        &parsed.quorum_dir.display().to_string(),
    )?;
    require_cli_arg_eq(
        &args,
        "synthesis status accept command",
        "--episode-id",
        &status.episode_id,
    )?;
    require_cli_arg_eq(
        &args,
        "synthesis status accept command",
        "--result-file",
        &status.result_path,
    )?;
    Ok(())
}

fn accept_synthesis_args_from_status(
    status: &QuorumSynthesisRunStatus,
) -> Result<Vec<String>, CliError> {
    if status.accept_synthesis_argv.len() < 3
        || status.accept_synthesis_argv[0] != "mimir-librarian"
        || status.accept_synthesis_argv[1] != "quorum"
        || status.accept_synthesis_argv[2] != "accept-synthesis"
    {
        return Err(CliError::Usage(format!(
            "synthesis status for episode '{}' does not carry a valid accept-synthesis command",
            status.episode_id
        )));
    }
    Ok(status
        .accept_synthesis_argv
        .iter()
        .skip(2)
        .cloned()
        .collect())
}

fn parse_quorum_synthesis_plan_args(args: &[String]) -> Result<QuorumSynthesisPlanArgs, CliError> {
    parse_quorum_synthesis_args(args, "synthesize-plan", false).map(|parsed| parsed.plan)
}

fn parse_quorum_synthesis_run_args(args: &[String]) -> Result<QuorumSynthesisRunArgs, CliError> {
    let mut parsed = parse_quorum_synthesis_args(args, "synthesize-run", true)?;
    let timeout = parsed
        .timeout
        .take()
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_QUORUM_ADAPTER_TIMEOUT_SECS));
    Ok(QuorumSynthesisRunArgs {
        plan: parsed.plan,
        timeout,
    })
}

fn parse_quorum_synthesis_args(
    args: &[String],
    command: &str,
    allow_timeout: bool,
) -> Result<ParsedQuorumSynthesisArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut adapter: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut binary: Option<String> = None;
    let mut timeout: Option<Duration> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--adapter" => {
                adapter = Some(parse_quorum_adapter(&take_value(
                    args,
                    &mut i,
                    "--adapter",
                )?)?);
            }
            "--out-dir" => {
                out_dir = Some(take_value(args, &mut i, "--out-dir")?.into());
            }
            "--binary" => {
                binary = Some(take_value(args, &mut i, "--binary")?);
            }
            "--timeout-secs" if allow_timeout => {
                let value = take_value(args, &mut i, "--timeout-secs")?;
                timeout = Some(Duration::from_secs(parse_positive_u64(
                    "--timeout-secs",
                    &value,
                )?));
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum {command} option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(ParsedQuorumSynthesisArgs {
        plan: QuorumSynthesisPlanArgs {
            quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
            episode_id: require_non_empty(episode_id, "--episode-id")?,
            adapter: require_non_empty(adapter, "--adapter")?,
            out_dir: require_path(out_dir, "--out-dir")?,
            binary,
        },
        timeout,
    })
}

fn parse_quorum_accept_synthesis_args(
    args: &[String],
) -> Result<QuorumAcceptSynthesisArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut result_file: Option<PathBuf> = None;
    let mut status_file: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--result-file" => {
                result_file = Some(take_value(args, &mut i, "--result-file")?.into());
            }
            "--status-file" => {
                status_file = Some(take_value(args, &mut i, "--status-file")?.into());
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum accept-synthesis option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumAcceptSynthesisArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id,
        result_file,
        status_file,
    })
}

fn synthesis_plan_paths(out_dir: &Path, episode_id: &str) -> QuorumSynthesisPlanPaths {
    let stem = format!("{}-synthesis", safe_file_stem(episode_id));
    QuorumSynthesisPlanPaths {
        transcript: out_dir.join(format!("{stem}-transcript.json")),
        prompt: out_dir.join(format!("{stem}-prompt.md")),
        result: out_dir.join(format!("{stem}-result.json")),
        status: out_dir.join(format!("{stem}-status.json")),
        stdout: out_dir.join(format!("{stem}-stdout.log")),
        stderr: out_dir.join(format!("{stem}-stderr.log")),
    }
}

fn write_synthesis_plan_files(
    paths: &QuorumSynthesisPlanPaths,
    episode: &QuorumEpisode,
    outputs: &[QuorumParticipantOutput],
) -> Result<(), CliError> {
    let transcript = serde_json::json!({
        "schema_version": QUORUM_SCHEMA_VERSION,
        "episode": episode,
        "outputs": outputs,
    });
    let transcript_json = serde_json::to_vec_pretty(&transcript).map_err(LibrarianError::from)?;
    fs::write(&paths.transcript, transcript_json).map_err(LibrarianError::from)?;
    fs::write(&paths.prompt, build_synthesis_prompt(episode, outputs))
        .map_err(LibrarianError::from)?;
    Ok(())
}

fn build_synthesis_prompt(episode: &QuorumEpisode, outputs: &[QuorumParticipantOutput]) -> String {
    let participants = episode
        .participants
        .iter()
        .map(|participant| {
            format!(
                "- participant_id: {}\n  adapter: {}\n  persona: {}\n  model: {}",
                participant.id,
                participant.adapter,
                participant.persona,
                participant.model.as_deref().unwrap_or("<unspecified>")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let outputs = format_synthesis_outputs(outputs);
    format!(
        "# Mimir quorum synthesis request\n\n\
You are synthesizing a governed Mimir quorum result.\n\
episode_id: {}\n\
requester: {}\n\
target_project: {}\n\
target_scope: {}\n\n\
Question:\n{}\n\n\
Evidence policy:\n{}\n\n\
Participants:\n{}\n\n\
Stored participant outputs:\n{}\n\n\
Return only a JSON object with these fields:\n\
{{\n\
  \"schema_version\": {},\n\
  \"episode_id\": \"{}\",\n\
  \"recommendation\": \"short recommendation\",\n\
  \"decision_status\": \"recommend|split|needs_evidence|reject|unsafe\",\n\
  \"consensus_level\": \"unanimous|strong_majority|weak_majority|contested|abstained\",\n\
  \"confidence\": 0.0,\n\
  \"supporting_points\": [\"point\"],\n\
  \"dissenting_points\": [\"dissent or limitation\"],\n\
  \"unresolved_questions\": [\"question\"],\n\
  \"evidence_used\": [\"source or artifact uri\"],\n\
  \"participant_votes\": [{{\"participant_id\":\"id\",\"vote\":\"agree|disagree|abstain\",\"confidence\":0.0,\"rationale\":\"short rationale\"}}],\n\
  \"proposed_memory_drafts\": [\"raw draft for librarian review\"]\n\
}}\n\n\
Preserve dissent and uncertainty. Do not write canonical Mimir memory, do not edit \
project files, and do not call Mimir write commands. This JSON is a proposed \
result artifact; it is recorded only after `mimir-librarian quorum accept-synthesis`.\n",
        episode.id,
        episode.requester,
        episode.target_project.as_deref().unwrap_or("<none>"),
        episode.target_scope.as_deref().unwrap_or("<none>"),
        episode.question,
        episode.evidence_policy,
        participants,
        outputs,
        QUORUM_SCHEMA_VERSION,
        episode.id,
    )
}

fn format_synthesis_outputs(outputs: &[QuorumParticipantOutput]) -> String {
    if outputs.is_empty() {
        return "- <none>".to_string();
    }
    outputs
        .iter()
        .map(|output| {
            format!(
                "- output_id: {}\n  participant_id: {}\n  round: {}\n  visible_prior_output_ids: {:?}\n  response: {}",
                output.output_id,
                output.participant_id,
                quorum_round_name(output.round),
                output.visible_prior_output_ids,
                output.response
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn accept_synthesis_plan_argv(parsed: &QuorumSynthesisPlanArgs, result_path: &str) -> Vec<String> {
    vec![
        "mimir-librarian".to_string(),
        "quorum".to_string(),
        "accept-synthesis".to_string(),
        "--quorum-dir".to_string(),
        parsed.quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        parsed.episode_id.clone(),
        "--result-file".to_string(),
        result_path.to_string(),
    ]
}

fn load_all_quorum_outputs(
    store: &QuorumStore,
    episode_id: &str,
) -> Result<Vec<QuorumParticipantOutput>, CliError> {
    let mut outputs = Vec::new();
    for round in [
        QuorumRound::Independent,
        QuorumRound::Critique,
        QuorumRound::Revision,
    ] {
        outputs.extend(store.load_round_outputs(episode_id, round)?);
    }
    Ok(outputs)
}

fn load_proposed_synthesis(path: &Path) -> Result<QuorumProposedSynthesis, CliError> {
    let raw = fs::read_to_string(path).map_err(LibrarianError::from)?;
    match serde_json::from_str(&raw) {
        Ok(proposed) => Ok(proposed),
        Err(err) => {
            if let Some(candidate) = json_object_slice(&raw) {
                return serde_json::from_str(candidate).map_err(|json_err| {
                    CliError::Usage(format!(
                        "synthesis result JSON is invalid: {json_err}; initial parse error: {err}"
                    ))
                });
            }
            Err(LibrarianError::from(err).into())
        }
    }
}

fn json_object_slice(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    (start <= end).then_some(&raw[start..=end])
}

fn validate_proposed_synthesis(
    proposed: &QuorumProposedSynthesis,
    episode: &QuorumEpisode,
) -> Result<(), CliError> {
    if proposed.schema_version != QUORUM_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "unsupported synthesis result schema version {}; expected {QUORUM_SCHEMA_VERSION}",
            proposed.schema_version
        )));
    }
    require_non_empty(Some(proposed.episode_id.clone()), "episode_id")?;
    if proposed.episode_id != episode.id {
        return Err(CliError::Usage(format!(
            "synthesis result episode_id '{}' does not match episode '{}'",
            proposed.episode_id, episode.id
        )));
    }
    require_non_empty(Some(proposed.recommendation.clone()), "recommendation")?;
    validate_unit_f32_value("confidence", proposed.confidence)?;
    validate_text_items("supporting_points", &proposed.supporting_points)?;
    validate_text_items("dissenting_points", &proposed.dissenting_points)?;
    validate_text_items("unresolved_questions", &proposed.unresolved_questions)?;
    validate_text_items("evidence_used", &proposed.evidence_used)?;
    validate_text_items("proposed_memory_drafts", &proposed.proposed_memory_drafts)?;
    validate_participant_votes(episode, &proposed.participant_votes)?;
    Ok(())
}

fn validate_unit_f32_value(flag: &str, value: f32) -> Result<(), CliError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::Usage(format!(
            "{flag} must be between 0.0 and 1.0"
        )));
    }
    Ok(())
}

fn validate_text_items(field: &str, values: &[String]) -> Result<(), CliError> {
    for (index, value) in values.iter().enumerate() {
        if value.trim().is_empty() {
            return Err(CliError::Usage(format!(
                "{field}[{index}] must not be empty"
            )));
        }
    }
    Ok(())
}

fn synthesize_args_from_proposed(
    quorum_dir: &Path,
    proposed: QuorumProposedSynthesis,
) -> Vec<String> {
    let mut args = vec![
        "--quorum-dir".to_string(),
        quorum_dir.display().to_string(),
        "--episode-id".to_string(),
        proposed.episode_id,
        "--recommendation".to_string(),
        proposed.recommendation,
        "--decision-status".to_string(),
        decision_status_name(proposed.decision_status).to_string(),
        "--consensus-level".to_string(),
        consensus_level_name(proposed.consensus_level).to_string(),
        "--confidence".to_string(),
        proposed.confidence.to_string(),
    ];
    for point in proposed.supporting_points {
        args.push("--supporting-point".to_string());
        args.push(point);
    }
    for point in proposed.dissenting_points {
        args.push("--dissenting-point".to_string());
        args.push(point);
    }
    for question in proposed.unresolved_questions {
        args.push("--unresolved-question".to_string());
        args.push(question);
    }
    for evidence in proposed.evidence_used {
        args.push("--evidence".to_string());
        args.push(evidence);
    }
    for vote in proposed.participant_votes {
        args.push("--participant-vote".to_string());
        args.push(format!(
            "{}:{}:{}:{}",
            vote.participant_id,
            vote_choice_name(vote.vote),
            vote.confidence,
            vote.rationale
        ));
    }
    for draft in proposed.proposed_memory_drafts {
        args.push("--proposed-memory-draft".to_string());
        args.push(draft);
    }
    args
}

struct QuorumSynthesizeArgs {
    quorum_dir: PathBuf,
    episode_id: String,
    recommendation: String,
    decision_status: DecisionStatus,
    consensus_level: ConsensusLevel,
    confidence: f32,
    supporting_points: Vec<String>,
    dissenting_points: Vec<String>,
    unresolved_questions: Vec<String>,
    evidence_used: Vec<String>,
    participant_votes: Vec<ParticipantVote>,
    proposed_memory_drafts: Vec<String>,
}

fn quorum_synthesize_from_args(args: &[String]) -> Result<QuorumCliOutcome, CliError> {
    let mut parsed = parse_quorum_synthesize_args(args)?;
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&parsed.episode_id)?;
    validate_synthesize_args(&episode, &parsed)?;
    parsed
        .evidence_used
        .extend(stored_output_evidence(&store, &parsed.episode_id)?);
    let result = QuorumResult {
        schema_version: QUORUM_SCHEMA_VERSION,
        episode_id: parsed.episode_id.clone(),
        question: episode.question,
        recommendation: parsed.recommendation,
        decision_status: parsed.decision_status,
        consensus_level: parsed.consensus_level,
        confidence: parsed.confidence,
        supporting_points: parsed.supporting_points,
        dissenting_points: parsed.dissenting_points,
        unresolved_questions: parsed.unresolved_questions,
        evidence_used: parsed.evidence_used,
        participant_votes: parsed.participant_votes,
        proposed_memory_drafts: parsed.proposed_memory_drafts,
    };
    let path = store.save_result(&result)?;
    Ok(QuorumCliOutcome::ResultSaved {
        episode_id: parsed.episode_id,
        path: path.display().to_string(),
    })
}

struct QuorumSubmitDraftsArgs {
    quorum_dir: PathBuf,
    drafts_dir: PathBuf,
    episode_id: String,
    project: Option<String>,
    operator: Option<String>,
    tags: Vec<String>,
}

fn quorum_submit_drafts_from_args(
    args: &[String],
    submitted_at: SystemTime,
) -> Result<QuorumCliOutcome, CliError> {
    let parsed = parse_quorum_submit_drafts_args(args)?;
    let store = QuorumStore::new(&parsed.quorum_dir);
    let episode = store.load_episode(&parsed.episode_id)?;
    let result = store.load_result(&parsed.episode_id)?;
    let draft_store = DraftStore::new(parsed.drafts_dir);
    let provenance_uri = format!("quorum://episode/{}", parsed.episode_id);
    let mut drafts = Vec::new();

    for raw_text in result.proposed_memory_drafts {
        if raw_text.trim().is_empty() {
            return Err(CliError::Usage(
                "quorum result contains an empty proposed memory draft".to_string(),
            ));
        }
        let mut metadata = DraftMetadata::new(DraftSourceSurface::ConsensusQuorum, submitted_at);
        metadata.source_agent = Some("quorum".to_string());
        metadata.source_project = parsed
            .project
            .clone()
            .or_else(|| episode.target_project.clone());
        metadata.operator.clone_from(&parsed.operator);
        metadata.provenance_uri = Some(provenance_uri.clone());
        metadata.context_tags =
            quorum_draft_context_tags(result.decision_status, result.consensus_level, &parsed.tags);
        let draft = Draft::with_metadata(raw_text, metadata);
        let path = draft_store.submit(&draft)?;
        drafts.push(SweepDraftOutcome {
            id: draft.id().to_hex(),
            path: path.display().to_string(),
            provenance_uri: provenance_uri.clone(),
        });
    }

    Ok(QuorumCliOutcome::DraftsSubmitted {
        episode_id: parsed.episode_id,
        submitted: drafts.len(),
        drafts,
    })
}

fn parse_quorum_submit_drafts_args(args: &[String]) -> Result<QuorumSubmitDraftsArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut drafts_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut project: Option<String> = None;
    let mut operator: Option<String> = None;
    let mut tags = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--drafts-dir" => {
                drafts_dir = Some(take_value(args, &mut i, "--drafts-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--project" => {
                project = Some(take_value(args, &mut i, "--project")?);
            }
            "--operator" => {
                operator = Some(take_value(args, &mut i, "--operator")?);
            }
            "--tag" => {
                tags.push(take_value(args, &mut i, "--tag")?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum submit-drafts option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumSubmitDraftsArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        drafts_dir: require_path(drafts_dir, "--drafts-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        project,
        operator,
        tags,
    })
}

fn quorum_draft_context_tags(
    decision_status: DecisionStatus,
    consensus_level: ConsensusLevel,
    extra_tags: &[String],
) -> Vec<String> {
    let mut tags = vec![
        "quorum".to_string(),
        decision_status_name(decision_status).to_string(),
        consensus_level_name(consensus_level).to_string(),
    ];
    tags.extend(extra_tags.iter().cloned());
    tags
}

fn parse_quorum_synthesize_args(args: &[String]) -> Result<QuorumSynthesizeArgs, CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut recommendation: Option<String> = None;
    let mut decision_status: Option<DecisionStatus> = None;
    let mut consensus_level: Option<ConsensusLevel> = None;
    let mut confidence: Option<f32> = None;
    let mut supporting_points = Vec::new();
    let mut dissenting_points = Vec::new();
    let mut unresolved_questions = Vec::new();
    let mut evidence_used = Vec::new();
    let mut participant_votes = Vec::new();
    let mut proposed_memory_drafts = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--recommendation" => {
                recommendation = Some(take_value(args, &mut i, "--recommendation")?);
            }
            "--decision-status" => {
                let value = take_value(args, &mut i, "--decision-status")?;
                decision_status = Some(parse_decision_status(&value)?);
            }
            "--consensus-level" => {
                let value = take_value(args, &mut i, "--consensus-level")?;
                consensus_level = Some(parse_consensus_level(&value)?);
            }
            "--confidence" => {
                let value = take_value(args, &mut i, "--confidence")?;
                confidence = Some(parse_unit_f32("--confidence", &value)?);
            }
            "--supporting-point" => {
                supporting_points.push(take_value(args, &mut i, "--supporting-point")?);
            }
            "--dissenting-point" => {
                dissenting_points.push(take_value(args, &mut i, "--dissenting-point")?);
            }
            "--unresolved-question" => {
                unresolved_questions.push(take_value(args, &mut i, "--unresolved-question")?);
            }
            "--evidence" => {
                evidence_used.push(take_value(args, &mut i, "--evidence")?);
            }
            "--participant-vote" => {
                let value = take_value(args, &mut i, "--participant-vote")?;
                participant_votes.push(parse_participant_vote(&value)?);
            }
            "--proposed-memory-draft" => {
                proposed_memory_drafts.push(take_value(args, &mut i, "--proposed-memory-draft")?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum synthesize option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok(QuorumSynthesizeArgs {
        quorum_dir: require_path(quorum_dir, "--quorum-dir")?,
        episode_id: require_non_empty(episode_id, "--episode-id")?,
        recommendation: require_non_empty(recommendation, "--recommendation")?,
        decision_status: decision_status
            .ok_or_else(|| CliError::Usage("--decision-status is required".to_string()))?,
        consensus_level: consensus_level
            .ok_or_else(|| CliError::Usage("--consensus-level is required".to_string()))?,
        confidence: confidence
            .ok_or_else(|| CliError::Usage("--confidence is required".to_string()))?,
        supporting_points,
        dissenting_points,
        unresolved_questions,
        evidence_used,
        participant_votes,
        proposed_memory_drafts,
    })
}

fn validate_participant_votes(
    episode: &QuorumEpisode,
    participant_votes: &[ParticipantVote],
) -> Result<(), CliError> {
    let expected: BTreeSet<String> = episode
        .participants
        .iter()
        .map(|participant| participant.id.clone())
        .collect();
    let mut seen = BTreeSet::new();
    for vote in participant_votes {
        require_non_empty(
            Some(vote.participant_id.clone()),
            "participant vote participant_id",
        )?;
        require_non_empty(Some(vote.rationale.clone()), "participant vote rationale")?;
        validate_unit_f32_value("participant vote confidence", vote.confidence)?;
        if !seen.insert(vote.participant_id.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate participant vote for '{}'",
                vote.participant_id
            )));
        }
        if !episode
            .participants
            .iter()
            .any(|participant| participant.id == vote.participant_id)
        {
            return Err(CliError::Usage(format!(
                "participant vote references unknown participant '{}'",
                vote.participant_id
            )));
        }
    }
    if seen != expected {
        let missing = expected
            .difference(&seen)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let unexpected = seen
            .difference(&expected)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let detail = match (missing.is_empty(), unexpected.is_empty()) {
            (false, false) => format!("missing: {missing}; unexpected: {unexpected}"),
            (false, true) => format!("missing: {missing}"),
            (true, false) => format!("unexpected: {unexpected}"),
            (true, true) => "vote set mismatch".to_string(),
        };
        return Err(CliError::Usage(format!(
            "synthesis result must include exactly one participant vote per episode participant ({detail})"
        )));
    }
    Ok(())
}

fn validate_synthesize_args(
    episode: &QuorumEpisode,
    parsed: &QuorumSynthesizeArgs,
) -> Result<(), CliError> {
    validate_unit_f32_value("confidence", parsed.confidence)?;
    validate_text_items("supporting_points", &parsed.supporting_points)?;
    validate_text_items("dissenting_points", &parsed.dissenting_points)?;
    validate_text_items("unresolved_questions", &parsed.unresolved_questions)?;
    validate_text_items("evidence_used", &parsed.evidence_used)?;
    validate_text_items("proposed_memory_drafts", &parsed.proposed_memory_drafts)?;
    validate_participant_votes(episode, &parsed.participant_votes)
}

fn stored_output_evidence(store: &QuorumStore, episode_id: &str) -> Result<Vec<String>, CliError> {
    let mut evidence = Vec::new();
    for round in [
        QuorumRound::Independent,
        QuorumRound::Critique,
        QuorumRound::Revision,
    ] {
        for output in store.load_round_outputs(episode_id, round)? {
            evidence.push(format!(
                "quorum-output://{episode_id}/{}/{}",
                quorum_round_name(round),
                output.output_id
            ));
        }
    }
    Ok(evidence)
}

fn parse_quorum_read_args(
    args: &[String],
    command: &str,
) -> Result<(PathBuf, String, QuorumRound), CliError> {
    let mut quorum_dir: Option<PathBuf> = None;
    let mut episode_id: Option<String> = None;
    let mut round: Option<QuorumRound> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--quorum-dir" => {
                quorum_dir = Some(take_value(args, &mut i, "--quorum-dir")?.into());
            }
            "--episode-id" => {
                episode_id = Some(take_value(args, &mut i, "--episode-id")?);
            }
            "--round" => {
                let value = take_value(args, &mut i, "--round")?;
                round = Some(parse_quorum_round(&value)?);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "unknown quorum {command} option '{other}'"
                )));
            }
        }
        i += 1;
    }

    Ok((
        require_path(quorum_dir, "--quorum-dir")?,
        require_non_empty(episode_id, "--episode-id")?,
        round.ok_or_else(|| CliError::Usage("--round is required".to_string()))?,
    ))
}

fn fill_default_agent(metadata: &mut DraftMetadata) {
    if metadata.source_agent.is_some() {
        return;
    }
    metadata.source_agent = match metadata.source_surface {
        DraftSourceSurface::ClaudeMemory => Some("claude".to_string()),
        DraftSourceSurface::CodexMemory => Some("codex".to_string()),
        DraftSourceSurface::CopilotSessionStore => Some("copilot".to_string()),
        _ => None,
    };
}

fn collect_sweep_files(path: &Path) -> Result<Vec<PathBuf>, CliError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(CliError::Usage(format!(
            "--path must be a file or directory: {}",
            path.display()
        )));
    }

    let mut files = Vec::new();
    collect_memory_files(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_memory_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), LibrarianError> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_memory_files(&path, files)?;
        } else if path.is_file() && is_supported_sweep_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_supported_sweep_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("md" | "markdown" | "txt")
    )
}

fn is_sweep_source_surface(source_surface: DraftSourceSurface) -> bool {
    matches!(
        source_surface,
        DraftSourceSurface::ClaudeMemory
            | DraftSourceSurface::CodexMemory
            | DraftSourceSurface::Directory
            | DraftSourceSurface::RepoHandoff
            | DraftSourceSurface::AgentExport
    )
}

fn file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn safe_file_stem(value: &str) -> String {
    let stem: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(80)
        .collect();
    if stem.trim_matches('-').is_empty() {
        "quorum".to_string()
    } else {
        stem
    }
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, CliError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a value")))
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn required_flag_value<'a>(args: &'a [String], flag: &str) -> Result<&'a str, CliError> {
    flag_value(args, flag).ok_or_else(|| {
        CliError::Usage(format!(
            "adapter status append command is missing required {flag}"
        ))
    })
}

fn require_text_or_file(
    inline: Option<String>,
    file: Option<PathBuf>,
    inline_flag: &str,
    file_flag: &str,
) -> Result<String, CliError> {
    match (inline, file) {
        (Some(_), Some(_)) => Err(CliError::Usage(format!(
            "{inline_flag} and {file_flag} are mutually exclusive"
        ))),
        (Some(value), None) => require_non_empty(Some(value), inline_flag),
        (None, Some(path)) => {
            let value = fs::read_to_string(path).map_err(LibrarianError::from)?;
            require_non_empty(Some(value), file_flag)
        }
        (None, None) => Err(CliError::Usage(format!(
            "{inline_flag} or {file_flag} is required"
        ))),
    }
}

fn require_non_empty(value: Option<String>, flag: &str) -> Result<String, CliError> {
    let value = value.ok_or_else(|| CliError::Usage(format!("{flag} is required")))?;
    if value.trim().is_empty() {
        return Err(CliError::Usage(format!("{flag} must not be empty")));
    }
    Ok(value)
}

fn require_path(value: Option<PathBuf>, flag: &str) -> Result<PathBuf, CliError> {
    value.ok_or_else(|| CliError::Usage(format!("{flag} is required")))
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, CliError> {
    value
        .parse::<u64>()
        .map_err(|_| CliError::Usage(format!("{flag} must be an integer: {value}")))
}

fn parse_usize(flag: &str, value: &str) -> Result<usize, CliError> {
    value
        .parse::<usize>()
        .map_err(|_| CliError::Usage(format!("{flag} must be an integer: {value}")))
}

fn parse_positive_u64(flag: &str, value: &str) -> Result<u64, CliError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| CliError::Usage(format!("{flag} must be an integer: {value}")))?;
    if parsed == 0 {
        return Err(CliError::Usage(format!("{flag} must be greater than zero")));
    }
    Ok(parsed)
}

fn parse_quorum_participant(value: &str) -> Result<QuorumParticipant, CliError> {
    let parts: Vec<&str> = value.split(':').collect();
    if !(3..=4).contains(&parts.len()) || parts.iter().take(3).any(|part| part.trim().is_empty()) {
        return Err(CliError::Usage(
            "--participant must be ID:ADAPTER:PERSONA[:MODEL]".to_string(),
        ));
    }
    Ok(QuorumParticipant {
        id: parts[0].to_string(),
        adapter: parts[1].to_string(),
        model: parts
            .get(3)
            .and_then(|model| (!model.trim().is_empty()).then(|| (*model).to_string())),
        persona: parts[2].to_string(),
        prompt_template_version: "v1".to_string(),
        runtime_surface: parts[1].to_string(),
        tool_permissions: Vec::new(),
    })
}

fn parse_quorum_round(value: &str) -> Result<QuorumRound, CliError> {
    match value {
        "independent" | "independent-round" | "independent_round" => Ok(QuorumRound::Independent),
        "critique" | "critique-round" | "critique_round" => Ok(QuorumRound::Critique),
        "revision" | "revision-round" | "revision_round" => Ok(QuorumRound::Revision),
        other => Err(CliError::Usage(format!("unknown quorum round '{other}'"))),
    }
}

fn parse_quorum_pilot_review_decision(value: &str) -> Result<QuorumPilotReviewDecision, CliError> {
    match value {
        "pass" => Ok(QuorumPilotReviewDecision::Pass),
        "needs-work" | "needs_work" => Ok(QuorumPilotReviewDecision::NeedsWork),
        "fail" => Ok(QuorumPilotReviewDecision::Fail),
        other => Err(CliError::Usage(format!(
            "unknown quorum pilot review decision '{other}'"
        ))),
    }
}

fn parse_quorum_pilot_review_finding(value: &str) -> Result<QuorumPilotReviewFinding, CliError> {
    let Some((severity, text)) = value.split_once(':') else {
        return Err(CliError::Usage(
            "--finding must be info|warning|blocker:TEXT".to_string(),
        ));
    };
    match severity {
        "info" | "warning" | "blocker" => {}
        other => {
            return Err(CliError::Usage(format!(
                "unknown quorum pilot review finding severity '{other}'"
            )));
        }
    }
    Ok(QuorumPilotReviewFinding {
        severity: severity.to_string(),
        text: require_non_empty(Some(text.to_string()), "--finding text")?,
    })
}

fn parse_quorum_adapter(value: &str) -> Result<String, CliError> {
    match value {
        "claude" | "codex" => Ok(value.to_string()),
        other => Err(CliError::Usage(format!("unknown quorum adapter '{other}'"))),
    }
}

fn parse_decision_status(value: &str) -> Result<DecisionStatus, CliError> {
    match value {
        "recommend" => Ok(DecisionStatus::Recommend),
        "split" => Ok(DecisionStatus::Split),
        "needs-evidence" | "needs_evidence" => Ok(DecisionStatus::NeedsEvidence),
        "reject" => Ok(DecisionStatus::Reject),
        "unsafe" => Ok(DecisionStatus::Unsafe),
        other => Err(CliError::Usage(format!(
            "unknown decision status '{other}'"
        ))),
    }
}

fn decision_status_name(status: DecisionStatus) -> &'static str {
    match status {
        DecisionStatus::Recommend => "recommend",
        DecisionStatus::Split => "split",
        DecisionStatus::NeedsEvidence => "needs_evidence",
        DecisionStatus::Reject => "reject",
        DecisionStatus::Unsafe => "unsafe",
    }
}

fn parse_consensus_level(value: &str) -> Result<ConsensusLevel, CliError> {
    match value {
        "unanimous" => Ok(ConsensusLevel::Unanimous),
        "strong-majority" | "strong_majority" => Ok(ConsensusLevel::StrongMajority),
        "weak-majority" | "weak_majority" => Ok(ConsensusLevel::WeakMajority),
        "contested" => Ok(ConsensusLevel::Contested),
        "abstained" => Ok(ConsensusLevel::Abstained),
        other => Err(CliError::Usage(format!(
            "unknown consensus level '{other}'"
        ))),
    }
}

fn consensus_level_name(level: ConsensusLevel) -> &'static str {
    match level {
        ConsensusLevel::Unanimous => "unanimous",
        ConsensusLevel::StrongMajority => "strong_majority",
        ConsensusLevel::WeakMajority => "weak_majority",
        ConsensusLevel::Contested => "contested",
        ConsensusLevel::Abstained => "abstained",
    }
}

fn parse_vote_choice(value: &str) -> Result<VoteChoice, CliError> {
    match value {
        "agree" => Ok(VoteChoice::Agree),
        "disagree" => Ok(VoteChoice::Disagree),
        "abstain" => Ok(VoteChoice::Abstain),
        other => Err(CliError::Usage(format!("unknown vote choice '{other}'"))),
    }
}

fn vote_choice_name(choice: VoteChoice) -> &'static str {
    match choice {
        VoteChoice::Agree => "agree",
        VoteChoice::Disagree => "disagree",
        VoteChoice::Abstain => "abstain",
    }
}

fn parse_participant_vote(value: &str) -> Result<ParticipantVote, CliError> {
    let parts: Vec<&str> = value.splitn(4, ':').collect();
    if parts.len() != 4
        || parts[0].trim().is_empty()
        || parts[1].trim().is_empty()
        || parts[2].trim().is_empty()
        || parts[3].trim().is_empty()
    {
        return Err(CliError::Usage(
            "--participant-vote must be PARTICIPANT:agree|disagree|abstain:CONFIDENCE:RATIONALE"
                .to_string(),
        ));
    }
    Ok(ParticipantVote {
        participant_id: parts[0].to_string(),
        vote: parse_vote_choice(parts[1])?,
        confidence: parse_unit_f32("--participant-vote confidence", parts[2])?,
        rationale: parts[3].to_string(),
    })
}

fn parse_unit_f32(flag: &str, value: &str) -> Result<f32, CliError> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| CliError::Usage(format!("{flag} must be a number: {value}")))?;
    if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
        return Err(CliError::Usage(format!(
            "{flag} must be between 0.0 and 1.0"
        )));
    }
    Ok(parsed)
}

fn quorum_round_name(round: QuorumRound) -> &'static str {
    match round {
        QuorumRound::Independent => "independent",
        QuorumRound::Critique => "critique",
        QuorumRound::Revision => "revision",
    }
}

fn system_time_to_unix_ms(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

fn clock_time_from_system_time(time: SystemTime) -> Result<ClockTime, LibrarianError> {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .map_err(|err| LibrarianError::ValidationClock {
            message: err.to_string(),
        })?
        .as_millis();
    let millis = u64::try_from(millis).unwrap_or(u64::MAX - 1);
    ClockTime::try_from_millis(millis).map_err(|err| LibrarianError::ValidationClock {
        message: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_quorum_shim(
        name: &str,
        script_body: &str,
    ) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join(name);
        let tmp_path = dir.path().join(format!(".{name}.tmp"));
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(script_body.as_bytes())?;
        file.sync_all()?;
        drop(file);
        let mut perms = std::fs::metadata(&tmp_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok((dir, path))
    }

    #[cfg(windows)]
    fn make_quorum_shim(
        name: &str,
        script_body: &str,
    ) -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let script_path = dir.path().join(format!("{name}.sh"));
        let path = dir.path().join(format!("{name}.cmd"));
        let tmp_script_path = dir.path().join(format!(".{name}.sh.tmp"));
        let tmp_path = dir.path().join(format!(".{name}.cmd.tmp"));
        let mut script = std::fs::File::create(&tmp_script_path)?;
        script.write_all(script_body.as_bytes())?;
        script.sync_all()?;
        drop(script);
        let mut command = std::fs::File::create(&tmp_path)?;
        command.write_all(format!("@echo off\r\nsh \"%~dp0{name}.sh\" %*\r\n").as_bytes())?;
        command.sync_all()?;
        drop(command);
        std::fs::rename(&tmp_script_path, &script_path)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok((dir, path))
    }

    #[test]
    fn submit_from_args_writes_pending_draft() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let args = vec![
            "--drafts-dir".to_string(),
            tmp.path().display().to_string(),
            "--text".to_string(),
            "Mimir should import Codex memory as drafts.".to_string(),
            "--source-surface".to_string(),
            "codex-memory".to_string(),
            "--agent".to_string(),
            "codex".to_string(),
            "--project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--operator".to_string(),
            "AlainDor".to_string(),
            "--provenance".to_string(),
            "file:///home/hasnobeef/.codex/memories/mimir.md".to_string(),
            "--tag".to_string(),
            "scope-model".to_string(),
        ];

        let outcome = match submit_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("submit failed: {err:?}").into()),
        };

        assert!(outcome.id.len() == 16);
        assert!(std::path::Path::new(&outcome.path).exists());
        let saved = std::fs::read_to_string(outcome.path)?;
        assert!(saved.contains("\"source_surface\": \"codex_memory\""));
        assert!(saved.contains("\"source_agent\": \"codex\""));
        assert!(saved.contains("\"operator\": \"AlainDor\""));
        Ok(())
    }

    #[test]
    fn submit_from_args_rejects_missing_text() {
        let args = Vec::new();
        match submit_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => assert!(message.contains("--text is required")),
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn submit_from_args_writes_consensus_quorum_draft() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let args = vec![
            "--drafts-dir".to_string(),
            tmp.path().display().to_string(),
            "--text".to_string(),
            "Quorum recommends keeping remote sync explicit; dissent preserved.".to_string(),
            "--source-surface".to_string(),
            "consensus-quorum".to_string(),
            "--agent".to_string(),
            "quorum".to_string(),
            "--project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--operator".to_string(),
            "AlainDor".to_string(),
            "--provenance".to_string(),
            "quorum://episode/2026-04-24T21:00:00Z".to_string(),
            "--tag".to_string(),
            "quorum".to_string(),
            "--tag".to_string(),
            "strong_majority".to_string(),
        ];

        let outcome = match submit_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("submit failed: {err:?}").into()),
        };

        let saved = std::fs::read_to_string(outcome.path)?;
        assert!(saved.contains("\"source_surface\": \"consensus_quorum\""));
        assert!(saved.contains("\"source_agent\": \"quorum\""));
        assert!(saved.contains("\"provenance_uri\": \"quorum://episode/2026-04-24T21:00:00Z\""));
        assert!(saved.contains("\"quorum\""));
        assert!(saved.contains("\"strong_majority\""));
        Ok(())
    }

    #[test]
    fn quorum_create_from_args_writes_episode() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-001".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should quorum artifacts stay governed drafts?".to_string(),
            "--target-project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--target-scope".to_string(),
            "project".to_string(),
            "--participant".to_string(),
            "claude:claude:architect:claude-sonnet-4.6".to_string(),
            "--participant".to_string(),
            "codex:codex:implementation_engineer:gpt-5.5".to_string(),
        ];

        let outcome = match quorum_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum create failed: {err:?}").into()),
        };

        let path = match outcome {
            QuorumCliOutcome::EpisodeCreated { id, path } => {
                assert_eq!(id, "qr-cli-001");
                path
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };
        assert!(std::path::Path::new(&path).exists());
        let saved = std::fs::read_to_string(path)?;
        assert!(saved.contains("\"id\": \"qr-cli-001\""));
        assert!(saved.contains("\"participant"));
        Ok(())
    }

    #[test]
    fn quorum_append_output_from_args_writes_independent_output(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let create_args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-002".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should Mimir keep quorum outputs auditable?".to_string(),
            "--participant".to_string(),
            "claude:claude:architect".to_string(),
        ];
        quorum_from_args(&create_args, SystemTime::UNIX_EPOCH)?;

        let append_args = vec![
            "append-output".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-002".to_string(),
            "--output-id".to_string(),
            "out-independent-claude".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--prompt".to_string(),
            "Answer independently.".to_string(),
            "--response".to_string(),
            "Keep the artifact auditable.".to_string(),
            "--evidence".to_string(),
            "docs/concepts/consensus-quorum.md".to_string(),
        ];

        let outcome = match quorum_from_args(&append_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum append failed: {err:?}").into()),
        };

        match outcome {
            QuorumCliOutcome::OutputAppended {
                episode_id,
                output_id,
                path,
            } => {
                assert_eq!(episode_id, "qr-cli-002");
                assert_eq!(output_id, "out-independent-claude");
                assert!(std::path::Path::new(&path).exists());
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        Ok(())
    }

    #[test]
    fn quorum_visible_from_args_returns_complete_independent_round(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let create_args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-003".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should critique wait for complete independent outputs?".to_string(),
            "--participant".to_string(),
            "claude:claude:architect".to_string(),
            "--participant".to_string(),
            "codex:codex:implementation_engineer".to_string(),
        ];
        quorum_from_args(&create_args, SystemTime::UNIX_EPOCH)?;

        for participant_id in ["claude", "codex"] {
            let append_args = vec![
                "append-output".to_string(),
                "--quorum-dir".to_string(),
                tmp.path().display().to_string(),
                "--episode-id".to_string(),
                "qr-cli-003".to_string(),
                "--output-id".to_string(),
                format!("out-independent-{participant_id}"),
                "--participant-id".to_string(),
                participant_id.to_string(),
                "--round".to_string(),
                "independent".to_string(),
                "--prompt".to_string(),
                "Answer independently.".to_string(),
                "--response".to_string(),
                format!("Independent answer from {participant_id}."),
            ];
            quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        }

        let visible_args = vec![
            "visible".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-003".to_string(),
            "--round".to_string(),
            "critique".to_string(),
        ];
        let outcome = match quorum_from_args(&visible_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum visible failed: {err:?}").into()),
        };

        match outcome {
            QuorumCliOutcome::VisibleOutputs { round, outputs, .. } => {
                assert_eq!(round, "critique");
                let ids: Vec<_> = outputs
                    .iter()
                    .map(|output| output.output_id.as_str())
                    .collect();
                assert_eq!(ids, vec!["out-independent-claude", "out-independent-codex"]);
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        Ok(())
    }

    #[test]
    fn quorum_adapter_request_from_args_returns_contract() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let create_args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-004".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should adapters consume a stable JSON request?".to_string(),
            "--participant".to_string(),
            "claude:claude:architect:claude-sonnet-4.6".to_string(),
        ];
        quorum_from_args(&create_args, SystemTime::UNIX_EPOCH)?;

        let request_args = vec![
            "adapter-request".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-004".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
        ];
        let outcome = match quorum_from_args(&request_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum adapter-request failed: {err:?}").into()),
        };

        match outcome {
            QuorumCliOutcome::AdapterRequest { request } => {
                assert_eq!(request.episode_id, "qr-cli-004");
                assert_eq!(request.participant.id, "claude");
                assert_eq!(request.round, QuorumRound::Independent);
                assert!(request.visible_prior_outputs.is_empty());
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        Ok(())
    }

    #[test]
    fn quorum_synthesize_from_args_saves_structured_result(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let create_args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-005".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should Mimir preserve dissent in quorum results?".to_string(),
            "--participant".to_string(),
            "claude:claude:architect".to_string(),
            "--participant".to_string(),
            "codex:codex:implementation_engineer".to_string(),
        ];
        quorum_from_args(&create_args, SystemTime::UNIX_EPOCH)?;

        for participant_id in ["claude", "codex"] {
            let append_args = vec![
                "append-output".to_string(),
                "--quorum-dir".to_string(),
                tmp.path().display().to_string(),
                "--episode-id".to_string(),
                "qr-cli-005".to_string(),
                "--output-id".to_string(),
                format!("out-independent-{participant_id}"),
                "--participant-id".to_string(),
                participant_id.to_string(),
                "--round".to_string(),
                "independent".to_string(),
                "--prompt".to_string(),
                "Answer independently.".to_string(),
                "--response".to_string(),
                format!("{participant_id} says dissent must stay visible."),
            ];
            quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        }

        let synthesize_args = vec![
            "synthesize".to_string(),
            "--quorum-dir".to_string(),
            tmp.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-005".to_string(),
            "--recommendation".to_string(),
            "Preserve dissent as first-class evidence.".to_string(),
            "--decision-status".to_string(),
            "recommend".to_string(),
            "--consensus-level".to_string(),
            "strong-majority".to_string(),
            "--confidence".to_string(),
            "0.82".to_string(),
            "--supporting-point".to_string(),
            "Both independent outputs support dissent preservation.".to_string(),
            "--dissenting-point".to_string(),
            "Codex still wants review burden tracked.".to_string(),
            "--participant-vote".to_string(),
            "claude:agree:0.88:Preserves auditability".to_string(),
            "--participant-vote".to_string(),
            "codex:disagree:0.41:Review burden remains unresolved".to_string(),
            "--proposed-memory-draft".to_string(),
            "Quorum results must preserve dissent as first-class evidence.".to_string(),
        ];
        let outcome = match quorum_from_args(&synthesize_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum synthesize failed: {err:?}").into()),
        };

        let path = match outcome {
            QuorumCliOutcome::ResultSaved { episode_id, path } => {
                assert_eq!(episode_id, "qr-cli-005");
                path
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };
        assert!(std::path::Path::new(&path).exists());
        let result = QuorumStore::new(tmp.path()).load_result("qr-cli-005")?;
        assert_eq!(result.participant_votes.len(), 2);
        assert_eq!(result.dissenting_points.len(), 1);
        assert!(result
            .evidence_used
            .iter()
            .any(|item| item.contains("out-independent-claude")));
        Ok(())
    }

    #[test]
    fn quorum_submit_drafts_from_result_uses_consensus_surface(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let create_args = vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--id".to_string(),
            "qr-cli-006".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should quorum drafts enter the governed draft store?".to_string(),
            "--target-project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--participant".to_string(),
            "claude:claude:architect".to_string(),
        ];
        quorum_from_args(&create_args, SystemTime::UNIX_EPOCH)?;
        let synthesize_args = vec![
            "synthesize".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-006".to_string(),
            "--recommendation".to_string(),
            "Submit only proposed memory drafts.".to_string(),
            "--decision-status".to_string(),
            "recommend".to_string(),
            "--consensus-level".to_string(),
            "unanimous".to_string(),
            "--confidence".to_string(),
            "0.9".to_string(),
            "--participant-vote".to_string(),
            "claude:agree:0.9:Clear governance path".to_string(),
            "--proposed-memory-draft".to_string(),
            "Quorum-proposed memory drafts must enter through consensus_quorum drafts.".to_string(),
        ];
        quorum_from_args(&synthesize_args, SystemTime::UNIX_EPOCH)?;

        let submit_args = vec![
            "submit-drafts".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-006".to_string(),
            "--operator".to_string(),
            "AlainDor".to_string(),
        ];
        let outcome = match quorum_from_args(&submit_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("quorum submit-drafts failed: {err:?}").into()),
        };

        match outcome {
            QuorumCliOutcome::DraftsSubmitted {
                episode_id,
                submitted,
                drafts: submitted_drafts,
            } => {
                assert_eq!(episode_id, "qr-cli-006");
                assert_eq!(submitted, 1);
                assert_eq!(submitted_drafts.len(), 1);
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        let staged = DraftStore::new(drafts.path()).list(mimir_librarian::DraftState::Pending)?;
        assert_eq!(staged.len(), 1);
        assert_eq!(
            staged[0].metadata().source_surface,
            DraftSourceSurface::ConsensusQuorum
        );
        assert_eq!(staged[0].metadata().source_agent.as_deref(), Some("quorum"));
        assert_eq!(
            staged[0].metadata().provenance_uri.as_deref(),
            Some("quorum://episode/qr-cli-006")
        );
        assert!(staged[0]
            .metadata()
            .context_tags
            .contains(&"unanimous".to_string()));
        Ok(())
    }

    fn recorded_fixture_create_args(quorum_dir: &std::path::Path) -> Vec<String> {
        vec![
            "create".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--id".to_string(),
            "qr-cli-007".to_string(),
            "--requester".to_string(),
            "operator:AlainDor".to_string(),
            "--question".to_string(),
            "Should recorded quorum fixtures preserve the draft boundary?".to_string(),
            "--target-project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--participant".to_string(),
            "claude:claude:architect".to_string(),
            "--participant".to_string(),
            "codex:codex:implementation_engineer".to_string(),
        ]
    }

    fn recorded_fixture_adapter_request_args(quorum_dir: &std::path::Path) -> Vec<String> {
        vec![
            "adapter-request".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
        ]
    }

    fn recorded_fixture_append_args(
        quorum_dir: &std::path::Path,
        participant_id: &str,
        response: &str,
    ) -> Vec<String> {
        vec![
            "append-output".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--output-id".to_string(),
            format!("out-independent-{participant_id}"),
            "--participant-id".to_string(),
            participant_id.to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--prompt".to_string(),
            "Answer independently from the fixture.".to_string(),
            "--response".to_string(),
            response.to_string(),
        ]
    }

    fn recorded_fixture_synthesize_args(quorum_dir: &std::path::Path) -> Vec<String> {
        vec![
            "synthesize".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--recommendation".to_string(),
            "Keep recorded quorum fixtures on the draft path.".to_string(),
            "--decision-status".to_string(),
            "recommend".to_string(),
            "--consensus-level".to_string(),
            "strong-majority".to_string(),
            "--confidence".to_string(),
            "0.87".to_string(),
            "--participant-vote".to_string(),
            "claude:agree:0.9:Audit trail is preserved".to_string(),
            "--participant-vote".to_string(),
            "codex:agree:0.84:Draft boundary remains intact".to_string(),
            "--proposed-memory-draft".to_string(),
            "Recorded quorum fixture outputs must enter Mimir through consensus_quorum drafts."
                .to_string(),
        ]
    }

    fn recorded_fixture_synthesize_without_drafts_args(
        quorum_dir: &std::path::Path,
    ) -> Vec<String> {
        vec![
            "synthesize".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--recommendation".to_string(),
            "Keep recorded quorum fixtures reviewable.".to_string(),
            "--decision-status".to_string(),
            "recommend".to_string(),
            "--consensus-level".to_string(),
            "strong-majority".to_string(),
            "--confidence".to_string(),
            "0.87".to_string(),
            "--participant-vote".to_string(),
            "claude:agree:0.9:Audit trail is preserved".to_string(),
            "--participant-vote".to_string(),
            "codex:agree:0.84:Draft boundary remains intact".to_string(),
        ]
    }

    fn recorded_fixture_submit_args(
        quorum_dir: &std::path::Path,
        drafts_dir: &std::path::Path,
    ) -> Vec<String> {
        vec![
            "submit-drafts".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--drafts-dir".to_string(),
            drafts_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--operator".to_string(),
            "AlainDor".to_string(),
        ]
    }

    fn create_test_pilot_plan(
        quorum_dir: &std::path::Path,
        out_dir: &std::path::Path,
        drafts_dir: &std::path::Path,
        through_round: &str,
    ) -> Result<Box<QuorumPilotPlan>, Box<dyn std::error::Error>> {
        create_test_pilot_plan_with_args(
            quorum_dir,
            out_dir,
            drafts_dir,
            through_round,
            "codex",
            Vec::new(),
        )
    }

    fn create_test_pilot_plan_with_synthesizer(
        quorum_dir: &std::path::Path,
        out_dir: &std::path::Path,
        drafts_dir: &std::path::Path,
        synthesizer_binary: &std::path::Path,
    ) -> Result<Box<QuorumPilotPlan>, Box<dyn std::error::Error>> {
        create_test_pilot_plan_with_args(
            quorum_dir,
            out_dir,
            drafts_dir,
            "independent",
            "codex",
            vec![
                "--synthesizer-binary".to_string(),
                synthesizer_binary.display().to_string(),
            ],
        )
    }

    fn create_test_pilot_plan_with_runtime_binaries(
        quorum_dir: &std::path::Path,
        out_dir: &std::path::Path,
        drafts_dir: &std::path::Path,
        claude_binary: &std::path::Path,
        codex_binary: &std::path::Path,
        synthesizer_binary: &std::path::Path,
    ) -> Result<Box<QuorumPilotPlan>, Box<dyn std::error::Error>> {
        create_test_pilot_plan_with_args(
            quorum_dir,
            out_dir,
            drafts_dir,
            "independent",
            "codex",
            vec![
                "--adapter-binary".to_string(),
                format!("claude={}", claude_binary.display()),
                "--adapter-binary".to_string(),
                format!("codex={}", codex_binary.display()),
                "--synthesizer-binary".to_string(),
                synthesizer_binary.display().to_string(),
            ],
        )
    }

    fn create_test_pilot_plan_with_args(
        quorum_dir: &std::path::Path,
        out_dir: &std::path::Path,
        drafts_dir: &std::path::Path,
        through_round: &str,
        synthesizer: &str,
        extra_args: Vec<String>,
    ) -> Result<Box<QuorumPilotPlan>, Box<dyn std::error::Error>> {
        let mut plan_args = vec![
            "pilot-plan".to_string(),
            "--quorum-dir".to_string(),
            quorum_dir.display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--through-round".to_string(),
            through_round.to_string(),
            "--out-dir".to_string(),
            out_dir.display().to_string(),
            "--drafts-dir".to_string(),
            drafts_dir.display().to_string(),
            "--synthesizer".to_string(),
            synthesizer.to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        plan_args.extend(extra_args);
        match quorum_from_args(&plan_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::PilotPlan { plan } => Ok(plan),
            other => Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
    }

    #[test]
    fn quorum_recorded_fixture_smoke_test_runs_to_pending_draft(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;

        let outcome = quorum_from_args(
            &recorded_fixture_adapter_request_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        match outcome {
            QuorumCliOutcome::AdapterRequest { request } => {
                assert_eq!(request.episode_id, "qr-cli-007");
                assert_eq!(request.participant.id, "claude");
                assert!(request.visible_prior_outputs.is_empty());
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }

        for (participant_id, response) in [
            ("claude", "Preserve the draft boundary and audit trail."),
            (
                "codex",
                "Submit fixture-derived memories through consensus_quorum drafts.",
            ),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }

        quorum_from_args(
            &recorded_fixture_synthesize_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let outcome = quorum_from_args(
            &recorded_fixture_submit_args(quorum.path(), drafts.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        match outcome {
            QuorumCliOutcome::DraftsSubmitted {
                episode_id,
                submitted,
                drafts: submitted_drafts,
            } => {
                assert_eq!(episode_id, "qr-cli-007");
                assert_eq!(submitted, 1);
                assert_eq!(submitted_drafts.len(), 1);
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }

        let result = QuorumStore::new(quorum.path()).load_result("qr-cli-007")?;
        assert_eq!(result.participant_votes.len(), 2);
        assert!(result
            .evidence_used
            .iter()
            .any(|item| item.contains("out-independent-codex")));
        let staged = DraftStore::new(drafts.path()).list(mimir_librarian::DraftState::Pending)?;
        assert_eq!(staged.len(), 1);
        assert_eq!(
            staged[0].metadata().source_project.as_deref(),
            Some("buildepicshit/Mimir")
        );
        assert!(staged[0]
            .metadata()
            .context_tags
            .contains(&"strong_majority".to_string()));
        Ok(())
    }

    #[test]
    fn quorum_pilot_plan_writes_replayable_manifest() -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;

        let plan_args = vec![
            "pilot-plan".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--through-round".to_string(),
            "critique".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--synthesizer".to_string(),
            "codex".to_string(),
            "--adapter-binary".to_string(),
            "claude=/bin/claude-fixture".to_string(),
            "--adapter-binary".to_string(),
            "codex=/bin/codex-fixture".to_string(),
            "--timeout-secs".to_string(),
            "7".to_string(),
            "--require-proposed-drafts".to_string(),
            "1".to_string(),
            "--project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--operator".to_string(),
            "operator:AlainDor".to_string(),
            "--tag".to_string(),
            "pilot".to_string(),
        ];
        let outcome = quorum_from_args(&plan_args, SystemTime::UNIX_EPOCH)?;
        let plan = match outcome {
            QuorumCliOutcome::PilotPlan { plan } => plan,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(plan.episode_id, "qr-cli-007");
        assert_eq!(plan.through_round, "critique");
        assert_eq!(plan.synthesizer_adapter, "codex");
        assert_eq!(plan.timeout_secs, 7);
        assert_eq!(plan.required_proposed_drafts, 1);
        assert_eq!(plan.participants.len(), 2);
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(plan.steps[0].name, "run_rounds");
        assert_eq!(plan.steps[0].argv[2], "adapter-run-rounds");
        assert!(plan.steps[0]
            .argv
            .windows(2)
            .any(|pair| pair == ["--adapter-binary", "claude=/bin/claude-fixture"]));
        assert_eq!(plan.steps[1].name, "run_synthesis");
        assert_eq!(plan.steps[1].argv[2], "synthesize-run");
        assert!(plan.steps[1]
            .argv
            .windows(2)
            .any(|pair| pair == ["--binary", "/bin/codex-fixture"]));
        assert_eq!(plan.steps[2].name, "accept_synthesis");
        assert_eq!(plan.steps[2].argv[2], "accept-synthesis");
        assert!(plan.steps[2]
            .argv
            .windows(2)
            .any(|pair| pair[0] == "--status-file" && pair[1].ends_with("-synthesis-status.json")));
        assert_eq!(plan.steps[3].name, "submit_drafts");
        assert_eq!(plan.steps[3].argv[2], "submit-drafts");
        assert_eq!(plan.artifacts.round_statuses.len(), 2);
        assert!(std::path::Path::new(&plan.manifest_path).exists());

        let manifest_bytes = std::fs::read(&plan.manifest_path)?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
        assert_eq!(manifest["episode_id"], "qr-cli-007");
        assert_eq!(manifest["required_proposed_drafts"], 1);
        assert_eq!(manifest["steps"].as_array().map(Vec::len), Some(4));
        Ok(())
    }

    #[test]
    fn quorum_pilot_status_reports_pending_manifest() -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan(quorum.path(), out.path(), drafts.path(), "independent")?;

        let outcome = quorum_from_args(
            &[
                "pilot-status".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let status = match outcome {
            QuorumCliOutcome::PilotStatus { status } => status,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(status.episode_id, "qr-cli-007");
        assert_eq!(status.overall_status, QuorumPilotGateStatus::Pending);
        assert!(!status.complete);
        assert_eq!(status.gates.len(), 4);
        assert!(
            status
                .gates
                .iter()
                .any(|gate| gate.name == "run_rounds"
                    && gate.status == QuorumPilotGateStatus::Pending)
        );
        Ok(())
    }

    #[test]
    fn quorum_pilot_status_reports_complete_after_manifest_gates(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Run the pilot through explicit gates.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"Both participants completed the independent round.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"The pilot preserves audit gates\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"The manifest is replayable\"}],\"proposed_memory_drafts\":[\"Replayable quorum pilots must complete adapter, synthesis, acceptance, and draft gates before being treated as submitted evidence.\"]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_synthesizer(
            quorum.path(),
            out.path(),
            drafts.path(),
            &synth_shim,
        )?;

        quorum_from_args(
            &[
                "adapter-run-rounds".to_string(),
                "--quorum-dir".to_string(),
                quorum.path().display().to_string(),
                "--episode-id".to_string(),
                "qr-cli-007".to_string(),
                "--through-round".to_string(),
                "independent".to_string(),
                "--out-dir".to_string(),
                out.path().display().to_string(),
                "--adapter-binary".to_string(),
                format!("claude={}", claude_shim.display()),
                "--adapter-binary".to_string(),
                format!("codex={}", codex_shim.display()),
                "--timeout-secs".to_string(),
                "5".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        quorum_from_args(
            &[
                "synthesize-run".to_string(),
                "--quorum-dir".to_string(),
                quorum.path().display().to_string(),
                "--episode-id".to_string(),
                "qr-cli-007".to_string(),
                "--adapter".to_string(),
                "codex".to_string(),
                "--out-dir".to_string(),
                out.path().display().to_string(),
                "--binary".to_string(),
                synth_shim.display().to_string(),
                "--timeout-secs".to_string(),
                "5".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        quorum_from_args(
            &[
                "accept-synthesis".to_string(),
                "--quorum-dir".to_string(),
                quorum.path().display().to_string(),
                "--episode-id".to_string(),
                "qr-cli-007".to_string(),
                "--status-file".to_string(),
                plan.artifacts.synthesis_status_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        quorum_from_args(
            &recorded_fixture_submit_args(quorum.path(), drafts.path()),
            SystemTime::UNIX_EPOCH,
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-status".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let status = match outcome {
            QuorumCliOutcome::PilotStatus { status } => status,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(status.overall_status, QuorumPilotGateStatus::Complete);
        assert!(status.complete);
        assert!(status
            .gates
            .iter()
            .all(|gate| gate.status == QuorumPilotGateStatus::Complete));
        Ok(())
    }

    #[test]
    fn quorum_pilot_run_executes_manifest_gates() -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-run-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot-run Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-run-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot-run Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-run-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Execute the pilot manifest through explicit gates.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"The manifest executor used the existing gates.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Round output was recorded\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Synthesis was accepted explicitly\"}],\"proposed_memory_drafts\":[\"Pilot manifests may be executed only by replaying the existing quorum gates in order.\"]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_runtime_binaries(
            quorum.path(),
            out.path(),
            drafts.path(),
            &claude_shim,
            &codex_shim,
            &synth_shim,
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let run = match outcome {
            QuorumCliOutcome::PilotRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(run.success);
        assert!(run.skipped_steps.is_empty());
        assert_eq!(run.failed_step, None);
        assert_eq!(run.error, None);
        assert_eq!(
            run.executed_steps,
            vec![
                "run_rounds".to_string(),
                "run_synthesis".to_string(),
                "accept_synthesis".to_string(),
                "submit_drafts".to_string(),
            ]
        );
        assert_eq!(
            run.final_status.overall_status,
            QuorumPilotGateStatus::Complete
        );
        assert!(QuorumStore::new(quorum.path())
            .load_result("qr-cli-007")
            .is_ok());
        let staged = DraftStore::new(drafts.path()).list(mimir_librarian::DraftState::Pending)?;
        assert_eq!(staged.len(), 1);
        Ok(())
    }

    #[test]
    fn quorum_pilot_run_reports_failed_step_with_final_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-run-fail-report-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot failure report Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-run-fail-report-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot failure report Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-run-invalid-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Incomplete synthesis.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.8,\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.8,\"rationale\":\"Only one vote\"}]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_runtime_binaries(
            quorum.path(),
            out.path(),
            drafts.path(),
            &claude_shim,
            &codex_shim,
            &synth_shim,
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let run = match outcome {
            QuorumCliOutcome::PilotRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(!run.success);
        assert_eq!(run.executed_steps, vec!["run_rounds".to_string()]);
        assert!(run.skipped_steps.is_empty());
        assert_eq!(run.failed_step.as_deref(), Some("run_synthesis"));
        assert!(run
            .error
            .as_deref()
            .is_some_and(|message| message.contains("run_synthesis")));
        assert_eq!(
            run.final_status.overall_status,
            QuorumPilotGateStatus::Failed
        );
        assert!(run.final_status.gates.iter().any(|gate| {
            gate.name == "run_synthesis" && gate.status == QuorumPilotGateStatus::Failed
        }));
        assert!(QuorumStore::new(quorum.path())
            .load_result("qr-cli-007")
            .is_err());
        Ok(())
    }

    #[test]
    fn quorum_pilot_run_skips_completed_gates_on_rerun() -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-run-rerun-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot rerun Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-run-rerun-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot rerun Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-run-rerun-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Rerun complete manifests by skipping completed gates.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"The first run completed every gate.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Round output was recorded\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Rerun should not duplicate outputs\"}],\"proposed_memory_drafts\":[\"Pilot manifest reruns should skip gates that already have complete recorded artifacts.\"]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_runtime_binaries(
            quorum.path(),
            out.path(),
            drafts.path(),
            &claude_shim,
            &codex_shim,
            &synth_shim,
        )?;

        let first = quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        assert!(matches!(
            first,
            QuorumCliOutcome::PilotRun { ref run } if run.success
        ));

        let second = quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let run = match second {
            QuorumCliOutcome::PilotRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(run.success);
        assert!(run.executed_steps.is_empty());
        assert_eq!(
            run.skipped_steps,
            vec![
                "run_rounds".to_string(),
                "run_synthesis".to_string(),
                "accept_synthesis".to_string(),
                "submit_drafts".to_string(),
            ]
        );
        assert_eq!(run.failed_step, None);
        assert_eq!(run.error, None);
        let staged = DraftStore::new(drafts.path()).list(mimir_librarian::DraftState::Pending)?;
        assert_eq!(staged.len(), 1);
        Ok(())
    }

    #[test]
    fn quorum_pilot_status_fails_when_required_proposed_drafts_are_missing(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_args(
            quorum.path(),
            out.path(),
            drafts.path(),
            "independent",
            "codex",
            vec!["--require-proposed-drafts".to_string(), "1".to_string()],
        )?;
        quorum_from_args(
            &recorded_fixture_synthesize_without_drafts_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-status".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let status = match outcome {
            QuorumCliOutcome::PilotStatus { status } => status,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(status.overall_status, QuorumPilotGateStatus::Failed);
        assert!(status.gates.iter().any(|gate| {
            gate.name == "accept_synthesis"
                && gate.status == QuorumPilotGateStatus::Failed
                && gate.detail.contains("requires at least 1")
        }));

        let err = match quorum_from_args(
            &[
                "pilot-review".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
                "--reviewer".to_string(),
                "operator:AlainDor".to_string(),
                "--decision".to_string(),
                "pass".to_string(),
                "--summary".to_string(),
                "Insufficient proposed drafts cannot be certified.".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        ) {
            Ok(outcome) => {
                return Err(format!("required-draft pass must reject, got {outcome:?}").into())
            }
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("complete pilot-status"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[test]
    fn quorum_pilot_run_fails_when_required_proposed_drafts_are_missing(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-required-drafts-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Required-drafts Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-required-drafts-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Required-drafts Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-required-drafts-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Synthesize without drafts despite the pilot requirement.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"The model produced a valid result shape.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Round output was recorded\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Synthesis accepted the evidence\"}],\"proposed_memory_drafts\":[]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_args(
            quorum.path(),
            out.path(),
            drafts.path(),
            "independent",
            "codex",
            vec![
                "--adapter-binary".to_string(),
                format!("claude={}", claude_shim.display()),
                "--adapter-binary".to_string(),
                format!("codex={}", codex_shim.display()),
                "--synthesizer-binary".to_string(),
                synth_shim.display().to_string(),
                "--require-proposed-drafts".to_string(),
                "1".to_string(),
            ],
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let run = match outcome {
            QuorumCliOutcome::PilotRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(!run.success);
        assert_eq!(run.failed_step.as_deref(), Some("accept_synthesis"));
        assert!(run
            .error
            .as_deref()
            .is_some_and(|message| message.contains("accept_synthesis")));
        assert_eq!(
            run.executed_steps,
            vec![
                "run_rounds".to_string(),
                "run_synthesis".to_string(),
                "accept_synthesis".to_string(),
            ]
        );
        let staged = DraftStore::new(drafts.path()).list(mimir_librarian::DraftState::Pending)?;
        assert!(staged.is_empty());
        Ok(())
    }

    #[test]
    fn quorum_pilot_review_rejects_pass_for_incomplete_manifest(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan(quorum.path(), out.path(), drafts.path(), "independent")?;

        let err = match quorum_from_args(
            &[
                "pilot-review".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
                "--reviewer".to_string(),
                "operator:AlainDor".to_string(),
                "--decision".to_string(),
                "pass".to_string(),
                "--summary".to_string(),
                "Incomplete pilots cannot be certified.".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        ) {
            Ok(outcome) => {
                return Err(format!("incomplete pass must reject, got {outcome:?}").into())
            }
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("complete pilot-status"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[test]
    fn quorum_pilot_review_writes_certification_artifact() -> Result<(), Box<dyn std::error::Error>>
    {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-review-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot review Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-review-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot review Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-review-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Certify the manifest after reviewing recorded gates.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"All gates completed before review.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Review sees completed artifacts\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Certification is separate from participation\"}],\"proposed_memory_drafts\":[\"Live pilot certification should be recorded as a non-participant review artifact before service adapter design.\"]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_runtime_binaries(
            quorum.path(),
            out.path(),
            drafts.path(),
            &claude_shim,
            &codex_shim,
            &synth_shim,
        )?;
        match quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )? {
            QuorumCliOutcome::PilotRun { run } => assert!(run.success),
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }

        let outcome = quorum_from_args(
            &[
                "pilot-review".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
                "--reviewer".to_string(),
                "operator:AlainDor".to_string(),
                "--decision".to_string(),
                "pass".to_string(),
                "--summary".to_string(),
                "Recorded gates satisfy the local pilot certification criteria.".to_string(),
                "--finding".to_string(),
                "info:All manifest gates completed before certification.".to_string(),
                "--next-action".to_string(),
                "Use the certified artifact to shape service adapter contracts.".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let review = match outcome {
            QuorumCliOutcome::PilotReview { review } => review,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(review.episode_id, "qr-cli-007");
        assert_eq!(review.decision, QuorumPilotReviewDecision::Pass);
        assert!(review.status_at_review.complete);
        assert_eq!(review.findings.len(), 1);
        assert_eq!(review.next_actions.len(), 1);
        assert!(std::path::Path::new(&review.review_path).exists());
        let bytes = std::fs::read(&review.review_path)?;
        let artifact: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(artifact["reviewer"], "operator:AlainDor");
        assert_eq!(artifact["decision"], "pass");
        assert_eq!(artifact["status_at_review"]["overall_status"], "complete");
        Ok(())
    }

    #[test]
    fn quorum_pilot_summary_reports_certified_pilot_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let drafts = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-pilot-summary-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot summary Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-pilot-summary-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Pilot summary Codex response.' > \"$3\"\n",
        )?;
        let (_synth_dir, synth_shim) = make_quorum_shim(
            "codex-pilot-summary-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Summarize certified pilots for the operator.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"All gates completed before review.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Summary sees completed artifacts\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Summary preserves draft count\"}],\"proposed_memory_drafts\":[\"Certified pilot summaries should report accepted results, staged drafts, and review decisions.\"]}' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan = create_test_pilot_plan_with_runtime_binaries(
            quorum.path(),
            out.path(),
            drafts.path(),
            &claude_shim,
            &codex_shim,
            &synth_shim,
        )?;
        match quorum_from_args(
            &[
                "pilot-run".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )? {
            QuorumCliOutcome::PilotRun { run } => assert!(run.success),
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        quorum_from_args(
            &[
                "pilot-review".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
                "--reviewer".to_string(),
                "operator:AlainDor".to_string(),
                "--decision".to_string(),
                "pass".to_string(),
                "--summary".to_string(),
                "Recorded gates satisfy the local pilot certification criteria.".to_string(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;

        let outcome = quorum_from_args(
            &[
                "pilot-summary".to_string(),
                "--manifest-file".to_string(),
                plan.manifest_path.clone(),
            ],
            SystemTime::UNIX_EPOCH,
        )?;
        let summary = match outcome {
            QuorumCliOutcome::PilotSummary { summary } => summary,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(summary.complete);
        assert_eq!(summary.overall_status, QuorumPilotGateStatus::Complete);
        assert_eq!(summary.proposed_memory_drafts, 1);
        assert_eq!(summary.submitted_drafts, 1);
        assert_eq!(summary.result_status, "present");
        assert!(summary
            .result_path
            .as_deref()
            .is_some_and(|path| std::path::Path::new(path).exists()));
        assert_eq!(summary.review_status, "present");
        assert_eq!(
            summary.review_decision,
            Some(QuorumPilotReviewDecision::Pass)
        );
        assert_eq!(summary.next_action, "none");
        assert_eq!(summary.gates.len(), 4);
        Ok(())
    }

    #[test]
    fn quorum_adapter_plan_materializes_native_claude_contract(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan_args = vec![
            "adapter-plan".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
        ];

        let outcome = quorum_from_args(&plan_args, SystemTime::UNIX_EPOCH)?;
        let plan = match outcome {
            QuorumCliOutcome::AdapterPlan { plan } => plan,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(plan.adapter, "claude");
        assert_eq!(plan.argv, vec!["claude".to_string(), "-p".to_string()]);
        assert_eq!(plan.stdin_path, plan.prompt_path);
        assert_eq!(
            plan.stdout_path.as_deref(),
            Some(plan.response_path.as_str())
        );
        assert!(std::path::Path::new(&plan.request_path).exists());
        let prompt = std::fs::read_to_string(&plan.prompt_path)?;
        assert!(prompt.contains("Should recorded quorum fixtures preserve the draft boundary?"));
        assert!(prompt.contains("persona: architect"));

        std::fs::write(&plan.response_path, "Claude fixture response.")?;
        let append_args = plan
            .append_output_argv
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>();
        quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].response, "Claude fixture response.");
        Ok(())
    }

    #[test]
    fn quorum_adapter_plan_materializes_native_codex_contract(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let plan_args = vec![
            "adapter-plan".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "codex".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
        ];

        let outcome = quorum_from_args(&plan_args, SystemTime::UNIX_EPOCH)?;
        let plan = match outcome {
            QuorumCliOutcome::AdapterPlan { plan } => plan,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(plan.adapter, "codex");
        assert_eq!(
            plan.argv,
            vec![
                "codex".to_string(),
                "exec".to_string(),
                "--output-last-message".to_string(),
                plan.response_path.clone(),
                "-".to_string()
            ]
        );
        assert_eq!(plan.stdin_path, plan.prompt_path);
        assert_eq!(plan.stdout_path, None);
        let prompt = std::fs::read_to_string(&plan.prompt_path)?;
        assert!(prompt.contains("persona: implementation_engineer"));

        std::fs::write(&plan.response_path, "Codex fixture response.")?;
        let append_args = plan
            .append_output_argv
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>();
        quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].response, "Codex fixture response.");
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_executes_claude_plan_without_appending_output(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Claude run response.'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];

        let outcome = quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)?;
        let run = match outcome {
            QuorumCliOutcome::AdapterRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(run.status.success);
        assert!(!run.status.timed_out);
        assert_eq!(run.status.exit_code, Some(0));
        assert!(std::path::Path::new(&run.status.status_path).exists());
        assert_eq!(
            std::fs::read_to_string(&run.status.response_path)?,
            "Claude run response."
        );
        assert!(
            QuorumStore::new(quorum.path())
                .load_round_outputs("qr-cli-007", QuorumRound::Independent)?
                .is_empty(),
            "adapter-run must not append participant outputs directly",
        );

        let append_args = run
            .status
            .append_output_argv
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>();
        quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].response, "Claude run response.");
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_executes_codex_plan_without_appending_output(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "codex-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Codex run response.' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "codex".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];

        let outcome = quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)?;
        let run = match outcome {
            QuorumCliOutcome::AdapterRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(run.status.success);
        assert_eq!(
            std::fs::read_to_string(&run.status.response_path)?,
            "Codex run response."
        );
        assert!(
            QuorumStore::new(quorum.path())
                .load_round_outputs("qr-cli-007", QuorumRound::Independent)?
                .is_empty(),
            "adapter-run must not append participant outputs directly",
        );

        let append_args = run
            .status
            .append_output_argv
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>();
        quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].response, "Codex run response.");
        Ok(())
    }

    #[test]
    fn quorum_append_status_output_records_successful_adapter_run_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Claude status response.'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::AdapterRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        let append_args = vec![
            "append-status-output".to_string(),
            "--status-file".to_string(),
            run.status.status_path.clone(),
        ];
        let outcome = quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        match outcome {
            QuorumCliOutcome::StatusOutputsAppended {
                status_path,
                appended,
                outputs,
            } => {
                assert_eq!(status_path, run.status.status_path);
                assert_eq!(appended, 1);
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].participant_id, "claude");
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].response, "Claude status response.");
        Ok(())
    }

    #[test]
    fn quorum_append_status_output_rejects_failed_adapter_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Failed response.'\nexit 2\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--participant-id".to_string(),
            "claude".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::AdapterRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };
        assert!(!run.status.success);

        let append_args = vec![
            "append-status-output".to_string(),
            "--status-file".to_string(),
            run.status.status_path,
        ];
        let err = match quorum_from_args(&append_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => {
                return Err(format!("failed status must not append, got {outcome:?}").into());
            }
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("not successful"),
            "unexpected error: {err}"
        );
        assert!(
            QuorumStore::new(quorum.path())
                .load_round_outputs("qr-cli-007", QuorumRound::Independent)?
                .is_empty(),
            "failed status must not append participant output",
        );
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_round_executes_independent_participants_without_appending(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Round Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Round Codex response.' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run-round".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--adapter-binary".to_string(),
            format!("claude={}", claude_shim.display()),
            "--adapter-binary".to_string(),
            format!("codex={}", codex_shim.display()),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];

        let outcome = quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)?;
        let round_run = match outcome {
            QuorumCliOutcome::AdapterRoundRun { round_run } => round_run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(round_run.success);
        assert_eq!(round_run.completed, 2);
        assert_eq!(round_run.failed, 0);
        assert_eq!(round_run.statuses.len(), 2);
        assert!(std::path::Path::new(&round_run.status_path).exists());
        assert!(
            QuorumStore::new(quorum.path())
                .load_round_outputs("qr-cli-007", QuorumRound::Independent)?
                .is_empty(),
            "adapter-run-round must not append participant outputs directly",
        );

        for status in &round_run.statuses {
            let append_args = status
                .append_output_argv
                .iter()
                .skip(2)
                .cloned()
                .collect::<Vec<_>>();
            quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        }
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        let responses = outputs
            .iter()
            .map(|output| output.response.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            responses,
            vec!["Round Claude response.", "Round Codex response."]
        );
        Ok(())
    }

    #[test]
    fn quorum_append_status_output_records_successful_round_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Round status Claude response.'\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Round status Codex response.' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run-round".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--round".to_string(),
            "independent".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--adapter-binary".to_string(),
            format!("claude={}", claude_shim.display()),
            "--adapter-binary".to_string(),
            format!("codex={}", codex_shim.display()),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let round_run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::AdapterRoundRun { round_run } => round_run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        let append_args = vec![
            "append-status-output".to_string(),
            "--status-file".to_string(),
            round_run.status_path.clone(),
        ];
        let outcome = quorum_from_args(&append_args, SystemTime::UNIX_EPOCH)?;
        match outcome {
            QuorumCliOutcome::StatusOutputsAppended {
                status_path,
                appended,
                outputs,
            } => {
                assert_eq!(status_path, round_run.status_path);
                assert_eq!(appended, 2);
                let participants = outputs
                    .iter()
                    .map(|output| output.participant_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(participants, vec!["claude", "codex"]);
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        let outputs = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        let responses = outputs
            .iter()
            .map(|output| output.response.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            responses,
            vec![
                "Round status Claude response.",
                "Round status Codex response."
            ]
        );
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_rounds_sequences_critique_after_append_gate(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\nprompt=$(cat)\nif printf '%s' \"$prompt\" | grep -q 'output_id: out-independent-claude'; then printf 'Claude critique saw prior outputs.'; else printf 'Claude independent response.'; fi\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-shim",
            "#!/bin/sh\nprompt=$(cat)\nif printf '%s' \"$prompt\" | grep -q 'output_id: out-independent-claude'; then printf 'Codex critique saw prior outputs.' > \"$3\"; else printf 'Codex independent response.' > \"$3\"; fi\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run-rounds".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--through-round".to_string(),
            "critique".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--adapter-binary".to_string(),
            format!("claude={}", claude_shim.display()),
            "--adapter-binary".to_string(),
            format!("codex={}", codex_shim.display()),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];

        let outcome = quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)?;
        let rounds_run = match outcome {
            QuorumCliOutcome::AdapterRoundsRun { rounds_run } => rounds_run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(rounds_run.success);
        assert_eq!(rounds_run.through_round, "critique");
        assert_eq!(rounds_run.rounds.len(), 2);
        assert_eq!(rounds_run.appended.len(), 4);
        let independent = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Independent)?;
        let critique = QuorumStore::new(quorum.path())
            .load_round_outputs("qr-cli-007", QuorumRound::Critique)?;
        assert_eq!(independent.len(), 2);
        assert_eq!(critique.len(), 2);
        assert!(critique.iter().all(|output| output.visible_prior_output_ids
            == vec![
                "out-independent-claude".to_string(),
                "out-independent-codex".to_string()
            ]));
        assert!(critique
            .iter()
            .all(|output| output.prompt.contains("Claude independent response.")));
        let critique_responses = critique
            .iter()
            .map(|output| output.response.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            critique_responses,
            vec![
                "Claude critique saw prior outputs.",
                "Codex critique saw prior outputs."
            ]
        );
        Ok(())
    }

    #[test]
    fn quorum_synthesize_plan_materializes_native_codex_contract_from_outputs(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            (
                "claude",
                "Claude fixture output supports the draft boundary.",
            ),
            (
                "codex",
                "Codex fixture output preserves explicit acceptance.",
            ),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }

        let plan_args = vec![
            "synthesize-plan".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
        ];
        let plan = match quorum_from_args(&plan_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::SynthesisPlan { plan } => plan,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert_eq!(plan.adapter, "codex");
        assert_eq!(
            plan.argv,
            vec![
                "codex".to_string(),
                "exec".to_string(),
                "--output-last-message".to_string(),
                plan.result_path.clone(),
                "-".to_string()
            ]
        );
        assert_eq!(plan.stdin_path, plan.prompt_path);
        assert_eq!(plan.stdout_path, None);
        assert!(std::path::Path::new(&plan.transcript_path).exists());
        let prompt = std::fs::read_to_string(&plan.prompt_path)?;
        assert!(prompt.contains("Should recorded quorum fixtures preserve the draft boundary?"));
        assert!(prompt.contains("Claude fixture output supports the draft boundary."));
        assert!(prompt.contains("\"proposed_memory_drafts\""));
        assert_eq!(plan.accept_synthesis_argv[2], "accept-synthesis");
        Ok(())
    }

    #[test]
    fn quorum_synthesize_run_executes_model_without_saving_result(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Keep synthesis explicit.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"Both outputs support the draft boundary.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Audit trail remains intact\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Acceptance remains explicit\"}],\"proposed_memory_drafts\":[\"Synthesis adapters must emit proposed quorum results for explicit acceptance.\"]}'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            ("claude", "Claude supports explicit synthesis acceptance."),
            ("codex", "Codex supports synthesis as evidence."),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }

        let run_args = vec![
            "synthesize-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::SynthesisRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(run.status.success);
        assert!(run.status.result_valid);
        assert_eq!(run.status.validation_error, None);
        assert!(!run.status.timed_out);
        assert!(std::path::Path::new(&run.status.result_path).exists());
        assert!(
            QuorumStore::new(quorum.path())
                .load_result("qr-cli-007")
                .is_err(),
            "synthesize-run must not save a quorum result directly",
        );
        assert_eq!(run.status.accept_synthesis_argv[2], "accept-synthesis");
        Ok(())
    }

    #[test]
    fn quorum_synthesize_run_reports_invalid_result_in_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-invalid-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Incomplete synthesis.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.8,\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.8,\"rationale\":\"Only one vote\"}]}'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            ("claude", "Claude supports explicit synthesis acceptance."),
            ("codex", "Codex supports synthesis as evidence."),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }

        let run_args = vec![
            "synthesize-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::SynthesisRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };

        assert!(!run.status.success);
        assert_eq!(
            run.status.process_status,
            Some(QuorumProcessStatus::Succeeded)
        );
        assert!(!run.status.result_valid);
        assert!(run
            .status
            .validation_error
            .as_deref()
            .is_some_and(|message| message.contains("participant vote")));
        assert!(std::path::Path::new(&run.status.status_path).exists());
        assert!(
            QuorumStore::new(quorum.path())
                .load_result("qr-cli-007")
                .is_err(),
            "invalid synthesize-run output must not save a quorum result",
        );
        Ok(())
    }

    #[test]
    fn quorum_accept_synthesis_records_result_through_existing_synthesize_path(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let result_file = tempfile::NamedTempFile::new()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            ("claude", "Claude wants dissent preserved."),
            ("codex", "Codex wants provenance attached."),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }
        std::fs::write(
            result_file.path(),
            r#"{
  "schema_version": 1,
  "episode_id": "qr-cli-007",
  "recommendation": "Accept synthesis only through the recorded result path.",
  "decision_status": "recommend",
  "consensus_level": "strong_majority",
  "confidence": 0.88,
  "supporting_points": ["Both participants require provenance."],
  "dissenting_points": ["No dissent, but acceptance remains explicit."],
  "unresolved_questions": [],
  "evidence_used": ["operator://fixture"],
  "participant_votes": [
    {"participant_id":"claude","vote":"agree","confidence":0.9,"rationale":"Dissent is visible"},
    {"participant_id":"codex","vote":"agree","confidence":0.86,"rationale":"Provenance is attached"}
  ],
  "proposed_memory_drafts": [
    "Accepted quorum synthesis must preserve participant evidence and enter drafts separately."
  ]
}"#,
        )?;

        let accept_args = vec![
            "accept-synthesis".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--result-file".to_string(),
            result_file.path().display().to_string(),
        ];
        let outcome = quorum_from_args(&accept_args, SystemTime::UNIX_EPOCH)?;
        match outcome {
            QuorumCliOutcome::ResultSaved { episode_id, .. } => {
                assert_eq!(episode_id, "qr-cli-007");
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        let result = QuorumStore::new(quorum.path()).load_result("qr-cli-007")?;
        assert_eq!(result.participant_votes.len(), 2);
        assert!(result
            .evidence_used
            .iter()
            .any(|item| item == "operator://fixture"));
        assert!(result
            .evidence_used
            .iter()
            .any(|item| item.contains("out-independent-codex")));
        Ok(())
    }

    #[test]
    fn quorum_accept_synthesis_records_result_from_successful_status(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-status-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Accept status-backed synthesis.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.84,\"supporting_points\":[\"Status artifacts preserve replayability.\"],\"dissenting_points\":[],\"unresolved_questions\":[],\"evidence_used\":[],\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.86,\"rationale\":\"Status is valid\"},{\"participant_id\":\"codex\",\"vote\":\"agree\",\"confidence\":0.82,\"rationale\":\"Acceptance remains explicit\"}],\"proposed_memory_drafts\":[\"Status-backed synthesis acceptance must still enter through explicit result recording.\"]}'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            (
                "claude",
                "Claude supports status-backed synthesis acceptance.",
            ),
            ("codex", "Codex supports replayable synthesis status."),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }
        let run_args = vec![
            "synthesize-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::SynthesisRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };
        assert!(run.status.success);

        let accept_args = vec![
            "accept-synthesis".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--status-file".to_string(),
            run.status.status_path.clone(),
        ];
        let outcome = quorum_from_args(&accept_args, SystemTime::UNIX_EPOCH)?;
        match outcome {
            QuorumCliOutcome::ResultSaved { episode_id, .. } => {
                assert_eq!(episode_id, "qr-cli-007");
            }
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        }
        let result = QuorumStore::new(quorum.path()).load_result("qr-cli-007")?;
        assert_eq!(result.participant_votes.len(), 2);
        assert!(result
            .evidence_used
            .iter()
            .any(|item| item.contains("out-independent-codex")));
        Ok(())
    }

    #[test]
    fn quorum_accept_synthesis_rejects_failed_status() -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_shim_dir, shim) = make_quorum_shim(
            "claude-invalid-status-synth-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"schema_version\":1,\"episode_id\":\"qr-cli-007\",\"recommendation\":\"Incomplete synthesis.\",\"decision_status\":\"recommend\",\"consensus_level\":\"strong_majority\",\"confidence\":0.8,\"participant_votes\":[{\"participant_id\":\"claude\",\"vote\":\"agree\",\"confidence\":0.8,\"rationale\":\"Only one vote\"}]}'\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        for (participant_id, response) in [
            ("claude", "Claude supports explicit synthesis acceptance."),
            ("codex", "Codex supports synthesis as evidence."),
        ] {
            quorum_from_args(
                &recorded_fixture_append_args(quorum.path(), participant_id, response),
                SystemTime::UNIX_EPOCH,
            )?;
        }
        let run_args = vec![
            "synthesize-run".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--adapter".to_string(),
            "claude".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--binary".to_string(),
            shim.display().to_string(),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];
        let run = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH)? {
            QuorumCliOutcome::SynthesisRun { run } => run,
            other => return Err(format!("unexpected quorum outcome: {other:?}").into()),
        };
        assert!(!run.status.success);

        let accept_args = vec![
            "accept-synthesis".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--status-file".to_string(),
            run.status.status_path,
        ];
        let err = match quorum_from_args(&accept_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => return Err(format!("failed status must reject, got {outcome:?}").into()),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("not successful"),
            "unexpected error: {err}"
        );
        assert!(QuorumStore::new(quorum.path())
            .load_result("qr-cli-007")
            .is_err());
        Ok(())
    }

    #[test]
    fn quorum_accept_synthesis_rejects_missing_participant_votes(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let result_file = tempfile::NamedTempFile::new()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        std::fs::write(
            result_file.path(),
            r#"{
  "schema_version": 1,
  "episode_id": "qr-cli-007",
  "recommendation": "Incomplete synthesis must not be accepted.",
  "decision_status": "recommend",
  "consensus_level": "strong_majority",
  "confidence": 0.8,
  "participant_votes": [
    {"participant_id":"claude","vote":"agree","confidence":0.8,"rationale":"Codex vote is missing"}
  ]
}"#,
        )?;

        let accept_args = vec![
            "accept-synthesis".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--result-file".to_string(),
            result_file.path().display().to_string(),
        ];
        let err = match quorum_from_args(&accept_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => return Err(format!("missing vote must reject, got {outcome:?}").into()),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("participant vote"),
            "unexpected error: {err}"
        );
        assert!(QuorumStore::new(quorum.path())
            .load_result("qr-cli-007")
            .is_err());
        Ok(())
    }

    #[test]
    fn quorum_accept_synthesis_rejects_duplicate_participant_votes(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let result_file = tempfile::NamedTempFile::new()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        std::fs::write(
            result_file.path(),
            r#"{
  "schema_version": 1,
  "episode_id": "qr-cli-007",
  "recommendation": "Duplicate votes must not be accepted.",
  "decision_status": "recommend",
  "consensus_level": "strong_majority",
  "confidence": 0.8,
  "participant_votes": [
    {"participant_id":"claude","vote":"agree","confidence":0.8,"rationale":"First vote"},
    {"participant_id":"claude","vote":"agree","confidence":0.7,"rationale":"Duplicate vote"},
    {"participant_id":"codex","vote":"agree","confidence":0.8,"rationale":"Codex vote"}
  ]
}"#,
        )?;

        let accept_args = vec![
            "accept-synthesis".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--result-file".to_string(),
            result_file.path().display().to_string(),
        ];
        let err = match quorum_from_args(&accept_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => {
                return Err(format!("duplicate vote must reject, got {outcome:?}").into());
            }
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("duplicate participant vote"),
            "unexpected error: {err}"
        );
        assert!(QuorumStore::new(quorum.path())
            .load_result("qr-cli-007")
            .is_err());
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_rounds_rejects_failed_round_without_appending(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        let (_claude_dir, claude_shim) = make_quorum_shim(
            "claude-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Failed independent response.'\nexit 2\n",
        )?;
        let (_codex_dir, codex_shim) = make_quorum_shim(
            "codex-shim",
            "#!/bin/sh\ncat >/dev/null\nprintf 'Codex independent response.' > \"$3\"\n",
        )?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run-rounds".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--through-round".to_string(),
            "independent".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
            "--adapter-binary".to_string(),
            format!("claude={}", claude_shim.display()),
            "--adapter-binary".to_string(),
            format!("codex={}", codex_shim.display()),
            "--timeout-secs".to_string(),
            "5".to_string(),
        ];

        let err = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => {
                return Err(format!("failed round must not append, got {outcome:?}").into());
            }
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("not successful"),
            "unexpected error: {err}"
        );
        assert!(
            QuorumStore::new(quorum.path())
                .load_round_outputs("qr-cli-007", QuorumRound::Independent)?
                .is_empty(),
            "failed round must not append participant outputs",
        );
        Ok(())
    }

    #[test]
    fn quorum_adapter_run_round_respects_critique_visibility_gate(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let quorum = tempfile::tempdir()?;
        let out = tempfile::tempdir()?;
        quorum_from_args(
            &recorded_fixture_create_args(quorum.path()),
            SystemTime::UNIX_EPOCH,
        )?;
        let run_args = vec![
            "adapter-run-round".to_string(),
            "--quorum-dir".to_string(),
            quorum.path().display().to_string(),
            "--episode-id".to_string(),
            "qr-cli-007".to_string(),
            "--round".to_string(),
            "critique".to_string(),
            "--out-dir".to_string(),
            out.path().display().to_string(),
        ];

        let err = match quorum_from_args(&run_args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => {
                return Err(format!(
                    "critique round must wait for independent outputs, got {outcome:?}"
                )
                .into());
            }
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("round is incomplete"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[test]
    fn sweep_writes_pending_drafts_for_text_files() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let source = tempfile::tempdir()?;
        std::fs::write(source.path().join("codex.md"), "Codex remembers Mimir.\n")?;
        std::fs::create_dir(source.path().join("nested"))?;
        std::fs::write(
            source.path().join("nested").join("claude.txt"),
            "Claude remembers the quorum boundary.\n",
        )?;
        std::fs::write(source.path().join("ignored.bin"), "not a memory file")?;

        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--path".to_string(),
            source.path().display().to_string(),
            "--source-surface".to_string(),
            "codex-memory".to_string(),
            "--project".to_string(),
            "buildepicshit/Mimir".to_string(),
            "--operator".to_string(),
            "AlainDor".to_string(),
            "--tag".to_string(),
            "sweep-test".to_string(),
        ];

        let outcome = match sweep_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("sweep failed: {err:?}").into()),
        };

        assert_eq!(outcome.submitted, 2);
        assert_eq!(outcome.skipped_empty, 0);
        assert_eq!(outcome.drafts.len(), 2);

        let store = DraftStore::new(drafts.path());
        let staged = store.list(mimir_librarian::DraftState::Pending)?;
        assert_eq!(staged.len(), 2);
        assert!(staged
            .iter()
            .all(|draft| draft.metadata().source_surface == DraftSourceSurface::CodexMemory));
        assert!(staged
            .iter()
            .all(|draft| draft.metadata().source_agent.as_deref() == Some("codex")));
        assert!(staged
            .iter()
            .all(|draft| draft.metadata().operator.as_deref() == Some("AlainDor")));
        Ok(())
    }

    #[test]
    fn sweep_from_args_skips_empty_files() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let source = tempfile::NamedTempFile::new()?;
        std::fs::write(source.path(), " \n\t")?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--path".to_string(),
            source.path().display().to_string(),
            "--source-surface".to_string(),
            "claude-memory".to_string(),
        ];

        let outcome = match sweep_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("sweep failed: {err:?}").into()),
        };

        assert_eq!(outcome.submitted, 0);
        assert_eq!(outcome.skipped_empty, 1);
        assert!(outcome.drafts.is_empty());
        Ok(())
    }

    #[test]
    fn sweep_from_args_rejects_missing_path() {
        let args = vec!["--source-surface".to_string(), "codex-memory".to_string()];
        match sweep_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => assert!(message.contains("--path is required")),
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn sweep_from_args_rejects_missing_source_surface() {
        let args = vec!["--path".to_string(), ".".to_string()];
        match sweep_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--source-surface is required"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn sweep_from_args_rejects_non_sweep_source_surface() {
        let args = vec![
            "--path".to_string(),
            ".".to_string(),
            "--source-surface".to_string(),
            "consensus-quorum".to_string(),
        ];
        match sweep_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => assert!(message.contains("not valid for sweep")),
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn run_from_args_reports_empty_store() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.processor, "retrying_llm");
        assert_eq!(outcome.summary.pending_seen, 0);
        assert_eq!(outcome.summary.claimed, 0);
        assert_eq!(outcome.summary.deferred, 0);
        Ok(())
    }

    #[test]
    fn run_from_args_accepts_review_conflicts_flag() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
            "--review-conflicts".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.processor, "retrying_llm");
        assert_eq!(outcome.summary.pending_seen, 0);
        assert_eq!(outcome.summary.quarantined, 0);
        Ok(())
    }

    #[test]
    fn run_from_args_accepts_dedup_window_flag() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
            "--dedup-valid-at-window-secs".to_string(),
            "0".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.processor, "retrying_llm");
        assert_eq!(outcome.summary.pending_seen, 0);
        Ok(())
    }

    #[test]
    fn run_from_args_rejects_invalid_dedup_window() {
        let args = vec![
            "--dedup-valid-at-window-secs".to_string(),
            "soon".to_string(),
        ];
        match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--dedup-valid-at-window-secs must be an integer"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn watch_from_args_runs_bounded_iteration() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
            "--iterations".to_string(),
            "1".to_string(),
        ];
        let mut outcomes = Vec::new();

        let outcome = match watch_from_args(&args, |run| outcomes.push(run.clone())) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("watch failed: {err:?}").into()),
        };

        assert_eq!(outcome.iterations, 1);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].summary.pending_seen, 0);
        Ok(())
    }

    #[test]
    fn watch_from_args_supports_defer_mode() -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let store = DraftStore::new(drafts.path());
        let draft = Draft::with_metadata(
            "watch should return this draft to pending".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--defer".to_string(),
            "--iterations".to_string(),
            "1".to_string(),
        ];
        let mut outcomes = Vec::new();

        let outcome = match watch_from_args(&args, |run| outcomes.push(run.clone())) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("watch failed: {err:?}").into()),
        };

        assert_eq!(outcome.iterations, 1);
        assert_eq!(outcomes[0].summary.deferred, 1);
        assert_eq!(store.list(mimir_librarian::DraftState::Pending)?.len(), 1);
        Ok(())
    }

    #[test]
    fn watch_from_args_rejects_invalid_poll_secs() {
        let args = vec!["--poll-secs".to_string(), "often".to_string()];
        match watch_from_args(&args, |_| {}) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--poll-secs must be an integer"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn run_from_args_defers_pending_without_losing_drafts() -> Result<(), Box<dyn std::error::Error>>
    {
        let drafts = tempfile::tempdir()?;
        let store = DraftStore::new(drafts.path());
        let draft = Draft::with_metadata(
            "The real processor lands after the run skeleton.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--defer".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.summary.pending_seen, 1);
        assert_eq!(outcome.summary.claimed, 1);
        assert_eq!(outcome.summary.deferred, 1);
        assert_eq!(store.list(mimir_librarian::DraftState::Pending)?.len(), 1);
        assert_eq!(
            store.list(mimir_librarian::DraftState::Processing)?.len(),
            0
        );
        Ok(())
    }

    #[test]
    fn run_from_args_recovers_stale_processing_before_deferring(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let store = DraftStore::new(drafts.path());
        let draft = Draft::with_metadata(
            "A crashed run should not leave this stuck forever.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        store.transition(
            draft.id(),
            mimir_librarian::DraftState::Pending,
            mimir_librarian::DraftState::Processing,
        )?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--stale-processing-secs".to_string(),
            "0".to_string(),
            "--defer".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::now() + Duration::from_secs(60)) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.summary.recovered_processing, 1);
        assert_eq!(outcome.summary.pending_seen, 1);
        assert_eq!(outcome.summary.deferred, 1);
        assert_eq!(store.list(mimir_librarian::DraftState::Pending)?.len(), 1);
        assert_eq!(
            store.list(mimir_librarian::DraftState::Processing)?.len(),
            0
        );
        Ok(())
    }

    #[test]
    fn run_from_args_archive_raw_commits_pending_without_llm(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let store = DraftStore::new(drafts.path());
        let draft = Draft::with_metadata(
            "Archive this raw checkpoint quickly.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
            "--archive-raw".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.processor, "archive_raw");
        assert_eq!(outcome.summary.pending_seen, 1);
        assert_eq!(outcome.summary.claimed, 1);
        assert_eq!(outcome.summary.accepted, 1);
        assert_eq!(store.list(mimir_librarian::DraftState::Accepted)?.len(), 1);
        let reopened = mimir_core::Store::open(&workspace_log)?;
        assert_eq!(reopened.pipeline().semantic_records().len(), 6);
        Ok(())
    }

    #[test]
    fn run_from_args_archive_raw_recovers_stale_processing(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let drafts = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let workspace_log = workspace.path().join("canonical.log");
        let store = DraftStore::new(drafts.path());
        let draft = Draft::with_metadata(
            "A crashed archive run should drain on the next pass.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        store.transition(
            draft.id(),
            mimir_librarian::DraftState::Pending,
            mimir_librarian::DraftState::Processing,
        )?;
        let args = vec![
            "--drafts-dir".to_string(),
            drafts.path().display().to_string(),
            "--workspace".to_string(),
            workspace_log.display().to_string(),
            "--stale-processing-secs".to_string(),
            "0".to_string(),
            "--archive-raw".to_string(),
        ];

        let outcome = match run_from_args(&args, SystemTime::now() + Duration::from_secs(60)) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("run failed: {err:?}").into()),
        };

        assert_eq!(outcome.processor, "archive_raw");
        assert_eq!(outcome.summary.recovered_processing, 1);
        assert_eq!(outcome.summary.pending_seen, 1);
        assert_eq!(outcome.summary.accepted, 1);
        assert_eq!(
            store.list(mimir_librarian::DraftState::Processing)?.len(),
            0
        );
        let reopened = mimir_core::Store::open(&workspace_log)?;
        assert_eq!(reopened.pipeline().semantic_records().len(), 6);
        Ok(())
    }

    #[test]
    fn run_from_args_rejects_archive_raw_with_defer() {
        let args = vec!["--archive-raw".to_string(), "--defer".to_string()];
        match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--archive-raw cannot be combined"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn run_from_args_rejects_bad_stale_processing_secs() {
        let args = vec![
            "--stale-processing-secs".to_string(),
            "not-a-number".to_string(),
        ];
        match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--stale-processing-secs must be an integer"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn run_from_args_rejects_bad_max_retries() {
        let args = vec!["--max-retries".to_string(), "not-a-number".to_string()];
        match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--max-retries must be an integer"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }

    #[test]
    fn run_from_args_rejects_bad_llm_timeout_secs() {
        let args = vec!["--llm-timeout-secs".to_string(), "not-a-number".to_string()];
        match run_from_args(&args, SystemTime::UNIX_EPOCH) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("--llm-timeout-secs must be an integer"));
            }
            other => assert!(
                matches!(other, Err(CliError::Usage(_))),
                "expected usage error, got {other:?}"
            ),
        }
    }
}
