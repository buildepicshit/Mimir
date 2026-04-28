//! Read-only GitHub Copilot CLI session-store adapter.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use mimir_librarian::{Draft, DraftMetadata, DraftSourceSurface, DraftStore, LibrarianConfig};
use rusqlite::{params, Connection, ErrorCode, OpenFlags, Row};
use serde::Serialize;

use super::CliError;

const DEFAULT_LIMIT: usize = 10;
const DEFAULT_CHECKPOINT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 100;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(25);

const EXPECTED_SCHEMA: &[ExpectedTable] = &[
    ExpectedTable {
        name: "sessions",
        columns: &[
            "id",
            "repository",
            "branch",
            "summary",
            "created_at",
            "updated_at",
        ],
    },
    ExpectedTable {
        name: "turns",
        columns: &[
            "session_id",
            "turn_index",
            "user_message",
            "assistant_response",
            "timestamp",
        ],
    },
    ExpectedTable {
        name: "session_files",
        columns: &[
            "session_id",
            "file_path",
            "tool_name",
            "turn_index",
            "first_seen_at",
        ],
    },
    ExpectedTable {
        name: "session_refs",
        columns: &[
            "session_id",
            "ref_type",
            "ref_value",
            "turn_index",
            "created_at",
        ],
    },
    ExpectedTable {
        name: "checkpoints",
        columns: &[
            "session_id",
            "checkpoint_number",
            "title",
            "overview",
            "created_at",
        ],
    },
];

struct ExpectedTable {
    name: &'static str,
    columns: &'static [&'static str],
}

#[derive(Debug)]
pub(crate) enum CopilotSessionStoreError {
    MissingDatabase {
        path: PathBuf,
    },
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    SchemaCheck {
        operation: &'static str,
        source: rusqlite::Error,
    },
    SchemaDrift {
        problems: Vec<String>,
    },
    Query {
        operation: &'static str,
        source: rusqlite::Error,
    },
    Locked {
        operation: &'static str,
        source: rusqlite::Error,
    },
}

impl fmt::Display for CopilotSessionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDatabase { path } => {
                write!(
                    formatter,
                    "Copilot session-store database not found: {}",
                    path.display()
                )
            }
            Self::Open { path, source } => {
                write!(
                    formatter,
                    "could not open Copilot session-store read-only at {}: {source}",
                    path.display()
                )
            }
            Self::SchemaCheck { operation, source } => {
                write!(
                    formatter,
                    "could not inspect Copilot session-store schema during {operation}: {source}"
                )
            }
            Self::SchemaDrift { problems } => {
                write!(
                    formatter,
                    "Copilot session-store schema drift detected: {}",
                    problems.join("; ")
                )
            }
            Self::Query { operation, source } => {
                write!(
                    formatter,
                    "could not query Copilot session-store during {operation}: {source}"
                )
            }
            Self::Locked { operation, source } => {
                write!(
                    formatter,
                    "Copilot session-store is locked during {operation}: {source}"
                )
            }
        }
    }
}

impl Error for CopilotSessionStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open { source, .. }
            | Self::SchemaCheck { source, .. }
            | Self::Query { source, .. }
            | Self::Locked { source, .. } => Some(source),
            Self::MissingDatabase { .. } | Self::SchemaDrift { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CopilotRepoScope {
    All,
    Scoped,
    Undetected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CopilotSessionStoreOutcome {
    SchemaCheck {
        db_path: String,
        schema_ok: bool,
        problems: Vec<String>,
    },
    Recent {
        db_path: String,
        repo: String,
        repo_scope: CopilotRepoScope,
        count: usize,
        sessions: Vec<CopilotSession>,
        recent_files: Vec<CopilotFile>,
    },
    Files {
        db_path: String,
        repo: String,
        repo_scope: CopilotRepoScope,
        count: usize,
        files: Vec<CopilotFile>,
    },
    Checkpoints {
        db_path: String,
        repo: String,
        repo_scope: CopilotRepoScope,
        count: usize,
        checkpoints: Vec<CopilotCheckpoint>,
    },
    Search {
        db_path: String,
        repo: String,
        repo_scope: CopilotRepoScope,
        query: String,
        count: usize,
        results: Vec<CopilotSearchResult>,
    },
    DraftsSubmitted {
        db_path: String,
        repo: String,
        repo_scope: CopilotRepoScope,
        submitted: usize,
        drafts: Vec<CopilotSubmittedDraft>,
    },
}

impl CopilotSessionStoreOutcome {
    pub(crate) fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::SchemaCheck {
                schema_ok: false,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopilotSession {
    id_short: String,
    id_full: String,
    repository: String,
    branch: Option<String>,
    summary: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    turns_count: i64,
    files_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopilotFile {
    file_path: String,
    tool_name: Option<String>,
    first_seen_at: Option<String>,
    session_id: String,
    session_id_full: String,
    session_summary: Option<String>,
    repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopilotCheckpoint {
    checkpoint_number: i64,
    title: Option<String>,
    overview: Option<String>,
    created_at: Option<String>,
    session_id: String,
    session_id_full: String,
    session_summary: Option<String>,
    repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopilotSearchResult {
    source_type: String,
    excerpt: String,
    session_id: String,
    session_id_full: String,
    session_summary: Option<String>,
    repository: String,
    created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CopilotSubmittedDraft {
    id: String,
    path: String,
    provenance_uri: String,
    session_id: String,
    checkpoint_number: i64,
}

struct ParsedCopilotArgs {
    db_path: Option<PathBuf>,
    repo: Option<String>,
    repo_root: Option<PathBuf>,
    limit: Option<usize>,
    query: Option<String>,
    drafts_dir: Option<PathBuf>,
    project: Option<String>,
    operator: Option<String>,
    tags: Vec<String>,
}

struct RepoContext {
    filter: Option<String>,
    label: String,
    scope: CopilotRepoScope,
}

pub(crate) fn copilot_session_store_from_args(
    args: &[String],
    submitted_at: SystemTime,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let Some(command) = args.first() else {
        return Err(CliError::Usage(
            "copilot subcommand is required".to_string(),
        ));
    };
    let parsed = parse_copilot_args(&args[1..])?;
    let db_path = parsed.db_path()?;

    match command.as_str() {
        "schema-check" => copilot_schema_check_outcome(&db_path),
        "recent" => copilot_recent_outcome(&db_path, &parsed),
        "files" => copilot_files_outcome(&db_path, &parsed),
        "checkpoints" => copilot_checkpoints_outcome(&db_path, &parsed),
        "search" => copilot_search_outcome(&db_path, &parsed),
        "submit-drafts" => copilot_submit_drafts_outcome(&db_path, &parsed, submitted_at),
        other => Err(CliError::Usage(format!(
            "unknown copilot subcommand '{other}'"
        ))),
    }
}

fn copilot_schema_check_outcome(db_path: &Path) -> Result<CopilotSessionStoreOutcome, CliError> {
    let connection = open_read_only(db_path).map_err(CliError::Copilot)?;
    let problems = schema_problems(&connection).map_err(CliError::Copilot)?;
    Ok(CopilotSessionStoreOutcome::SchemaCheck {
        db_path: db_path.display().to_string(),
        schema_ok: problems.is_empty(),
        problems,
    })
}

fn copilot_recent_outcome(
    db_path: &Path,
    parsed: &ParsedCopilotArgs,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let repo = repo_context(parsed);
    let limit = parsed.limit_or(DEFAULT_LIMIT)?;
    let connection = open_checked_store(db_path).map_err(CliError::Copilot)?;
    let sessions =
        list_sessions(&connection, repo.filter.as_deref(), limit).map_err(CliError::Copilot)?;
    let recent_files =
        list_files(&connection, repo.filter.as_deref(), limit).map_err(CliError::Copilot)?;
    Ok(CopilotSessionStoreOutcome::Recent {
        db_path: db_path.display().to_string(),
        repo: repo.label,
        repo_scope: repo.scope,
        count: sessions.len(),
        sessions,
        recent_files,
    })
}

fn copilot_files_outcome(
    db_path: &Path,
    parsed: &ParsedCopilotArgs,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let repo = repo_context(parsed);
    let limit = parsed.limit_or(DEFAULT_LIMIT)?;
    let connection = open_checked_store(db_path).map_err(CliError::Copilot)?;
    let files =
        list_files(&connection, repo.filter.as_deref(), limit).map_err(CliError::Copilot)?;
    Ok(CopilotSessionStoreOutcome::Files {
        db_path: db_path.display().to_string(),
        repo: repo.label,
        repo_scope: repo.scope,
        count: files.len(),
        files,
    })
}

fn copilot_checkpoints_outcome(
    db_path: &Path,
    parsed: &ParsedCopilotArgs,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let repo = repo_context(parsed);
    let limit = parsed.limit_or(DEFAULT_CHECKPOINT_LIMIT)?;
    let connection = open_checked_store(db_path).map_err(CliError::Copilot)?;
    let checkpoints =
        list_checkpoints(&connection, repo.filter.as_deref(), limit).map_err(CliError::Copilot)?;
    Ok(CopilotSessionStoreOutcome::Checkpoints {
        db_path: db_path.display().to_string(),
        repo: repo.label,
        repo_scope: repo.scope,
        count: checkpoints.len(),
        checkpoints,
    })
}

fn copilot_search_outcome(
    db_path: &Path,
    parsed: &ParsedCopilotArgs,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let repo = repo_context(parsed);
    let limit = parsed.limit_or(DEFAULT_LIMIT)?;
    let query = parsed.required_query()?;
    let connection = open_checked_store(db_path).map_err(CliError::Copilot)?;
    let results = search_store(&connection, repo.filter.as_deref(), &query, limit)
        .map_err(CliError::Copilot)?;
    Ok(CopilotSessionStoreOutcome::Search {
        db_path: db_path.display().to_string(),
        repo: repo.label,
        repo_scope: repo.scope,
        query,
        count: results.len(),
        results,
    })
}

fn copilot_submit_drafts_outcome(
    db_path: &Path,
    parsed: &ParsedCopilotArgs,
    submitted_at: SystemTime,
) -> Result<CopilotSessionStoreOutcome, CliError> {
    let repo = repo_context(parsed);
    let limit = parsed.limit_or(DEFAULT_CHECKPOINT_LIMIT)?;
    let drafts_dir = parsed.drafts_dir.clone().unwrap_or_else(|| {
        let default_cfg = LibrarianConfig::default();
        default_cfg.drafts_dir
    });
    let connection = open_checked_store(db_path).map_err(CliError::Copilot)?;
    let checkpoints =
        list_checkpoints(&connection, repo.filter.as_deref(), limit).map_err(CliError::Copilot)?;
    let drafts = submit_checkpoint_drafts(
        &checkpoints,
        drafts_dir,
        parsed,
        repo.filter.as_deref(),
        submitted_at,
    )?;
    Ok(CopilotSessionStoreOutcome::DraftsSubmitted {
        db_path: db_path.display().to_string(),
        repo: repo.label,
        repo_scope: repo.scope,
        submitted: drafts.len(),
        drafts,
    })
}

impl ParsedCopilotArgs {
    fn db_path(&self) -> Result<PathBuf, CliError> {
        self.db_path
            .clone()
            .or_else(default_db_path)
            .ok_or_else(|| CliError::Usage("--db is required when HOME is unavailable".to_string()))
    }

    fn limit_or(&self, default: usize) -> Result<usize, CliError> {
        let limit = self.limit.unwrap_or(default);
        if limit == 0 {
            return Err(CliError::Usage(
                "--limit must be greater than 0".to_string(),
            ));
        }
        if limit > MAX_LIMIT {
            return Err(CliError::Usage(format!("--limit must be <= {MAX_LIMIT}")));
        }
        Ok(limit)
    }

    fn required_query(&self) -> Result<String, CliError> {
        let Some(query) = self.query.clone() else {
            return Err(CliError::Usage("--query is required".to_string()));
        };
        if query.trim().is_empty() {
            return Err(CliError::Usage("--query must not be empty".to_string()));
        }
        Ok(query)
    }
}

fn parse_copilot_args(args: &[String]) -> Result<ParsedCopilotArgs, CliError> {
    let mut parsed = ParsedCopilotArgs {
        db_path: None,
        repo: None,
        repo_root: None,
        limit: None,
        query: None,
        drafts_dir: None,
        project: None,
        operator: None,
        tags: Vec::new(),
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => {
                parsed.db_path = Some(take_copilot_value(args, &mut i, "--db")?.into());
            }
            "--repo" => {
                parsed.repo = Some(take_copilot_value(args, &mut i, "--repo")?);
            }
            "--repo-root" => {
                parsed.repo_root = Some(take_copilot_value(args, &mut i, "--repo-root")?.into());
            }
            "--limit" => {
                let value = take_copilot_value(args, &mut i, "--limit")?;
                parsed.limit = Some(value.parse::<usize>().map_err(|_| {
                    CliError::Usage(format!("--limit must be an integer: {value}"))
                })?);
            }
            "--query" => {
                parsed.query = Some(take_copilot_value(args, &mut i, "--query")?);
            }
            "--drafts-dir" => {
                parsed.drafts_dir = Some(take_copilot_value(args, &mut i, "--drafts-dir")?.into());
            }
            "--project" => {
                parsed.project = Some(take_copilot_value(args, &mut i, "--project")?);
            }
            "--operator" => {
                parsed.operator = Some(take_copilot_value(args, &mut i, "--operator")?);
            }
            "--tag" => {
                parsed.tags.push(take_copilot_value(args, &mut i, "--tag")?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown copilot option '{other}'")));
            }
        }
        i += 1;
    }

    Ok(parsed)
}

fn take_copilot_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, CliError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("{flag} requires a value")))
}

fn default_db_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| {
            PathBuf::from(home)
                .join(".copilot")
                .join("session-store.db")
        })
}

fn repo_context(parsed: &ParsedCopilotArgs) -> RepoContext {
    if let Some(repo) = parsed.repo.as_deref() {
        if repo == "all" {
            return RepoContext {
                filter: None,
                label: "all".to_string(),
                scope: CopilotRepoScope::All,
            };
        }
        return RepoContext {
            filter: Some(repo.to_string()),
            label: repo.to_string(),
            scope: CopilotRepoScope::Scoped,
        };
    }

    let root = parsed
        .repo_root
        .clone()
        .unwrap_or_else(|| match std::env::current_dir() {
            Ok(path) => path,
            Err(_) => PathBuf::from("."),
        });
    if let Some(repo) = detect_repo_from_root(&root) {
        return RepoContext {
            filter: Some(repo.clone()),
            label: repo,
            scope: CopilotRepoScope::Scoped,
        };
    }

    RepoContext {
        filter: None,
        label: "all".to_string(),
        scope: CopilotRepoScope::Undetected,
    }
}

fn open_checked_store(db_path: &Path) -> Result<Connection, CopilotSessionStoreError> {
    let connection = open_read_only(db_path)?;
    ensure_schema(&connection)?;
    Ok(connection)
}

fn open_read_only(db_path: &Path) -> Result<Connection, CopilotSessionStoreError> {
    if !db_path.is_file() {
        return Err(CopilotSessionStoreError::MissingDatabase {
            path: db_path.to_path_buf(),
        });
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(db_path, flags).map_err(|source| {
        CopilotSessionStoreError::Open {
            path: db_path.to_path_buf(),
            source,
        }
    })?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|source| CopilotSessionStoreError::Open {
            path: db_path.to_path_buf(),
            source,
        })?;
    connection
        .execute_batch("PRAGMA query_only = ON;")
        .map_err(|source| CopilotSessionStoreError::Open {
            path: db_path.to_path_buf(),
            source,
        })?;
    Ok(connection)
}

fn ensure_schema(connection: &Connection) -> Result<(), CopilotSessionStoreError> {
    let problems = schema_problems(connection)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(CopilotSessionStoreError::SchemaDrift { problems })
    }
}

fn schema_problems(connection: &Connection) -> Result<Vec<String>, CopilotSessionStoreError> {
    let mut problems = Vec::new();
    for table in EXPECTED_SCHEMA {
        let actual = table_columns(connection, table.name)?;
        if actual.is_empty() {
            problems.push(format!("missing table: {}", table.name));
            continue;
        }
        let missing: Vec<&str> = table
            .columns
            .iter()
            .copied()
            .filter(|column| !actual.contains(*column))
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "{}: missing columns {}",
                table.name,
                missing.join(", ")
            ));
        }
    }
    Ok(problems)
}

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<BTreeSet<String>, CopilotSessionStoreError> {
    let sql = format!("PRAGMA table_info({table_name})");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|source| map_schema_error("prepare table_info", source))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| map_schema_error("query table_info", source))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row.map_err(|source| map_schema_error("read table_info", source))?);
    }
    Ok(columns)
}

fn list_sessions(
    connection: &Connection,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<CopilotSession>, CopilotSessionStoreError> {
    let limit = limit_i64(limit);
    if let Some(repo) = repo {
        let mut statement = connection
            .prepare(
                "SELECT s.id, s.repository, s.branch, s.summary, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id) AS turns_count,
                        (SELECT COUNT(*) FROM session_files f WHERE f.session_id = s.id) AS files_count
                 FROM sessions s
                 WHERE s.repository = ?1
                 ORDER BY COALESCE(s.updated_at, s.created_at) DESC
                 LIMIT ?2",
            )
            .map_err(|source| map_query_error("prepare recent sessions", source))?;
        let rows = statement
            .query_map(params![repo, limit], session_from_row)
            .map_err(|source| map_query_error("query recent sessions", source))?;
        collect_rows(rows, "read recent sessions")
    } else {
        let mut statement = connection
            .prepare(
                "SELECT s.id, s.repository, s.branch, s.summary, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM turns t WHERE t.session_id = s.id) AS turns_count,
                        (SELECT COUNT(*) FROM session_files f WHERE f.session_id = s.id) AS files_count
                 FROM sessions s
                 ORDER BY COALESCE(s.updated_at, s.created_at) DESC
                 LIMIT ?1",
            )
            .map_err(|source| map_query_error("prepare recent sessions", source))?;
        let rows = statement
            .query_map(params![limit], session_from_row)
            .map_err(|source| map_query_error("query recent sessions", source))?;
        collect_rows(rows, "read recent sessions")
    }
}

fn list_files(
    connection: &Connection,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<CopilotFile>, CopilotSessionStoreError> {
    let limit = limit_i64(limit);
    if let Some(repo) = repo {
        let mut statement = connection
            .prepare(
                "SELECT sf.file_path, sf.tool_name, sf.first_seen_at,
                        sf.session_id, s.summary AS session_summary, s.repository
                 FROM session_files sf
                 JOIN sessions s ON s.id = sf.session_id
                 WHERE s.repository = ?1
                 ORDER BY sf.first_seen_at DESC
                 LIMIT ?2",
            )
            .map_err(|source| map_query_error("prepare recent files", source))?;
        let rows = statement
            .query_map(params![repo, limit], file_from_row)
            .map_err(|source| map_query_error("query recent files", source))?;
        collect_rows(rows, "read recent files")
    } else {
        let mut statement = connection
            .prepare(
                "SELECT sf.file_path, sf.tool_name, sf.first_seen_at,
                        sf.session_id, s.summary AS session_summary, s.repository
                 FROM session_files sf
                 JOIN sessions s ON s.id = sf.session_id
                 ORDER BY sf.first_seen_at DESC
                 LIMIT ?1",
            )
            .map_err(|source| map_query_error("prepare recent files", source))?;
        let rows = statement
            .query_map(params![limit], file_from_row)
            .map_err(|source| map_query_error("query recent files", source))?;
        collect_rows(rows, "read recent files")
    }
}

fn list_checkpoints(
    connection: &Connection,
    repo: Option<&str>,
    limit: usize,
) -> Result<Vec<CopilotCheckpoint>, CopilotSessionStoreError> {
    let limit = limit_i64(limit);
    if let Some(repo) = repo {
        let mut statement = connection
            .prepare(
                "SELECT c.checkpoint_number, c.title, c.overview, c.created_at,
                        c.session_id, s.summary AS session_summary, s.repository
                 FROM checkpoints c
                 JOIN sessions s ON s.id = c.session_id
                 WHERE s.repository = ?1
                 ORDER BY c.created_at DESC
                 LIMIT ?2",
            )
            .map_err(|source| map_query_error("prepare checkpoints", source))?;
        let rows = statement
            .query_map(params![repo, limit], checkpoint_from_row)
            .map_err(|source| map_query_error("query checkpoints", source))?;
        collect_rows(rows, "read checkpoints")
    } else {
        let mut statement = connection
            .prepare(
                "SELECT c.checkpoint_number, c.title, c.overview, c.created_at,
                        c.session_id, s.summary AS session_summary, s.repository
                 FROM checkpoints c
                 JOIN sessions s ON s.id = c.session_id
                 ORDER BY c.created_at DESC
                 LIMIT ?1",
            )
            .map_err(|source| map_query_error("prepare checkpoints", source))?;
        let rows = statement
            .query_map(params![limit], checkpoint_from_row)
            .map_err(|source| map_query_error("query checkpoints", source))?;
        collect_rows(rows, "read checkpoints")
    }
}

fn search_store(
    connection: &Connection,
    repo: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<CopilotSearchResult>, CopilotSessionStoreError> {
    let pattern = format!("%{}%", escape_like(query));
    let limit = limit_i64(limit);
    let mut results = search_turns(connection, repo, &pattern, limit)?;
    if results.len() < usize::try_from(limit).unwrap_or(usize::MAX) {
        let remaining = limit - i64::try_from(results.len()).unwrap_or(limit);
        results.extend(search_files(connection, repo, &pattern, remaining)?);
    }
    results.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(results)
}

fn search_turns(
    connection: &Connection,
    repo: Option<&str>,
    pattern: &str,
    limit: i64,
) -> Result<Vec<CopilotSearchResult>, CopilotSessionStoreError> {
    if let Some(repo) = repo {
        let mut statement = connection
            .prepare(
                "SELECT 'turn' AS source_type,
                        TRIM(COALESCE(t.user_message, '') || '\n' || COALESCE(t.assistant_response, '')) AS excerpt,
                        t.session_id, s.summary AS session_summary, s.repository, s.created_at
                 FROM turns t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE s.repository = ?1
                   AND (t.user_message LIKE ?2 ESCAPE '\\'
                        OR t.assistant_response LIKE ?2 ESCAPE '\\'
                        OR s.summary LIKE ?2 ESCAPE '\\')
                 ORDER BY t.timestamp DESC
                 LIMIT ?3",
            )
            .map_err(|source| map_query_error("prepare turn search", source))?;
        let rows = statement
            .query_map(params![repo, pattern, limit], search_result_from_row)
            .map_err(|source| map_query_error("query turn search", source))?;
        collect_rows(rows, "read turn search")
    } else {
        let mut statement = connection
            .prepare(
                "SELECT 'turn' AS source_type,
                        TRIM(COALESCE(t.user_message, '') || '\n' || COALESCE(t.assistant_response, '')) AS excerpt,
                        t.session_id, s.summary AS session_summary, s.repository, s.created_at
                 FROM turns t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE t.user_message LIKE ?1 ESCAPE '\\'
                    OR t.assistant_response LIKE ?1 ESCAPE '\\'
                    OR s.summary LIKE ?1 ESCAPE '\\'
                 ORDER BY t.timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|source| map_query_error("prepare turn search", source))?;
        let rows = statement
            .query_map(params![pattern, limit], search_result_from_row)
            .map_err(|source| map_query_error("query turn search", source))?;
        collect_rows(rows, "read turn search")
    }
}

fn search_files(
    connection: &Connection,
    repo: Option<&str>,
    pattern: &str,
    limit: i64,
) -> Result<Vec<CopilotSearchResult>, CopilotSessionStoreError> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    if let Some(repo) = repo {
        let mut statement = connection
            .prepare(
                "SELECT 'file' AS source_type,
                        sf.file_path || COALESCE(' (' || sf.tool_name || ')', '') AS excerpt,
                        sf.session_id, s.summary AS session_summary, s.repository, s.created_at
                 FROM session_files sf
                 JOIN sessions s ON s.id = sf.session_id
                 WHERE s.repository = ?1
                   AND sf.file_path LIKE ?2 ESCAPE '\\'
                 ORDER BY sf.first_seen_at DESC
                 LIMIT ?3",
            )
            .map_err(|source| map_query_error("prepare file search", source))?;
        let rows = statement
            .query_map(params![repo, pattern, limit], search_result_from_row)
            .map_err(|source| map_query_error("query file search", source))?;
        collect_rows(rows, "read file search")
    } else {
        let mut statement = connection
            .prepare(
                "SELECT 'file' AS source_type,
                        sf.file_path || COALESCE(' (' || sf.tool_name || ')', '') AS excerpt,
                        sf.session_id, s.summary AS session_summary, s.repository, s.created_at
                 FROM session_files sf
                 JOIN sessions s ON s.id = sf.session_id
                 WHERE sf.file_path LIKE ?1 ESCAPE '\\'
                 ORDER BY sf.first_seen_at DESC
                 LIMIT ?2",
            )
            .map_err(|source| map_query_error("prepare file search", source))?;
        let rows = statement
            .query_map(params![pattern, limit], search_result_from_row)
            .map_err(|source| map_query_error("query file search", source))?;
        collect_rows(rows, "read file search")
    }
}

fn submit_checkpoint_drafts(
    checkpoints: &[CopilotCheckpoint],
    drafts_dir: PathBuf,
    parsed: &ParsedCopilotArgs,
    scoped_repo: Option<&str>,
    submitted_at: SystemTime,
) -> Result<Vec<CopilotSubmittedDraft>, CliError> {
    let store = DraftStore::new(drafts_dir);
    let mut submitted = Vec::new();
    for checkpoint in checkpoints {
        let provenance_uri = format!(
            "copilot-session-store://session/{}/checkpoint/{}",
            checkpoint.session_id_full, checkpoint.checkpoint_number
        );
        let mut metadata =
            DraftMetadata::new(DraftSourceSurface::CopilotSessionStore, submitted_at);
        metadata.source_agent = Some("copilot".to_string());
        metadata.source_project = parsed
            .project
            .clone()
            .or_else(|| scoped_repo.map(ToOwned::to_owned));
        metadata.operator.clone_from(&parsed.operator);
        metadata.provenance_uri = Some(provenance_uri.clone());
        metadata.context_tags.clone_from(&parsed.tags);
        metadata.context_tags.push("copilot".to_string());
        metadata.context_tags.push("session_store".to_string());

        let draft = Draft::with_metadata(checkpoint_draft_text(checkpoint), metadata);
        let path = store.submit(&draft)?;
        submitted.push(CopilotSubmittedDraft {
            id: draft.id().to_hex(),
            path: path.display().to_string(),
            provenance_uri,
            session_id: checkpoint.session_id_full.clone(),
            checkpoint_number: checkpoint.checkpoint_number,
        });
    }
    Ok(submitted)
}

fn checkpoint_draft_text(checkpoint: &CopilotCheckpoint) -> String {
    format!(
        "Copilot session-store checkpoint\nRepository: {}\nSession: {}\nCheckpoint: {}\nTitle: {}\nCreated at: {}\n\nOverview:\n{}\n\nSession summary:\n{}\n",
        checkpoint.repository,
        checkpoint.session_id_full,
        checkpoint.checkpoint_number,
        checkpoint.title.as_deref().unwrap_or(""),
        checkpoint.created_at.as_deref().unwrap_or(""),
        checkpoint.overview.as_deref().unwrap_or(""),
        checkpoint.session_summary.as_deref().unwrap_or("")
    )
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<CopilotSession> {
    let id: String = row.get("id")?;
    Ok(CopilotSession {
        id_short: short_id(&id),
        id_full: id,
        repository: row.get("repository")?,
        branch: row.get("branch")?,
        summary: row.get("summary")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        turns_count: row.get("turns_count")?,
        files_count: row.get("files_count")?,
    })
}

fn file_from_row(row: &Row<'_>) -> rusqlite::Result<CopilotFile> {
    let session_id: String = row.get("session_id")?;
    Ok(CopilotFile {
        file_path: row.get("file_path")?,
        tool_name: row.get("tool_name")?,
        first_seen_at: row.get("first_seen_at")?,
        session_id: short_id(&session_id),
        session_id_full: session_id,
        session_summary: row.get("session_summary")?,
        repository: row.get("repository")?,
    })
}

fn checkpoint_from_row(row: &Row<'_>) -> rusqlite::Result<CopilotCheckpoint> {
    let session_id: String = row.get("session_id")?;
    Ok(CopilotCheckpoint {
        checkpoint_number: row.get("checkpoint_number")?,
        title: row.get("title")?,
        overview: row.get("overview")?,
        created_at: row.get("created_at")?,
        session_id: short_id(&session_id),
        session_id_full: session_id,
        session_summary: row.get("session_summary")?,
        repository: row.get("repository")?,
    })
}

fn search_result_from_row(row: &Row<'_>) -> rusqlite::Result<CopilotSearchResult> {
    let session_id: String = row.get("session_id")?;
    Ok(CopilotSearchResult {
        source_type: row.get("source_type")?,
        excerpt: truncate_chars(&row.get::<_, String>("excerpt")?, 400),
        session_id: short_id(&session_id),
        session_id_full: session_id,
        session_summary: row.get("session_summary")?,
        repository: row.get("repository")?,
        created_at: row.get("created_at")?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&Row<'_>) -> rusqlite::Result<T>>,
    operation: &'static str,
) -> Result<Vec<T>, CopilotSessionStoreError> {
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(|source| map_query_error(operation, source))?);
    }
    Ok(values)
}

fn limit_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn map_schema_error(operation: &'static str, source: rusqlite::Error) -> CopilotSessionStoreError {
    if is_locked_error(&source) {
        CopilotSessionStoreError::Locked { operation, source }
    } else {
        CopilotSessionStoreError::SchemaCheck { operation, source }
    }
}

fn map_query_error(operation: &'static str, source: rusqlite::Error) -> CopilotSessionStoreError {
    if is_locked_error(&source) {
        CopilotSessionStoreError::Locked { operation, source }
    } else {
        CopilotSessionStoreError::Query { operation, source }
    }
}

fn is_locked_error(source: &rusqlite::Error) -> bool {
    if let rusqlite::Error::SqliteFailure(error, _) = source {
        matches!(
            error.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        )
    } else {
        let text = source.to_string().to_ascii_lowercase();
        text.contains("locked") || text.contains("busy")
    }
}

fn detect_repo_from_root(root: &Path) -> Option<String> {
    let config_path = find_git_config(root)?;
    let config = fs::read_to_string(config_path).ok()?;
    let mut in_origin = false;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_origin = matches!(
                trimmed,
                "[remote \"origin\"]" | "[remote 'origin']" | "[remote origin]"
            );
            continue;
        }
        if in_origin {
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            if key.trim() == "url" {
                return normalize_git_remote(value.trim());
            }
        }
    }
    None
}

fn find_git_config(root: &Path) -> Option<PathBuf> {
    let mut current = if root.is_file() {
        root.parent()?.to_path_buf()
    } else {
        root.to_path_buf()
    };
    loop {
        let candidate = current.join(".git").join("config");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn normalize_git_remote(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches(".git").trim_end_matches('/');
    let repo_path = if let Some((_, path)) = trimmed.split_once("://") {
        path.split_once('/')?.1
    } else if let Some((_, path)) = trimmed.split_once(':') {
        path
    } else {
        trimmed
    };
    let repo_path = repo_path.trim_matches('/');
    if repo_path.matches('/').count() >= 1 {
        Some(repo_path.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn create_fixture(path: &Path) -> Result<(), Box<dyn Error>> {
        let connection = Connection::open(path)?;
        create_fixture_schema(&connection)?;
        seed_fixture(&connection)?;
        Ok(())
    }

    fn create_fixture_schema(connection: &Connection) -> Result<(), Box<dyn Error>> {
        connection.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                repository TEXT,
                branch TEXT,
                summary TEXT,
                created_at TEXT,
                updated_at TEXT
            );
            CREATE TABLE turns (
                session_id TEXT,
                turn_index INTEGER,
                user_message TEXT,
                assistant_response TEXT,
                timestamp TEXT
            );
            CREATE TABLE session_files (
                session_id TEXT,
                file_path TEXT,
                tool_name TEXT,
                turn_index INTEGER,
                first_seen_at TEXT
            );
            CREATE TABLE session_refs (
                session_id TEXT,
                ref_type TEXT,
                ref_value TEXT,
                turn_index INTEGER,
                created_at TEXT
            );
            CREATE TABLE checkpoints (
                session_id TEXT,
                checkpoint_number INTEGER,
                title TEXT,
                overview TEXT,
                created_at TEXT
            );",
        )?;
        Ok(())
    }

    fn seed_fixture(connection: &Connection) -> Result<(), Box<dyn Error>> {
        connection.execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "session-mimir-0001",
                "buildepicshit/Mimir",
                "main",
                "Mimir Copilot session summary",
                "2026-04-26T10:00:00Z",
                "2026-04-26T11:00:00Z"
            ],
        )?;
        connection.execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "session-other-0001",
                "buildepicshit/Other",
                "main",
                "Other repo summary",
                "2026-04-26T09:00:00Z",
                "2026-04-26T09:30:00Z"
            ],
        )?;
        connection.execute(
            "INSERT INTO turns VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "session-mimir-0001",
                1_i64,
                "How should Copilot recall native session files?",
                "Use read-only SQLite recall with schema checks.",
                "2026-04-26T10:05:00Z"
            ],
        )?;
        connection.execute(
            "INSERT INTO session_files VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "session-mimir-0001",
                "/work/Mimir/STATUS.md",
                "edit",
                1_i64,
                "2026-04-26T10:10:00Z"
            ],
        )?;
        connection.execute(
            "INSERT INTO checkpoints VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "session-mimir-0001",
                1_i64,
                "Copilot adapter checkpoint",
                "Read-only recall should become untrusted Mimir draft input.",
                "2026-04-26T10:20:00Z"
            ],
        )?;
        connection.execute(
            "INSERT INTO checkpoints VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "session-other-0001",
                1_i64,
                "Other checkpoint",
                "This checkpoint must not appear in Mimir-scoped output.",
                "2026-04-26T09:20:00Z"
            ],
        )?;
        Ok(())
    }

    #[test]
    fn schema_check_reports_missing_database() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("missing.db");
        let command = args(&["schema-check", "--db", &db.display().to_string()]);

        match copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH) {
            Err(CliError::Copilot(CopilotSessionStoreError::MissingDatabase { path })) => {
                assert_eq!(path, db);
            }
            other => {
                assert!(
                    matches!(
                        other,
                        Err(CliError::Copilot(
                            CopilotSessionStoreError::MissingDatabase { .. }
                        ))
                    ),
                    "expected missing database error, got {other:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn schema_check_reports_schema_drift_without_querying() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("session-store.db");
        let connection = Connection::open(&db)?;
        connection.execute_batch("CREATE TABLE sessions (id TEXT);")?;
        drop(connection);

        let command = args(&["schema-check", "--db", &db.display().to_string()]);
        let outcome = copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH)?;

        match outcome {
            CopilotSessionStoreOutcome::SchemaCheck {
                schema_ok,
                problems,
                ..
            } => {
                assert!(!schema_ok);
                assert!(problems.iter().any(|problem| problem.contains("sessions")));
                assert!(problems
                    .iter()
                    .any(|problem| problem.contains("missing table: turns")));
            }
            other => {
                assert!(
                    matches!(other, CopilotSessionStoreOutcome::SchemaCheck { .. }),
                    "expected schema-check outcome, got {other:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn recent_sessions_are_scoped_to_explicit_repo() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("session-store.db");
        create_fixture(&db)?;
        let command = args(&[
            "recent",
            "--db",
            &db.display().to_string(),
            "--repo",
            "buildepicshit/Mimir",
            "--limit",
            "10",
        ]);

        let outcome = copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH)?;

        match outcome {
            CopilotSessionStoreOutcome::Recent {
                repo,
                repo_scope,
                sessions,
                recent_files,
                ..
            } => {
                assert_eq!(repo, "buildepicshit/Mimir");
                assert_eq!(repo_scope, CopilotRepoScope::Scoped);
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id_full, "session-mimir-0001");
                assert_eq!(recent_files.len(), 1);
                assert_eq!(recent_files[0].repository, "buildepicshit/Mimir");
            }
            other => {
                assert!(
                    matches!(other, CopilotSessionStoreOutcome::Recent { .. }),
                    "expected recent outcome, got {other:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn search_reads_turns_and_files_as_untrusted_recall() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("session-store.db");
        create_fixture(&db)?;
        let command = args(&[
            "search",
            "--db",
            &db.display().to_string(),
            "--repo",
            "buildepicshit/Mimir",
            "--query",
            "STATUS.md",
        ]);

        let outcome = copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH)?;

        match outcome {
            CopilotSessionStoreOutcome::Search { results, .. } => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].source_type, "file");
                assert!(results[0].excerpt.contains("STATUS.md"));
            }
            other => {
                assert!(
                    matches!(other, CopilotSessionStoreOutcome::Search { .. }),
                    "expected search outcome, got {other:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn submit_drafts_stages_copilot_checkpoint_provenance() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("session-store.db");
        let drafts = tmp.path().join("drafts");
        create_fixture(&db)?;
        let command = args(&[
            "submit-drafts",
            "--db",
            &db.display().to_string(),
            "--drafts-dir",
            &drafts.display().to_string(),
            "--repo",
            "buildepicshit/Mimir",
            "--operator",
            "AlainDor",
            "--tag",
            "fixture",
        ]);

        let outcome = copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH)?;

        match outcome {
            CopilotSessionStoreOutcome::DraftsSubmitted {
                submitted, drafts, ..
            } => {
                assert_eq!(submitted, 1);
                assert_eq!(drafts.len(), 1);
                let saved = fs::read_to_string(&drafts[0].path)?;
                assert!(saved.contains("\"source_surface\": \"copilot_session_store\""));
                assert!(saved.contains("\"source_agent\": \"copilot\""));
                assert!(saved.contains("\"source_project\": \"buildepicshit/Mimir\""));
                assert!(saved.contains("\"operator\": \"AlainDor\""));
                assert!(saved
                    .contains("copilot-session-store://session/session-mimir-0001/checkpoint/1"));
                assert!(
                    saved.contains("Read-only recall should become untrusted Mimir draft input.")
                );
            }
            other => {
                assert!(
                    matches!(other, CopilotSessionStoreOutcome::DraftsSubmitted { .. }),
                    "expected draft submission outcome, got {other:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn queries_fail_safely_when_database_is_locked() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let db = tmp.path().join("session-store.db");
        create_fixture(&db)?;
        let writer = Connection::open(&db)?;
        writer.execute_batch("BEGIN EXCLUSIVE;")?;
        let command = args(&[
            "recent",
            "--db",
            &db.display().to_string(),
            "--repo",
            "buildepicshit/Mimir",
        ]);

        match copilot_session_store_from_args(&command, SystemTime::UNIX_EPOCH) {
            Err(CliError::Copilot(CopilotSessionStoreError::Locked { .. })) => {}
            other => {
                assert!(
                    matches!(
                        other,
                        Err(CliError::Copilot(CopilotSessionStoreError::Locked { .. }))
                    ),
                    "expected locked database error, got {other:?}"
                );
            }
        }
        writer.execute_batch("ROLLBACK;")?;
        Ok(())
    }

    #[test]
    fn repo_detection_reads_git_origin_without_spawning_git() -> Result<(), Box<dyn Error>> {
        let tmp = tempfile::tempdir()?;
        let git_dir = tmp.path().join(".git");
        fs::create_dir(&git_dir)?;
        fs::write(
            git_dir.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:buildepicshit/Mimir.git\n",
        )?;

        assert_eq!(
            detect_repo_from_root(tmp.path()).as_deref(),
            Some("buildepicshit/Mimir")
        );
        Ok(())
    }
}
