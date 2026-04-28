//! Prose memory drafts — the librarian's input unit.
//!
//! A draft is a piece of prose an agent, adapter, or operator wrote,
//! intended to be captured as durable Mimir memory after librarian
//! validation. Drafts are untrusted. They carry scope-model metadata
//! and provenance, then live on the filesystem under a state-directory
//! flow so the librarian's processing is crash-safe: a draft's
//! directory tells you its lifecycle state, and state transitions are
//! atomic renames. Processing claim markers record when a draft entered
//! `processing/` so stale recovery is based on claim age, not original
//! submission age.
//!
//! ```text
//! drafts/pending/<id>.json       ─ waiting for a librarian run
//! drafts/processing/<id>.json    ─ currently being structured
//! drafts/accepted/<id>.json    ─ successfully written to the log
//! drafts/skipped/<id>.json     ─ intentionally ignored / duplicate
//! drafts/failed/<id>.json        ─ retry budget exhausted
//! drafts/quarantined/<id>.json ─ unsafe or unresolved, review only
//! ```
//!
//! Draft IDs are content-addressed over the raw text plus stable
//! provenance identity fields. Identical sweeps of the same source
//! produce the same ID, but identical text from different agents or
//! files remains distinct so provenance is never collapsed away.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::LibrarianError;

/// Current on-disk draft schema version.
pub const DRAFT_SCHEMA_VERSION: u32 = 2;
const PROCESSING_CLAIM_EXT: &str = "claim";

/// A prose memory draft staged for librarian processing.
///
/// Constructed from raw text plus [`DraftMetadata`]. The [`DraftId`]
/// is derived deterministically from raw text plus stable provenance
/// identity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    id: DraftId,
    metadata: DraftMetadata,
    submitted_at: SystemTime,
    raw_text: String,
}

impl Draft {
    /// Construct a draft from raw text + legacy source + timestamp.
    ///
    /// New callers should prefer [`Draft::with_metadata`] so the
    /// scope-model fields are explicit.
    #[must_use]
    pub fn new(raw_text: String, source: &DraftSource, submitted_at: SystemTime) -> Self {
        let metadata = DraftMetadata::from_source(source, submitted_at);
        Self::with_metadata(raw_text, metadata)
    }

    /// Construct a draft from raw text plus explicit scope-model
    /// metadata.
    #[must_use]
    pub fn with_metadata(raw_text: String, metadata: DraftMetadata) -> Self {
        let id = DraftId::from_raw_text_and_metadata(&raw_text, &metadata);
        let submitted_at = metadata.submitted_at;
        Self {
            id,
            metadata,
            submitted_at,
            raw_text,
        }
    }

    /// Content-addressed identifier for this draft.
    #[must_use]
    pub fn id(&self) -> DraftId {
        self.id
    }

    /// Scope-model metadata and provenance for this draft.
    #[must_use]
    pub fn metadata(&self) -> &DraftMetadata {
        &self.metadata
    }

    /// When the draft was staged for processing.
    #[must_use]
    pub fn submitted_at(&self) -> SystemTime {
        self.submitted_at
    }

    /// The raw prose the draft carries. Never logged.
    #[must_use]
    pub fn prose(&self) -> &str {
        &self.raw_text
    }

    /// The raw text the draft carries. Same content as [`Draft::prose`],
    /// named for the v2 draft schema.
    #[must_use]
    pub fn raw_text(&self) -> &str {
        &self.raw_text
    }
}

/// Content-addressed identifier for a draft. First 8 bytes of the
/// SHA-256 of the prose, rendered as a 16-character lowercase hex
/// string on display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DraftId([u8; 8]);

impl DraftId {
    /// Derive a draft ID from prose content via SHA-256.
    #[must_use]
    pub fn from_prose(prose: &str) -> Self {
        let digest = Sha256::digest(prose.as_bytes());
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        Self(bytes)
    }

    /// Derive a draft ID from raw text plus stable provenance fields.
    #[must_use]
    pub fn from_raw_text_and_metadata(raw_text: &str, metadata: &DraftMetadata) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(raw_text.as_bytes());
        hasher.update([0]);
        hasher.update(metadata.source_surface.as_str().as_bytes());
        hasher.update([0]);
        update_optional(&mut hasher, metadata.source_agent.as_deref());
        update_optional(&mut hasher, metadata.source_project.as_deref());
        update_optional(&mut hasher, metadata.operator.as_deref());
        update_optional(&mut hasher, metadata.provenance_uri.as_deref());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        Self(bytes)
    }

    /// The raw 8-byte identifier.
    #[must_use]
    pub fn as_bytes(&self) -> [u8; 8] {
        self.0
    }

    /// Lower-case 16-character hex encoding.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(16);
        for byte in self.0 {
            use std::fmt::Write as _;
            // `write!` on a `String` is infallible; the `ok()` swallow
            // satisfies `clippy::unwrap_used` without panicking.
            write!(&mut out, "{byte:02x}").ok();
        }
        out
    }
}

fn update_optional(hasher: &mut Sha256, value: Option<&str>) {
    if let Some(v) = value {
        hasher.update(v.as_bytes());
    }
    hasher.update([0]);
}

/// Metadata required by `scope-model.md` § 4 for every raw draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftMetadata {
    /// Surface that submitted or exposed the raw memory.
    pub source_surface: DraftSourceSurface,
    /// Agent identity, when known (`claude`, `codex`, etc.).
    pub source_agent: Option<String>,
    /// Project or workspace identity, when known.
    pub source_project: Option<String>,
    /// Operator identity, when known.
    pub operator: Option<String>,
    /// Stable URI/path/event id for the source material.
    pub provenance_uri: Option<String>,
    /// Optional caller-provided context tags.
    pub context_tags: Vec<String>,
    /// When this draft entered Mimir's draft surface.
    pub submitted_at: SystemTime,
}

impl DraftMetadata {
    /// Construct metadata with only the required surface + timestamp.
    #[must_use]
    pub fn new(source_surface: DraftSourceSurface, submitted_at: SystemTime) -> Self {
        Self {
            source_surface,
            source_agent: None,
            source_project: None,
            operator: None,
            provenance_uri: None,
            context_tags: Vec::new(),
            submitted_at,
        }
    }

    /// Convert the older coarse [`DraftSource`] enum into v2 metadata.
    #[must_use]
    pub fn from_source(source: &DraftSource, submitted_at: SystemTime) -> Self {
        match source {
            DraftSource::Directory { path } => {
                let mut metadata = Self::new(DraftSourceSurface::Directory, submitted_at);
                metadata.provenance_uri = Some(path_to_file_uri(path));
                metadata
            }
            DraftSource::AutoMemorySweep { file } => {
                let mut metadata = Self::new(DraftSourceSurface::ClaudeMemory, submitted_at);
                metadata.source_agent = Some("claude".to_string());
                metadata.provenance_uri = Some(path_to_file_uri(file));
                metadata
            }
            DraftSource::CodexMemorySweep { file } => {
                let mut metadata = Self::new(DraftSourceSurface::CodexMemory, submitted_at);
                metadata.source_agent = Some("codex".to_string());
                metadata.provenance_uri = Some(path_to_file_uri(file));
                metadata
            }
            DraftSource::McpSubmit { workspace } => {
                let mut metadata = Self::new(DraftSourceSurface::Mcp, submitted_at);
                metadata.source_project = Some(workspace.clone());
                metadata
            }
            DraftSource::RepoHandoff { file } => {
                let mut metadata = Self::new(DraftSourceSurface::RepoHandoff, submitted_at);
                metadata.provenance_uri = Some(path_to_file_uri(file));
                metadata
            }
            DraftSource::CliSubmit => Self::new(DraftSourceSurface::Cli, submitted_at),
        }
    }
}

/// Source surface that exposed a draft to Mimir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftSourceSurface {
    /// File under the configured drafts directory.
    Directory,
    /// Claude native memory file sweep.
    ClaudeMemory,
    /// Codex memory file sweep.
    CodexMemory,
    /// MCP submission tool.
    Mcp,
    /// Librarian CLI submission.
    Cli,
    /// Repo-local handoff/status document.
    RepoHandoff,
    /// Future harness or persistent-agent export.
    AgentExport,
    /// Governed consensus quorum episode/result artifact.
    ConsensusQuorum,
    /// GitHub Copilot CLI local session-store database.
    CopilotSessionStore,
}

impl DraftSourceSurface {
    /// Parse a CLI/config spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "directory" => Some(Self::Directory),
            "claude_memory" | "claude-memory" => Some(Self::ClaudeMemory),
            "codex_memory" | "codex-memory" => Some(Self::CodexMemory),
            "mcp" => Some(Self::Mcp),
            "cli" => Some(Self::Cli),
            "repo_handoff" | "repo-handoff" => Some(Self::RepoHandoff),
            "agent_export" | "agent-export" => Some(Self::AgentExport),
            "consensus_quorum" | "consensus-quorum" => Some(Self::ConsensusQuorum),
            "copilot_session_store" | "copilot-session-store" => Some(Self::CopilotSessionStore),
            _ => None,
        }
    }

    /// Stable schema string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::ClaudeMemory => "claude_memory",
            Self::CodexMemory => "codex_memory",
            Self::Mcp => "mcp",
            Self::Cli => "cli",
            Self::RepoHandoff => "repo_handoff",
            Self::AgentExport => "agent_export",
            Self::ConsensusQuorum => "consensus_quorum",
            Self::CopilotSessionStore => "copilot_session_store",
        }
    }
}

impl fmt::Display for DraftId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Where a draft came from. Drives provenance + retention policy
/// (e.g. `AutoMemorySweep` drafts may be deleted after successful
/// commit; `CliSubmit` drafts may be retained for operator review).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DraftSource {
    /// Draft came from a file under the configured `drafts_dir`.
    Directory {
        /// Absolute path of the source file.
        path: PathBuf,
    },
    /// Draft came from sweeping Claude's auto-memory directory.
    AutoMemorySweep {
        /// Absolute path of the auto-memory file.
        file: PathBuf,
    },
    /// Draft came from sweeping Codex's memory directory.
    CodexMemorySweep {
        /// Absolute path of the Codex memory file.
        file: PathBuf,
    },
    /// Draft was submitted via the `mimir_submit_draft` MCP tool
    /// (a future addition; not yet wired).
    McpSubmit {
        /// The workspace identifier the submit was scoped to.
        workspace: String,
    },
    /// Draft came from an opted-in repo-local handoff/status file.
    RepoHandoff {
        /// Absolute path of the source file.
        file: PathBuf,
    },
    /// Draft was submitted via `mimir-librarian submit` CLI.
    CliSubmit,
}

/// Lifecycle state of a draft on the filesystem.
///
/// State transitions are atomic directory renames; the on-disk
/// location of the file IS the ground truth for state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DraftState {
    /// Not yet processed by any librarian run.
    Pending,
    /// A librarian run is currently processing this draft.
    Processing,
    /// Successfully written to the canonical log.
    Accepted,
    /// Intentionally skipped (for example exact duplicate).
    Skipped,
    /// Retry budget exhausted; operator review required.
    Failed,
    /// Unsafe, conflicting, or unresolved; review only.
    Quarantined,
}

impl DraftState {
    /// The directory name under `drafts_dir` where files in this
    /// state live.
    #[must_use]
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Accepted => "accepted",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
        }
    }
}

/// Result of a successful draft lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftTransition {
    /// Draft moved by this transition.
    pub id: DraftId,
    /// Source lifecycle state.
    pub from: DraftState,
    /// Target lifecycle state.
    pub to: DraftState,
    /// Path occupied before the atomic rename.
    pub source_path: PathBuf,
    /// Path occupied after the atomic rename.
    pub target_path: PathBuf,
}

/// Filesystem-backed draft surface.
#[derive(Debug, Clone)]
pub struct DraftStore {
    root: PathBuf,
}

impl DraftStore {
    /// Construct a store rooted at `drafts_dir`.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Root drafts directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory for a lifecycle state.
    #[must_use]
    pub fn state_dir(&self, state: DraftState) -> PathBuf {
        self.root.join(state.dir_name())
    }

    /// Canonical path for a draft in a lifecycle state.
    #[must_use]
    pub fn path_for(&self, state: DraftState, id: DraftId) -> PathBuf {
        self.state_dir(state).join(format!("{id}.json"))
    }

    /// Ensure all lifecycle directories exist.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::DraftIo`] when a lifecycle directory
    /// cannot be created.
    pub fn ensure_dirs(&self) -> Result<(), LibrarianError> {
        for state in DraftState::ALL {
            fs::create_dir_all(self.state_dir(state))?;
        }
        Ok(())
    }

    /// Submit a draft into `pending/` as a v2 JSON envelope.
    ///
    /// If the target file already exists, submission is treated as
    /// idempotent and the existing path is returned.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::DraftIo`] if the directory or file
    /// operations fail, or [`LibrarianError::DraftJson`] if the draft
    /// envelope cannot be encoded.
    pub fn submit(&self, draft: &Draft) -> Result<PathBuf, LibrarianError> {
        self.ensure_dirs()?;
        let target = self.path_for(DraftState::Pending, draft.id());
        if target.exists() {
            return Ok(target);
        }

        let tmp = target.with_file_name(format!(".{id}.json.tmp", id = draft.id()));
        let bytes = serde_json::to_vec_pretty(&DraftFileV2::from_draft(draft))?;
        fs::write(&tmp, bytes)?;
        if target.exists() {
            return Ok(target);
        }
        fs::rename(&tmp, &target)?;
        Ok(target)
    }

    /// Load one draft from a state directory.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::DraftIo`] if the file cannot be read,
    /// [`LibrarianError::DraftJson`] if it is not valid JSON,
    /// [`LibrarianError::UnsupportedDraftSchema`] for unknown schema
    /// versions, or [`LibrarianError::DraftIdMismatch`] if the stored
    /// ID does not match the envelope.
    pub fn load(&self, state: DraftState, id: DraftId) -> Result<Draft, LibrarianError> {
        Self::load_path(&self.path_for(state, id))
    }

    /// List drafts in one state directory.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DraftStore::load`] for any listed
    /// draft, and [`LibrarianError::DraftIo`] if the state directory
    /// cannot be read.
    pub fn list(&self, state: DraftState) -> Result<Vec<Draft>, LibrarianError> {
        self.ensure_dirs()?;
        let mut paths = Vec::new();
        for entry in fs::read_dir(self.state_dir(state))? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();

        let mut drafts = Vec::with_capacity(paths.len());
        for path in paths {
            drafts.push(Self::load_path(&path)?);
        }
        Ok(drafts)
    }

    /// Move a draft from one lifecycle state to another.
    ///
    /// The state graph is intentionally small:
    ///
    /// - `pending -> processing` claims work for a run.
    /// - `processing -> accepted | skipped | failed | quarantined`
    ///   finishes work.
    /// - `processing -> pending` recovers stale in-flight work after
    ///   a crash or abandoned run.
    ///
    /// The file move is a single filesystem rename. The method refuses
    /// to overwrite an existing target path.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::InvalidDraftTransition`] for edges
    /// outside the lifecycle graph, [`LibrarianError::DraftNotFound`]
    /// when the source file is absent, [`LibrarianError::DraftAlreadyExists`]
    /// when the target path is already occupied, or the same load / I/O
    /// errors as [`DraftStore::load`].
    pub fn transition(
        &self,
        id: DraftId,
        from: DraftState,
        to: DraftState,
    ) -> Result<DraftTransition, LibrarianError> {
        self.ensure_dirs()?;
        if !is_valid_transition(from, to) {
            return Err(LibrarianError::InvalidDraftTransition { from, to });
        }

        let source_path = self.path_for(from, id);
        if !source_path.exists() {
            return Err(LibrarianError::DraftNotFound { state: from, id });
        }

        let draft = Self::load_path(&source_path)?;
        if draft.id() != id {
            return Err(LibrarianError::DraftIdMismatch {
                declared: id.to_hex(),
                computed: draft.id().to_hex(),
            });
        }

        let target_path = self.path_for(to, id);
        if target_path.exists() {
            return Err(LibrarianError::DraftAlreadyExists { state: to, id });
        }

        if to == DraftState::Processing {
            self.write_processing_claim_marker(id)?;
        }

        if let Err(err) = fs::rename(&source_path, &target_path) {
            if to == DraftState::Processing {
                self.remove_processing_claim_marker(id)?;
            }
            return Err(err.into());
        }

        if from == DraftState::Processing {
            self.remove_processing_claim_marker(id)?;
        }
        Ok(DraftTransition {
            id,
            from,
            to,
            source_path,
            target_path,
        })
    }

    /// Recover stale drafts from `processing/` back to `pending/`.
    ///
    /// A processing draft is stale when its claim marker modified time
    /// is at or before `stale_before`. Callers usually pass
    /// `SystemTime::now() - processing_timeout`; tests can pass a future
    /// cutoff to recover all processing drafts deterministically.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DraftStore::transition`] for any
    /// stale draft that cannot be moved, plus [`LibrarianError::DraftIo`]
    /// for directory iteration or metadata errors.
    pub fn recover_stale_processing(
        &self,
        stale_before: SystemTime,
    ) -> Result<Vec<DraftTransition>, LibrarianError> {
        self.ensure_dirs()?;
        let mut stale = Vec::new();
        for entry in fs::read_dir(self.state_dir(DraftState::Processing))? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let draft = Self::load_path(&path)?;
            let modified = self.processing_claim_modified_at(draft.id(), &path)?;
            if modified <= stale_before {
                stale.push((path, draft.id()));
            }
        }
        stale.sort_by(|left, right| left.0.cmp(&right.0));

        let mut recovered = Vec::with_capacity(stale.len());
        for (_, id) in stale {
            recovered.push(self.transition(id, DraftState::Processing, DraftState::Pending)?);
        }
        Ok(recovered)
    }

    fn load_path(path: &Path) -> Result<Draft, LibrarianError> {
        let text = fs::read_to_string(path)?;
        let file: DraftFileV2 = serde_json::from_str(&text)?;
        file.into_draft()
    }

    fn processing_claim_path(&self, id: DraftId) -> PathBuf {
        self.state_dir(DraftState::Processing).join(format!(
            "{}.{}",
            id.to_hex(),
            PROCESSING_CLAIM_EXT
        ))
    }

    fn write_processing_claim_marker(&self, id: DraftId) -> Result<(), LibrarianError> {
        fs::write(self.processing_claim_path(id), b"claimed\n")?;
        Ok(())
    }

    fn remove_processing_claim_marker(&self, id: DraftId) -> Result<(), LibrarianError> {
        match fs::remove_file(self.processing_claim_path(id)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn processing_claim_modified_at(
        &self,
        id: DraftId,
        draft_path: &Path,
    ) -> Result<SystemTime, LibrarianError> {
        match fs::metadata(self.processing_claim_path(id)) {
            Ok(metadata) => Ok(metadata.modified()?),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                Ok(fs::metadata(draft_path)?.modified()?)
            }
            Err(err) => Err(err.into()),
        }
    }
}

fn is_valid_transition(from: DraftState, to: DraftState) -> bool {
    matches!(
        (from, to),
        (DraftState::Pending, DraftState::Processing)
            | (
                DraftState::Processing,
                DraftState::Pending
                    | DraftState::Accepted
                    | DraftState::Skipped
                    | DraftState::Failed
                    | DraftState::Quarantined,
            )
    )
}

impl DraftState {
    /// All known lifecycle states.
    pub const ALL: [Self; 6] = [
        Self::Pending,
        Self::Processing,
        Self::Accepted,
        Self::Skipped,
        Self::Failed,
        Self::Quarantined,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DraftFileV2 {
    schema_version: u32,
    id: String,
    source_surface: DraftSourceSurface,
    source_agent: Option<String>,
    source_project: Option<String>,
    operator: Option<String>,
    provenance_uri: Option<String>,
    context_tags: Vec<String>,
    submitted_at_unix_ms: u64,
    raw_text: String,
}

impl DraftFileV2 {
    fn from_draft(draft: &Draft) -> Self {
        let metadata = draft.metadata();
        Self {
            schema_version: DRAFT_SCHEMA_VERSION,
            id: draft.id().to_hex(),
            source_surface: metadata.source_surface,
            source_agent: metadata.source_agent.clone(),
            source_project: metadata.source_project.clone(),
            operator: metadata.operator.clone(),
            provenance_uri: metadata.provenance_uri.clone(),
            context_tags: metadata.context_tags.clone(),
            submitted_at_unix_ms: system_time_to_unix_ms(metadata.submitted_at),
            raw_text: draft.raw_text().to_string(),
        }
    }

    fn into_draft(self) -> Result<Draft, LibrarianError> {
        if self.schema_version != DRAFT_SCHEMA_VERSION {
            return Err(LibrarianError::UnsupportedDraftSchema {
                version: self.schema_version,
            });
        }
        let metadata = DraftMetadata {
            source_surface: self.source_surface,
            source_agent: self.source_agent,
            source_project: self.source_project,
            operator: self.operator,
            provenance_uri: self.provenance_uri,
            context_tags: self.context_tags,
            submitted_at: unix_ms_to_system_time(self.submitted_at_unix_ms),
        };
        let draft = Draft::with_metadata(self.raw_text, metadata);
        let computed = draft.id().to_hex();
        if self.id != computed {
            return Err(LibrarianError::DraftIdMismatch {
                declared: self.id,
                computed,
            });
        }
        Ok(draft)
    }
}

fn system_time_to_unix_ms(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

fn unix_ms_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn path_to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_id_is_content_addressed() {
        let id_a = DraftId::from_prose("hello world");
        let id_b = DraftId::from_prose("hello world");
        let id_c = DraftId::from_prose("hello world!");
        assert_eq!(id_a, id_b, "identical prose must produce identical IDs");
        assert_ne!(id_a, id_c, "different prose must produce different IDs");
    }

    #[test]
    fn draft_id_hex_is_16_chars() {
        let id = DraftId::from_prose("anything");
        let hex = id.to_hex();
        assert_eq!(hex.len(), 16);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn draft_id_display_matches_hex() {
        let id = DraftId::from_prose("anything");
        assert_eq!(format!("{id}"), id.to_hex());
    }

    #[test]
    fn draft_state_dir_names_are_distinct() {
        let names: std::collections::HashSet<_> =
            DraftState::ALL.iter().map(|s| s.dir_name()).collect();
        assert_eq!(
            names.len(),
            DraftState::ALL.len(),
            "every state must have a distinct dir name"
        );
    }

    #[test]
    fn draft_constructor_derives_id_from_text_and_metadata() {
        let prose = "Alain is the owner of Mimir.";
        let draft = Draft::new(
            prose.to_string(),
            &DraftSource::CliSubmit,
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(
            draft.id(),
            DraftId::from_raw_text_and_metadata(prose, draft.metadata())
        );
        assert_eq!(draft.prose(), prose);
        assert_eq!(draft.metadata().source_surface, DraftSourceSurface::Cli);
    }

    #[test]
    fn draft_metadata_carries_scope_model_fields() {
        let mut metadata =
            DraftMetadata::new(DraftSourceSurface::CodexMemory, SystemTime::UNIX_EPOCH);
        metadata.source_agent = Some("codex".to_string());
        metadata.source_project = Some("buildepicshit/Mimir".to_string());
        metadata.operator = Some("AlainDor".to_string());
        metadata.provenance_uri =
            Some("file:///home/hasnobeef/.codex/memories/mimir.md".to_string());
        metadata.context_tags = vec!["mimir".to_string(), "scope-model".to_string()];

        let draft = Draft::with_metadata(
            "remember the governed scope invariant".to_string(),
            metadata,
        );

        assert_eq!(
            draft.metadata().source_surface,
            DraftSourceSurface::CodexMemory
        );
        assert_eq!(draft.metadata().source_agent.as_deref(), Some("codex"));
        assert_eq!(
            draft.metadata().source_project.as_deref(),
            Some("buildepicshit/Mimir")
        );
        assert_eq!(draft.metadata().operator.as_deref(), Some("AlainDor"));
        assert_eq!(draft.metadata().context_tags.len(), 2);
    }

    #[test]
    fn draft_id_distinguishes_same_text_from_different_provenance() {
        let raw = "Use governed promotion for ecosystem memory.";
        let mut claude =
            DraftMetadata::new(DraftSourceSurface::ClaudeMemory, SystemTime::UNIX_EPOCH);
        claude.provenance_uri =
            Some("file:///home/hasnobeef/.claude/projects/mimir/memory/a.md".into());
        let mut codex = DraftMetadata::new(DraftSourceSurface::CodexMemory, SystemTime::UNIX_EPOCH);
        codex.provenance_uri = Some("file:///home/hasnobeef/.codex/memories/mimir.md".into());

        let claude_draft = Draft::with_metadata(raw.to_string(), claude);
        let codex_draft = Draft::with_metadata(raw.to_string(), codex);

        assert_ne!(
            claude_draft.id(),
            codex_draft.id(),
            "same text from different provenance must not collapse into one draft"
        );
    }

    #[test]
    fn draft_state_dir_names_cover_scope_model_lifecycle() {
        let states = [
            DraftState::Pending,
            DraftState::Processing,
            DraftState::Accepted,
            DraftState::Skipped,
            DraftState::Failed,
            DraftState::Quarantined,
        ];
        let names: std::collections::HashSet<_> = states.iter().map(|s| s.dir_name()).collect();
        assert_eq!(names.len(), states.len());
        assert_eq!(DraftState::Accepted.dir_name(), "accepted");
        assert_eq!(DraftState::Quarantined.dir_name(), "quarantined");
    }

    #[test]
    fn draft_store_submits_v2_json_to_pending() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let mut metadata = DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH);
        metadata.operator = Some("AlainDor".to_string());
        metadata.provenance_uri = Some("cli://mimir-librarian/submit".to_string());
        let draft =
            Draft::with_metadata("Mimir should govern memory scopes.".to_string(), metadata);

        let path = store.submit(&draft)?;
        assert_eq!(path, store.path_for(DraftState::Pending, draft.id()));
        assert!(path.exists());

        let saved = std::fs::read_to_string(&path)?;
        assert!(saved.contains("\"schema_version\": 2"));
        assert!(saved.contains("\"source_surface\": \"cli\""));
        assert!(saved.contains("\"operator\": \"AlainDor\""));

        let loaded = store.load(DraftState::Pending, draft.id())?;
        assert_eq!(loaded.id(), draft.id());
        assert_eq!(loaded.raw_text(), draft.raw_text());
        assert_eq!(loaded.metadata().source_surface, DraftSourceSurface::Cli);
        Ok(())
    }

    #[test]
    fn draft_store_submit_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Repeated sweeps should not duplicate a draft.".to_string(),
            DraftMetadata::new(DraftSourceSurface::ClaudeMemory, SystemTime::UNIX_EPOCH),
        );

        let first = store.submit(&draft)?;
        let second = store.submit(&draft)?;

        assert_eq!(first, second);
        assert_eq!(store.list(DraftState::Pending)?.len(), 1);
        Ok(())
    }

    #[test]
    fn draft_store_moves_pending_to_processing_then_terminal(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Draft lifecycle movement should be atomic and visible.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;

        let claimed = store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;
        assert_eq!(claimed.id, draft.id());
        assert_eq!(claimed.from, DraftState::Pending);
        assert_eq!(claimed.to, DraftState::Processing);
        assert!(!store.path_for(DraftState::Pending, draft.id()).exists());
        assert!(store.path_for(DraftState::Processing, draft.id()).exists());
        assert!(store.processing_claim_path(draft.id()).exists());
        assert_eq!(store.list(DraftState::Pending)?.len(), 0);
        assert_eq!(store.list(DraftState::Processing)?.len(), 1);

        let accepted =
            store.transition(draft.id(), DraftState::Processing, DraftState::Accepted)?;
        assert_eq!(accepted.to, DraftState::Accepted);
        assert!(!store.path_for(DraftState::Processing, draft.id()).exists());
        assert!(store.path_for(DraftState::Accepted, draft.id()).exists());
        assert!(!store.processing_claim_path(draft.id()).exists());
        let loaded = store.load(DraftState::Accepted, draft.id())?;
        assert_eq!(loaded.raw_text(), draft.raw_text());
        assert_eq!(store.list(DraftState::Processing)?.len(), 0);
        assert_eq!(store.list(DraftState::Accepted)?.len(), 1);
        Ok(())
    }

    #[test]
    fn draft_store_rejects_invalid_lifecycle_transition() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Terminal states should only be reached from processing.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;

        let err = match store.transition(draft.id(), DraftState::Pending, DraftState::Accepted) {
            Err(err) => err,
            Ok(transition) => {
                return Err(
                    format!("pending -> accepted must be rejected, got {transition:?}").into(),
                );
            }
        };
        assert!(matches!(
            err,
            LibrarianError::InvalidDraftTransition {
                from: DraftState::Pending,
                to: DraftState::Accepted
            }
        ));
        assert!(store.path_for(DraftState::Pending, draft.id()).exists());
        assert!(!store.path_for(DraftState::Accepted, draft.id()).exists());
        Ok(())
    }

    #[test]
    fn draft_store_allows_every_processing_terminal_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        for terminal in [
            DraftState::Accepted,
            DraftState::Skipped,
            DraftState::Failed,
            DraftState::Quarantined,
        ] {
            let draft = Draft::with_metadata(
                format!("Draft should finish as {}.", terminal.dir_name()),
                DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
            );
            store.submit(&draft)?;
            store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;

            let finished = store.transition(draft.id(), DraftState::Processing, terminal)?;
            assert_eq!(finished.to, terminal);
            assert!(store.path_for(terminal, draft.id()).exists());
            assert_eq!(store.load(terminal, draft.id())?.id(), draft.id());
        }
        assert_eq!(store.list(DraftState::Processing)?.len(), 0);
        Ok(())
    }

    #[test]
    fn draft_store_rejects_transition_when_target_exists() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Draft transition should never overwrite a target file.".to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;
        std::fs::copy(
            store.path_for(DraftState::Processing, draft.id()),
            store.path_for(DraftState::Accepted, draft.id()),
        )?;

        let err = match store.transition(draft.id(), DraftState::Processing, DraftState::Accepted) {
            Err(err) => err,
            Ok(transition) => {
                return Err(format!(
                    "transition must not overwrite an existing terminal draft, got {transition:?}"
                )
                .into());
            }
        };
        assert!(matches!(
            err,
            LibrarianError::DraftAlreadyExists {
                state: DraftState::Accepted,
                id: existing
            } if existing == draft.id()
        ));
        assert!(store.path_for(DraftState::Processing, draft.id()).exists());
        Ok(())
    }

    #[test]
    fn draft_store_reports_missing_transition_source() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let id = DraftId::from_prose("missing");

        let err = match store.transition(id, DraftState::Pending, DraftState::Processing) {
            Err(err) => err,
            Ok(transition) => {
                return Err(format!(
                    "missing pending draft should be an explicit error, got {transition:?}"
                )
                .into());
            }
        };
        assert!(matches!(
            err,
            LibrarianError::DraftNotFound {
                state: DraftState::Pending,
                id: missing
            } if missing == id
        ));
        Ok(())
    }

    #[test]
    fn draft_store_recovers_stale_processing_back_to_pending(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Crash recovery should return stale processing drafts to pending.".to_string(),
            DraftMetadata::new(DraftSourceSurface::CodexMemory, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;

        let cutoff = SystemTime::now() + Duration::from_secs(60);
        let recovered = store.recover_stale_processing(cutoff)?;
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, draft.id());
        assert_eq!(recovered[0].from, DraftState::Processing);
        assert_eq!(recovered[0].to, DraftState::Pending);
        assert_eq!(store.list(DraftState::Processing)?.len(), 0);
        assert_eq!(store.list(DraftState::Pending)?.len(), 1);

        let recovered_again = store.recover_stale_processing(cutoff)?;
        assert!(
            recovered_again.is_empty(),
            "recovery should be idempotent once no drafts remain in processing"
        );
        Ok(())
    }

    #[test]
    fn draft_store_keeps_fresh_processing_drafts_in_place() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = Draft::with_metadata(
            "Fresh in-flight work should not be recovered early.".to_string(),
            DraftMetadata::new(DraftSourceSurface::ClaudeMemory, SystemTime::UNIX_EPOCH),
        );
        store.submit(&draft)?;
        store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;

        let recovered = store.recover_stale_processing(SystemTime::UNIX_EPOCH)?;
        assert!(recovered.is_empty());
        assert_eq!(store.list(DraftState::Processing)?.len(), 1);
        assert_eq!(store.list(DraftState::Pending)?.len(), 0);
        Ok(())
    }

    #[test]
    fn source_surface_parse_accepts_cli_spellings() {
        assert_eq!(
            DraftSourceSurface::parse("codex-memory"),
            Some(DraftSourceSurface::CodexMemory)
        );
        assert_eq!(
            DraftSourceSurface::parse("codex_memory"),
            Some(DraftSourceSurface::CodexMemory)
        );
        assert_eq!(
            DraftSourceSurface::parse("consensus-quorum"),
            Some(DraftSourceSurface::ConsensusQuorum)
        );
        assert_eq!(
            DraftSourceSurface::parse("consensus_quorum"),
            Some(DraftSourceSurface::ConsensusQuorum)
        );
        assert_eq!(DraftSourceSurface::parse("unknown"), None);
    }
}
