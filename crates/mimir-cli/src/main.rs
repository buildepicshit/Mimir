//! Entry point for the `mimir-cli` read-only inspection tool per
//! `docs/concepts/decoder-tool-contract.md`.
//!
//! Minimal v1 subcommand surface:
//!
//! ```text
//! mimir-cli log     <path>   — stream canonical records as annotated Lisp
//! mimir-cli decode  <path>   — emit re-parseable Lisp for memory records
//! mimir-cli symbols <path>   — list symbols in the reconstructed table
//! mimir-cli verify  <path>   — integrity check with corruption diagnostics
//! mimir-cli parse            — read Lisp from stdin; exit 0 if it parses, 1 if not
//! ```
//!
//! All subcommands are **read-only** (spec § 10 invariant 1). The
//! binary exits `0` for healthy, `1` for corruption signals, `2` for
//! argument / filesystem errors.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};

use mimir_cli::{iso8601_from_millis, load_table_from_log, verify, LispRenderer};
use mimir_core::bind::SymbolTable;
use mimir_core::canonical::{decode_all, CanonicalRecord};
use mimir_core::log::{CanonicalLog, LogBackend};

const USAGE: &str = "\
mimir-cli — read-only inspection for Mimir canonical logs.

Usage:
    mimir-cli log     <path>
    mimir-cli decode  <path>
    mimir-cli symbols <path>
    mimir-cli verify  <path>
    mimir-cli parse

`parse` reads Lisp from stdin and exits 0 on a clean parse, 1 on
parse error (error surfaced on stderr). Useful for corpus validation
and fluency benchmarks.

All other subcommands are read-only over a canonical log; the binary
never writes to the log.
";

fn main() -> ExitCode {
    init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(err) => {
            // `{err:#}` prints the source chain per `anyhow`'s
            // convention so the typed cause survives to the CLI
            // surface rather than being flattened to a leaf message.
            eprintln!("mimir-cli: {err:#}");
            ExitCode::from(2)
        }
    }
}

/// Initialise a stderr subscriber so library-emitted events reach the
/// operator. Filter defaults to `info` with `RUST_LOG` override, per
/// `docs/observability.md`. Install failures are ignored — a subscriber
/// may already be installed by an embedder; we never want tracing
/// setup to take the CLI down.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run(args: &[String]) -> Result<ExitCode> {
    if matches!(args, [flag] if flag == "-h" || flag == "--help") {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if matches!(args, [flag] if flag == "--version") {
        println!("mimir-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    let Some((sub, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    match sub.as_str() {
        "log" => single_path_cmd(rest, cmd_log),
        "decode" => single_path_cmd(rest, cmd_decode),
        "symbols" => single_path_cmd(rest, cmd_symbols),
        "verify" => single_path_cmd(rest, cmd_verify),
        "parse" => cmd_parse(rest),
        other => Err(anyhow!("unknown subcommand '{other}'; see --help")),
    }
}

/// Dispatch a subcommand that takes exactly one path argument.
fn single_path_cmd(args: &[String], f: fn(&Path) -> Result<ExitCode>) -> Result<ExitCode> {
    let [path] = args else {
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    f(Path::new(path))
}

/// `parse` subcommand — read Lisp from stdin and report parse success.
/// Exits `0` on success, `1` on parse error (typed error printed to
/// stderr), `2` on argument misuse (any trailing args).
fn cmd_parse(args: &[String]) -> Result<ExitCode> {
    if !args.is_empty() {
        eprintln!("parse takes no positional arguments; it reads Lisp from stdin");
        eprintln!("{USAGE}");
        return Ok(ExitCode::from(2));
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading stdin")?;
    match mimir_core::parse::parse(&input) {
        Ok(_) => Ok(ExitCode::SUCCESS),
        Err(err) => {
            eprintln!("parse error: {err}");
            Ok(ExitCode::from(1))
        }
    }
}

fn cmd_log(path: &Path) -> Result<ExitCode> {
    let mut log = CanonicalLog::open(path).context("opening canonical log")?;
    let bytes = log.read_all().context("reading canonical log")?;
    let records = decode_all(&bytes).context("decoding canonical log")?;
    for record in records {
        println!("{}", summarize_record(&record));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_decode(path: &Path) -> Result<ExitCode> {
    let table = load_table_from_log(path).context("loading symbol table")?;
    let mut log = CanonicalLog::open(path).context("opening canonical log")?;
    let bytes = log.read_all().context("reading canonical log")?;
    let records = decode_all(&bytes).context("decoding canonical log")?;
    let renderer = LispRenderer::new(&table);
    for record in records {
        match renderer.render_memory(&record) {
            Ok(text) => println!("{text}"),
            Err(mimir_cli::RenderError::NotAMemory) => {} // skip non-memory records
            Err(e) => return Err(e).context("rendering record as Lisp"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_symbols(path: &Path) -> Result<ExitCode> {
    let table = load_table_from_log(path).context("loading symbol table")?;
    print_symbols(&table);
    Ok(ExitCode::SUCCESS)
}

fn cmd_verify(path: &Path) -> Result<ExitCode> {
    use mimir_cli::TailStatus;
    let report = verify(path).context("running integrity check")?;
    println!("records_decoded : {}", report.records_decoded);
    println!("checkpoints     : {}", report.checkpoints);
    println!("memory_records  : {}", report.memory_records);
    println!("symbol_events   : {}", report.symbol_events);
    match &report.tail {
        TailStatus::Clean => println!("tail            : clean"),
        TailStatus::OrphanTail { bytes } => {
            println!("tail            : orphan ({bytes} bytes, recoverable)");
        }
        TailStatus::Corrupt {
            bytes,
            first_decode_error,
        } => {
            println!("tail            : CORRUPT ({bytes} bytes): {first_decode_error}");
        }
    }
    println!("dangling_symbols: {}", report.dangling_symbols);
    // Exit 1 for genuine corruption (dangling symbol refs OR a tail
    // that fails to decode for a non-truncation reason). An orphan
    // tail alone is NOT a corruption signal — `write-protocol.md`
    // § 10 explicitly expects such tails on next open, and they get
    // truncated automatically on `Store::open`.
    if report.tail.is_corrupt() || report.dangling_symbols > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn summarize_record(record: &CanonicalRecord) -> String {
    match record {
        CanonicalRecord::Sem(r) => format!(
            "SEM memory_id={:?} s={:?} p={:?} v={}",
            r.memory_id,
            r.s,
            r.p,
            iso8601_from_millis(r.clocks.valid_at)
        ),
        CanonicalRecord::Epi(r) => format!(
            "EPI memory_id={:?} event_id={:?} at={}",
            r.memory_id,
            r.event_id,
            iso8601_from_millis(r.at_time)
        ),
        CanonicalRecord::Pro(r) => {
            format!("PRO memory_id={:?} rule_id={:?}", r.memory_id, r.rule_id)
        }
        CanonicalRecord::Inf(r) => {
            format!("INF memory_id={:?} s={:?} p={:?}", r.memory_id, r.s, r.p)
        }
        CanonicalRecord::Checkpoint(c) => format!(
            "CHECKPOINT episode_id={:?} at={} memory_count={}",
            c.episode_id,
            iso8601_from_millis(c.at),
            c.memory_count
        ),
        CanonicalRecord::SymbolAlloc(e) => {
            format!("SYMBOL_ALLOC id={:?} name={:?}", e.symbol_id, e.name)
        }
        CanonicalRecord::SymbolAlias(e) => {
            format!("SYMBOL_ALIAS id={:?} alias={:?}", e.symbol_id, e.name)
        }
        CanonicalRecord::SymbolRename(e) => {
            format!("SYMBOL_RENAME id={:?} new={:?}", e.symbol_id, e.name)
        }
        CanonicalRecord::SymbolRetire(e) => {
            format!("SYMBOL_RETIRE id={:?} name={:?}", e.symbol_id, e.name)
        }
        _ => format!("{record:?}"),
    }
}

fn print_symbols(table: &SymbolTable) {
    // Iterate in sorted order of canonical_name for deterministic
    // output (spec § 10 invariant 2).
    let mut entries: Vec<_> = table.iter_entries().collect();
    entries.sort_by(|a, b| a.1.canonical_name.cmp(&b.1.canonical_name));
    for (id, entry) in entries {
        let retired = if entry.retired { " RETIRED" } else { "" };
        let aliases = if entry.aliases.is_empty() {
            String::new()
        } else {
            format!(" aliases={:?}", entry.aliases)
        };
        println!(
            "{id:?} {name:<32} kind={kind:?}{retired}{aliases}",
            name = entry.canonical_name,
            kind = entry.kind
        );
    }
}
