//! File-backed consensus quorum episode/result/output envelopes.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::LibrarianError;

/// Current on-disk quorum episode/result schema version.
pub const QUORUM_SCHEMA_VERSION: u32 = 1;

/// File-backed quorum episode/result store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuorumStore {
    root: PathBuf,
}

impl QuorumStore {
    /// Create a quorum store rooted at `root`.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Write an episode envelope.
    ///
    /// # Errors
    ///
    /// Returns I/O or JSON serialization errors.
    pub fn create_episode(&self, episode: &QuorumEpisode) -> Result<PathBuf, LibrarianError> {
        let path = self.episode_path(&episode.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(episode)?;
        fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Load a quorum episode by id.
    ///
    /// # Errors
    ///
    /// Returns I/O or JSON decoding errors.
    pub fn load_episode(&self, id: &str) -> Result<QuorumEpisode, LibrarianError> {
        let bytes = fs::read(self.episode_path(id))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Write the synthesized result for an episode.
    ///
    /// # Errors
    ///
    /// Returns I/O or JSON serialization errors.
    pub fn save_result(&self, result: &QuorumResult) -> Result<PathBuf, LibrarianError> {
        let path = self.result_path(&result.episode_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(result)?;
        fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Load the synthesized result for an episode.
    ///
    /// # Errors
    ///
    /// Returns I/O or JSON decoding errors.
    pub fn load_result(&self, episode_id: &str) -> Result<QuorumResult, LibrarianError> {
        let bytes = fs::read(self.result_path(episode_id))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Return the canonical result artifact path for an episode.
    #[must_use]
    pub fn result_artifact_path(&self, episode_id: &str) -> PathBuf {
        self.result_path(episode_id)
    }

    /// Append one participant output for a deliberation round.
    ///
    /// # Errors
    ///
    /// Returns I/O, JSON serialization, duplicate-output, or protocol errors.
    pub fn append_participant_output(
        &self,
        output: &QuorumParticipantOutput,
    ) -> Result<PathBuf, LibrarianError> {
        let episode = self.load_episode(&output.episode_id)?;
        self.validate_participant_output(&episode, output)?;
        let path = self.output_path(output);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(output)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => file.write_all(&bytes)?,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(LibrarianError::QuorumOutputAlreadyExists {
                    episode_id: output.episode_id.clone(),
                    round: output.round.as_str().to_string(),
                    participant_id: output.participant_id.clone(),
                });
            }
            Err(err) => return Err(err.into()),
        }
        Ok(path)
    }

    /// Load all participant outputs for one round.
    ///
    /// # Errors
    ///
    /// Returns I/O or JSON decoding errors.
    pub fn load_round_outputs(
        &self,
        episode_id: &str,
        round: QuorumRound,
    ) -> Result<Vec<QuorumParticipantOutput>, LibrarianError> {
        let dir = self.round_dir(episode_id, round);
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut outputs = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let bytes = fs::read(path)?;
                let output: QuorumParticipantOutput = serde_json::from_slice(&bytes)?;
                if output.round == round {
                    outputs.push(output);
                }
            }
        }
        outputs.sort_by(|left, right| {
            left.participant_id
                .cmp(&right.participant_id)
                .then(left.output_id.cmp(&right.output_id))
        });
        Ok(outputs)
    }

    /// Return the prior outputs that may be shown to a participant in `round`.
    ///
    /// # Errors
    ///
    /// Returns a protocol error until the prior round is complete.
    pub fn visible_outputs_for_round(
        &self,
        episode_id: &str,
        round: QuorumRound,
    ) -> Result<Vec<QuorumParticipantOutput>, LibrarianError> {
        let episode = self.load_episode(episode_id)?;
        match round {
            QuorumRound::Independent => Ok(Vec::new()),
            QuorumRound::Critique => {
                self.require_round_complete(&episode, QuorumRound::Independent)
            }
            QuorumRound::Revision => {
                let mut outputs =
                    self.require_round_complete(&episode, QuorumRound::Independent)?;
                outputs.extend(self.require_round_complete(&episode, QuorumRound::Critique)?);
                Ok(outputs)
            }
        }
    }

    /// Build the JSON request contract a participant adapter should consume.
    ///
    /// # Errors
    ///
    /// Returns I/O, JSON decoding, or protocol errors when the participant
    /// is unknown or prior-round visibility is not available yet.
    pub fn build_adapter_request(
        &self,
        episode_id: &str,
        participant_id: &str,
        round: QuorumRound,
    ) -> Result<QuorumAdapterRequest, LibrarianError> {
        let episode = self.load_episode(episode_id)?;
        let participant = episode
            .participants
            .iter()
            .find(|candidate| candidate.id == participant_id)
            .cloned()
            .ok_or_else(|| {
                quorum_protocol_violation(
                    &episode.id,
                    format!("unknown participant {participant_id}"),
                )
            })?;
        let visible_prior_outputs = self.visible_outputs_for_round(episode_id, round)?;
        let visible_prior_output_ids = visible_prior_outputs
            .iter()
            .map(|output| output.output_id.clone())
            .collect();
        Ok(QuorumAdapterRequest {
            schema_version: QUORUM_SCHEMA_VERSION,
            episode_id: episode.id,
            participant,
            round,
            question: episode.question,
            target_project: episode.target_project,
            target_scope: episode.target_scope,
            evidence_policy: episode.evidence_policy,
            visible_prior_output_ids,
            visible_prior_outputs,
        })
    }

    fn episode_path(&self, id: &str) -> PathBuf {
        self.episode_dir(id).join("episode.json")
    }

    fn result_path(&self, episode_id: &str) -> PathBuf {
        self.episode_dir(episode_id).join("result.json")
    }

    fn episode_dir(&self, id: &str) -> PathBuf {
        self.root.join("episodes").join(quorum_id_slug(id))
    }

    fn round_dir(&self, episode_id: &str, round: QuorumRound) -> PathBuf {
        self.episode_dir(episode_id)
            .join("outputs")
            .join(round.as_str())
    }

    fn output_path(&self, output: &QuorumParticipantOutput) -> PathBuf {
        self.round_dir(&output.episode_id, output.round)
            .join(format!("{}.json", quorum_id_slug(&output.participant_id)))
    }

    fn validate_participant_output(
        &self,
        episode: &QuorumEpisode,
        output: &QuorumParticipantOutput,
    ) -> Result<(), LibrarianError> {
        if output.schema_version != QUORUM_SCHEMA_VERSION {
            return Err(quorum_protocol_violation(
                &episode.id,
                format!(
                    "unsupported output schema version {}; expected {QUORUM_SCHEMA_VERSION}",
                    output.schema_version
                ),
            ));
        }
        if output.output_id.trim().is_empty() {
            return Err(quorum_protocol_violation(
                &episode.id,
                "participant output id must not be empty",
            ));
        }
        if !episode
            .participants
            .iter()
            .any(|participant| participant.id == output.participant_id)
        {
            return Err(quorum_protocol_violation(
                &episode.id,
                format!("unknown participant {}", output.participant_id),
            ));
        }
        match output.round {
            QuorumRound::Independent => {
                if !output.visible_prior_output_ids.is_empty() {
                    return Err(quorum_protocol_violation(
                        &episode.id,
                        "independent outputs must not reference prior visible outputs",
                    ));
                }
            }
            QuorumRound::Critique => {
                let visible = self.require_round_complete(episode, QuorumRound::Independent)?;
                require_exact_visible_prior_ids(output, &visible)?;
            }
            QuorumRound::Revision => {
                let mut visible = self.require_round_complete(episode, QuorumRound::Independent)?;
                visible.extend(self.require_round_complete(episode, QuorumRound::Critique)?);
                require_exact_visible_prior_ids(output, &visible)?;
            }
        }
        Ok(())
    }

    fn require_round_complete(
        &self,
        episode: &QuorumEpisode,
        round: QuorumRound,
    ) -> Result<Vec<QuorumParticipantOutput>, LibrarianError> {
        let outputs = self.load_round_outputs(&episode.id, round)?;
        let expected: BTreeSet<&str> = episode
            .participants
            .iter()
            .map(|participant| participant.id.as_str())
            .collect();
        let actual: BTreeSet<&str> = outputs
            .iter()
            .map(|output| output.participant_id.as_str())
            .collect();
        let missing: Vec<&str> = expected.difference(&actual).copied().collect();
        if !missing.is_empty() {
            return Err(quorum_protocol_violation(
                &episode.id,
                format!(
                    "{} round is incomplete; missing participant outputs: {}",
                    round.as_str(),
                    missing.join(", ")
                ),
            ));
        }
        Ok(outputs)
    }
}

/// Lifecycle state for one quorum episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuorumEpisodeState {
    /// Episode has been requested but participants are not enlisted yet.
    Requested,
    /// Participant surfaces/personas have been selected.
    Enlisted,
    /// Participants are producing independent first-pass outputs.
    IndependentRound,
    /// Participants are critiquing prior outputs.
    CritiqueRound,
    /// Participants may revise or hold positions.
    RevisionRound,
    /// Participants vote with rationale.
    VoteRound,
    /// A result has been synthesized.
    Synthesized,
    /// Proposed memory drafts entered the librarian draft path.
    SubmittedToLibrarian,
    /// Episode retained for audit but not submitted.
    Archived,
    /// Episode requires review and must not be used as evidence.
    Quarantined,
}

/// Participant identity captured for quorum auditability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumParticipant {
    /// Stable participant id inside the episode.
    pub id: String,
    /// Adapter name, such as `claude` or `codex`.
    pub adapter: String,
    /// Concrete model name when available.
    pub model: Option<String>,
    /// Persona prompt lens used for this participant.
    pub persona: String,
    /// Prompt template version used for this participant.
    pub prompt_template_version: String,
    /// Runtime surface that executed the participant.
    pub runtime_surface: String,
    /// Explicit tool grants for the participant.
    pub tool_permissions: Vec<String>,
}

/// Deliberation round that can receive participant outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuorumRound {
    /// Independent first pass before seeing other answers.
    Independent,
    /// Critique of completed independent outputs.
    Critique,
    /// Revision after critique visibility.
    Revision,
}

impl QuorumRound {
    fn as_str(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::Critique => "critique",
            Self::Revision => "revision",
        }
    }
}

/// Requested quorum episode envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumEpisode {
    /// On-disk schema version.
    pub schema_version: u32,
    /// Stable quorum episode id.
    pub id: String,
    /// Request timestamp in Unix milliseconds.
    pub requested_at_unix_ms: u64,
    /// Human or agent that requested the quorum.
    pub requester: String,
    /// Question under deliberation.
    pub question: String,
    /// Project/workspace target, when project-bound.
    pub target_project: Option<String>,
    /// Governance scope target, when known.
    pub target_scope: Option<String>,
    /// Evidence policy for the episode.
    pub evidence_policy: String,
    /// Current protocol state.
    pub state: QuorumEpisodeState,
    /// Participants/personas selected for the episode.
    pub participants: Vec<QuorumParticipant>,
    /// Stable provenance URI for artifacts derived from this episode.
    pub provenance_uri: String,
}

/// Participant prompt/response captured for a deliberation round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumParticipantOutput {
    /// On-disk schema version.
    pub schema_version: u32,
    /// Episode this output belongs to.
    pub episode_id: String,
    /// Stable participant-output id within the episode.
    pub output_id: String,
    /// Participant id from the episode.
    pub participant_id: String,
    /// Deliberation round.
    pub round: QuorumRound,
    /// Submission timestamp in Unix milliseconds.
    pub submitted_at_unix_ms: u64,
    /// Prompt sent to the participant.
    pub prompt: String,
    /// Participant response.
    pub response: String,
    /// Prior output ids visible to the participant while producing this output.
    pub visible_prior_output_ids: Vec<String>,
    /// Evidence used by this participant output.
    pub evidence_used: Vec<String>,
}

/// Request payload consumed by a future participant adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumAdapterRequest {
    /// On-disk/wire schema version.
    pub schema_version: u32,
    /// Episode under deliberation.
    pub episode_id: String,
    /// Participant identity and adapter metadata.
    pub participant: QuorumParticipant,
    /// Round the adapter is being asked to answer.
    pub round: QuorumRound,
    /// Question under deliberation.
    pub question: String,
    /// Project/workspace target, when project-bound.
    pub target_project: Option<String>,
    /// Governance scope target, when known.
    pub target_scope: Option<String>,
    /// Evidence policy for the episode.
    pub evidence_policy: String,
    /// Prior output ids the adapter is allowed to see.
    pub visible_prior_output_ids: Vec<String>,
    /// Prior outputs the adapter is allowed to see.
    pub visible_prior_outputs: Vec<QuorumParticipantOutput>,
}

/// Final quorum decision status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Strong enough to act, still subject to owner choice.
    Recommend,
    /// Material disagreement remains.
    Split,
    /// More source checking or experiments are needed.
    NeedsEvidence,
    /// Quorum recommends against the proposed direction.
    Reject,
    /// Trust, safety, or prompt-injection issue.
    Unsafe,
}

/// Degree of agreement in a quorum result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusLevel {
    /// All non-abstaining participants agree.
    Unanimous,
    /// Most agree and dissent is weak or bounded.
    StrongMajority,
    /// Most agree but dissent remains important.
    WeakMajority,
    /// No stable consensus.
    Contested,
    /// Insufficient participation or evidence.
    Abstained,
}

/// Individual participant vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteChoice {
    /// Participant agrees with the recommendation.
    Agree,
    /// Participant disagrees with the recommendation.
    Disagree,
    /// Participant abstains.
    Abstain,
}

/// Vote plus confidence and rationale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticipantVote {
    /// Participant id from the episode.
    pub participant_id: String,
    /// Participant vote.
    pub vote: VoteChoice,
    /// Vote confidence, in the range 0.0 through 1.0.
    pub confidence: f32,
    /// Short vote rationale.
    pub rationale: String,
}

/// Synthesized quorum result envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuorumResult {
    /// On-disk schema version.
    pub schema_version: u32,
    /// Episode this result belongs to.
    pub episode_id: String,
    /// Question under deliberation.
    pub question: String,
    /// Final recommendation text.
    pub recommendation: String,
    /// Decision status.
    pub decision_status: DecisionStatus,
    /// Consensus level.
    pub consensus_level: ConsensusLevel,
    /// Overall result confidence, in the range 0.0 through 1.0.
    pub confidence: f32,
    /// Main supporting points.
    pub supporting_points: Vec<String>,
    /// Dissenting points, retained as first-class evidence.
    pub dissenting_points: Vec<String>,
    /// Questions left unresolved by the quorum.
    pub unresolved_questions: Vec<String>,
    /// Evidence used by participants/synthesis.
    pub evidence_used: Vec<String>,
    /// Votes from participants.
    pub participant_votes: Vec<ParticipantVote>,
    /// Proposed raw memory drafts for the librarian path.
    pub proposed_memory_drafts: Vec<String>,
}

fn quorum_id_slug(id: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(id.as_bytes());
    let mut suffix = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        suffix.push(char::from(HEX[usize::from(byte >> 4)]));
        suffix.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let prefix: String = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    if prefix.is_empty() {
        suffix
    } else {
        format!("{prefix}-{suffix}")
    }
}

fn require_exact_visible_prior_ids(
    output: &QuorumParticipantOutput,
    visible: &[QuorumParticipantOutput],
) -> Result<(), LibrarianError> {
    let expected: BTreeSet<&str> = visible.iter().map(|item| item.output_id.as_str()).collect();
    let actual: BTreeSet<&str> = output
        .visible_prior_output_ids
        .iter()
        .map(String::as_str)
        .collect();
    if expected != actual || actual.len() != output.visible_prior_output_ids.len() {
        return Err(quorum_protocol_violation(
            &output.episode_id,
            format!(
                "{} output from {} must reference exactly the visible prior output ids",
                output.round.as_str(),
                output.participant_id
            ),
        ));
    }
    Ok(())
}

fn quorum_protocol_violation(episode_id: &str, message: impl Into<String>) -> LibrarianError {
    LibrarianError::QuorumProtocolViolation {
        episode_id: episode_id.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(name: &str, persona: &str) -> QuorumParticipant {
        QuorumParticipant {
            id: name.to_string(),
            adapter: name.to_string(),
            model: Some(format!("{name}-model")),
            persona: persona.to_string(),
            prompt_template_version: "v1".to_string(),
            runtime_surface: name.to_string(),
            tool_permissions: vec!["read_memory".to_string()],
        }
    }

    fn episode() -> QuorumEpisode {
        QuorumEpisode {
            schema_version: QUORUM_SCHEMA_VERSION,
            id: "qr-2026-04-24-001".to_string(),
            requested_at_unix_ms: 1_772_000_000_000,
            requester: "operator:AlainDor".to_string(),
            question: "Should Mimir keep remote sync explicit?".to_string(),
            target_project: Some("buildepicshit/Mimir".to_string()),
            target_scope: Some("project".to_string()),
            evidence_policy: "source_backed_when_claiming_external_facts".to_string(),
            state: QuorumEpisodeState::Requested,
            participants: vec![
                participant("claude", "architect"),
                participant("codex", "implementation_engineer"),
            ],
            provenance_uri: "quorum://episode/qr-2026-04-24-001".to_string(),
        }
    }

    fn output(
        output_id: &str,
        participant_id: &str,
        round: QuorumRound,
        visible_prior_output_ids: Vec<String>,
    ) -> QuorumParticipantOutput {
        QuorumParticipantOutput {
            schema_version: QUORUM_SCHEMA_VERSION,
            episode_id: "qr-2026-04-24-001".to_string(),
            output_id: output_id.to_string(),
            participant_id: participant_id.to_string(),
            round,
            submitted_at_unix_ms: 1_772_000_001_000,
            prompt: format!("Prompt for {participant_id}"),
            response: format!("Response from {participant_id}"),
            visible_prior_output_ids,
            evidence_used: vec!["docs/concepts/consensus-quorum.md".to_string()],
        }
    }

    #[test]
    fn quorum_store_creates_and_loads_episode() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = QuorumStore::new(tmp.path());
        let episode = episode();

        let path = store.create_episode(&episode)?;
        assert!(path.ends_with("episode.json"));
        let loaded = store.load_episode(&episode.id)?;
        assert_eq!(loaded, episode);
        Ok(())
    }

    #[test]
    fn quorum_store_saves_result_with_dissent_and_votes() -> Result<(), Box<dyn std::error::Error>>
    {
        let tmp = tempfile::tempdir()?;
        let store = QuorumStore::new(tmp.path());
        let result = QuorumResult {
            schema_version: QUORUM_SCHEMA_VERSION,
            episode_id: "qr-2026-04-24-001".to_string(),
            question: "Should Mimir keep remote sync explicit?".to_string(),
            recommendation: "Keep sync explicit and expose refresh status.".to_string(),
            decision_status: DecisionStatus::Recommend,
            consensus_level: ConsensusLevel::StrongMajority,
            confidence: 0.82,
            supporting_points: vec!["Launch/capture stay transparent.".to_string()],
            dissenting_points: vec!["Operator may forget to push.".to_string()],
            unresolved_questions: vec!["Service adapter protocol remains open.".to_string()],
            evidence_used: vec!["docs/planning/2026-04-24-transparent-agent-harness.md".to_string()],
            participant_votes: vec![
                ParticipantVote {
                    participant_id: "claude".to_string(),
                    vote: VoteChoice::Agree,
                    confidence: 0.86,
                    rationale: "Explicit sync protects native launch flow.".to_string(),
                },
                ParticipantVote {
                    participant_id: "codex".to_string(),
                    vote: VoteChoice::Disagree,
                    confidence: 0.42,
                    rationale: "A reminder surface may still be needed.".to_string(),
                },
            ],
            proposed_memory_drafts: vec![
                "Remote sync must remain explicit during launch and capture.".to_string(),
            ],
        };

        store.save_result(&result)?;
        let loaded = store.load_result(&result.episode_id)?;
        assert_eq!(loaded, result);
        assert_eq!(loaded.dissenting_points.len(), 1);
        assert_eq!(loaded.participant_votes[1].vote, VoteChoice::Disagree);
        Ok(())
    }

    #[test]
    fn quorum_store_blocks_critique_until_independent_outputs_complete(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = QuorumStore::new(tmp.path());
        let episode = episode();
        store.create_episode(&episode)?;

        store.append_participant_output(&output(
            "out-independent-claude",
            "claude",
            QuorumRound::Independent,
            Vec::new(),
        ))?;

        let critique_before_complete = output(
            "out-critique-claude",
            "claude",
            QuorumRound::Critique,
            vec!["out-independent-claude".to_string()],
        );
        let err = match store.append_participant_output(&critique_before_complete) {
            Ok(path) => {
                return Err(std::io::Error::other(format!(
                    "critique must wait for every independent first pass, wrote {}",
                    path.display()
                ))
                .into());
            }
            Err(err) => err,
        };
        assert!(matches!(
            err,
            LibrarianError::QuorumProtocolViolation { .. }
        ));
        assert!(
            store
                .visible_outputs_for_round(&episode.id, QuorumRound::Critique)
                .is_err(),
            "critique visibility must stay closed until every independent output is present",
        );

        store.append_participant_output(&output(
            "out-independent-codex",
            "codex",
            QuorumRound::Independent,
            Vec::new(),
        ))?;
        let visible = store.visible_outputs_for_round(&episode.id, QuorumRound::Critique)?;
        let visible_ids: Vec<_> = visible.iter().map(|item| item.output_id.clone()).collect();
        assert_eq!(
            visible_ids,
            vec!["out-independent-claude", "out-independent-codex"]
        );

        store.append_participant_output(&output(
            "out-critique-claude",
            "claude",
            QuorumRound::Critique,
            visible_ids,
        ))?;
        let critiques = store.load_round_outputs(&episode.id, QuorumRound::Critique)?;
        assert_eq!(critiques.len(), 1);
        assert_eq!(critiques[0].participant_id, "claude");
        Ok(())
    }

    #[test]
    fn quorum_store_rejects_independent_output_with_prior_visibility(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = QuorumStore::new(tmp.path());
        store.create_episode(&episode())?;

        let output = output(
            "out-independent-claude",
            "claude",
            QuorumRound::Independent,
            vec!["out-independent-codex".to_string()],
        );
        let err = match store.append_participant_output(&output) {
            Ok(path) => {
                return Err(std::io::Error::other(format!(
                    "independent output cannot see prior answers, wrote {}",
                    path.display()
                ))
                .into());
            }
            Err(err) => err,
        };
        assert!(matches!(
            err,
            LibrarianError::QuorumProtocolViolation { .. }
        ));
        Ok(())
    }

    #[test]
    fn quorum_store_builds_adapter_request_with_visible_outputs(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = QuorumStore::new(tmp.path());
        let episode = episode();
        store.create_episode(&episode)?;
        store.append_participant_output(&output(
            "out-independent-claude",
            "claude",
            QuorumRound::Independent,
            Vec::new(),
        ))?;
        store.append_participant_output(&output(
            "out-independent-codex",
            "codex",
            QuorumRound::Independent,
            Vec::new(),
        ))?;

        let request = store.build_adapter_request(&episode.id, "codex", QuorumRound::Critique)?;
        assert_eq!(request.episode_id, episode.id);
        assert_eq!(request.participant.id, "codex");
        assert_eq!(request.round, QuorumRound::Critique);
        assert_eq!(
            request.visible_prior_output_ids,
            vec!["out-independent-claude", "out-independent-codex"]
        );
        assert_eq!(request.visible_prior_outputs.len(), 2);
        Ok(())
    }
}
