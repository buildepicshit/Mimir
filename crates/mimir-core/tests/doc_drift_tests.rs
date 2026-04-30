//! Drift gates that prevent the doc-vs-code drift class from
//! regressing. Companion to the in-source `DefaultToolGroupsTests`
//! pattern (here: in-tree integration tests over the *repository
//! state* rather than the runtime state).
//!
//! Each test exists to catch a specific past failure. When a test
//! fails, its message names the v1.1 fresh-assessment finding it
//! exists to prevent — so a future contributor can decide whether
//! the regression is intentional (then update the test) or
//! accidental (then revert the offending edit).
//!
//! Coverage:
//!
//! - [`status_banner_consistency`] — every spec under
//!   `docs/concepts/` carries a parseable `> **Status: <state>`
//!   first-paragraph banner; once `Cargo.toml [workspace.package]
//!   version` ≥ 1.0.0 every spec must be `authoritative`. (v1.1
//!   audit doc-finding F4 / Rolls-Royce gap row 11.)
//!
//! - [`readme_no_design_phase_lies`] — the front-door `README.md`
//!   may not claim "no production code yet" / "to be authored" /
//!   "Private while it is design-phase" once the crate tree is
//!   non-trivially populated. (Doc finding F1 — the single biggest
//!   onboarding blocker the audit identified.)
//!
//! - [`version_consistency`] — root `Cargo.toml [workspace.package]`
//!   version matches the latest git tag. Skipped when no tags exist
//!   (project is pre-tag); load-bearing the moment the release
//!   pipeline lands. (Roadmap Phase 4 + Rolls-Royce gap row 11.)
//!
//! - [`mimir_core_workspace_dep_version_consistency`] — the
//!   `[workspace.dependencies] mimir_core` version pin must equal
//!   `[workspace.package].version`. The release pipeline at
//!   `.github/workflows/release.yml` requires this match for
//!   `cargo publish -p mimir-cli` to resolve `mimir_core` from
//!   crates.io once published. (Roadmap Phase 1.5.)

// Integration-test binary; idiomatic Result-assertion patterns use
// unwrap/expect/panic. The workspace-level denies those for library
// correctness (PRINCIPLES.md § 7); relax here per the same convention
// `tests/properties.rs` uses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::manual_assert
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Walk up from this test crate's manifest dir until we find the
/// repo root, identified by the presence of both the workspace
/// `Cargo.toml` (with `[workspace]`) and the `LICENSE` file. Panics
/// (test failure) if the walk reaches the filesystem root without
/// finding it.
fn find_repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is the dir containing the test crate's
    // Cargo.toml — i.e., crates/mimir-core. Walk up.
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("LICENSE").exists() {
            // Quick check: the Cargo.toml at this level should declare a workspace.
            if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml")) {
                if text.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            panic!("could not find Mimir repo root walking up from CARGO_MANIFEST_DIR");
        }
    }
}

/// Read `[workspace.package] version = "X.Y.Z"` from the root
/// Cargo.toml.
fn read_workspace_version(repo_root: &Path) -> String {
    let cargo_toml = repo_root.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_toml)
        .unwrap_or_else(|e| panic!("read {}: {e}", cargo_toml.display()));
    // Naïve TOML scan — sufficient for a single-line field. Avoids
    // pulling toml-edit into mimir_core's dev-deps just for tests.
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version") {
            // [workspace.package] version = "0.1.0"
            if let Some(eq_pos) = rest.find('=') {
                let value = rest[eq_pos + 1..].trim();
                if let Some(unquoted) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    return unquoted.to_string();
                }
            }
        }
    }
    panic!(
        "could not find `version = \"...\"` in {}",
        cargo_toml.display()
    );
}

/// Best-effort parse of `M.m.p` semver. Returns `None` for
/// pre-release / metadata variants we don't currently use; tests
/// that gate on version comparison treat `None` as "skip the
/// stricter check."
fn parse_major(version: &str) -> Option<u64> {
    version.split('.').next()?.parse::<u64>().ok()
}

// ----------------------------------------------------------------
// Test 1 — status banner consistency
// ----------------------------------------------------------------

#[test]
fn status_banner_consistency() {
    let repo_root = find_repo_root();
    let concepts = repo_root.join("docs").join("concepts");
    let entries =
        fs::read_dir(&concepts).unwrap_or_else(|e| panic!("read {}: {e}", concepts.display()));

    let version = read_workspace_version(&repo_root);
    let major = parse_major(&version);
    let stable_required = major.unwrap_or(0) >= 1;

    let mut violations: Vec<String> = Vec::new();
    let mut spec_count = 0;

    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        // README.md is the concepts INDEX, not a spec. Skip.
        if name.eq_ignore_ascii_case("README.md") {
            continue;
        }
        spec_count += 1;

        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        // Expected banner format (sampled across all 14 specs):
        //   > **Status: <state>...**
        // Where <state> is the first whitespace- or punctuation-
        // delimited word after "Status: ".
        let state = parse_status_banner(&text);
        match state {
            None => {
                violations.push(format!(
                    "{name}: missing or unparseable `> **Status: <state>` banner \
                     in the first paragraph"
                ));
            }
            Some(s) if stable_required && s != "authoritative" => {
                violations.push(format!(
                    "{name}: status `{s}` (project version {version} is stable; \
                     every spec must be `authoritative`)"
                ));
            }
            Some(_) => { /* OK */ }
        }
    }

    assert!(
        spec_count > 0,
        "no docs/concepts/*.md specs found at {} — directory layout regressed?",
        concepts.display()
    );
    assert!(
        violations.is_empty(),
        "status banner drift detected ({} violations across {} specs):\n  - {}",
        violations.len(),
        spec_count,
        violations.join("\n  - "),
    );
}

/// Extract the `<state>` word from the first `> **Status: <state>`
/// banner. Returns `None` if no such banner is found in the first
/// non-blank-non-heading line block. State is the first
/// whitespace-delimited token after `Status:`, with leading dashes
/// and trailing punctuation trimmed.
fn parse_status_banner(text: &str) -> Option<String> {
    let needle = "Status:";
    for line in text.lines().take(20) {
        // Limit search to head — banner is always in first paragraph.
        let trimmed = line.trim_start();
        if !trimmed.starts_with('>') {
            continue;
        }
        if let Some(idx) = trimmed.find(needle) {
            let after = &trimmed[idx + needle.len()..];
            let state = after
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == '*' || c == '.' || c == ',')
                .find(|s| !s.is_empty())?
                .trim_matches('—');
            if state.is_empty() {
                return None;
            }
            return Some(state.to_string());
        }
    }
    None
}

// ----------------------------------------------------------------
// Test 2 — README must not lie about implementation state
// ----------------------------------------------------------------

#[test]
fn readme_no_design_phase_lies() {
    let repo_root = find_repo_root();
    let readme = repo_root.join("README.md");
    let text =
        fs::read_to_string(&readme).unwrap_or_else(|e| panic!("read {}: {e}", readme.display()));

    // Sentinel that the crate tree is non-trivially populated. If
    // these counts ever drop to zero, this test passes vacuously
    // (and someone deleted the implementation, which is a much
    // bigger problem the test suite will surface elsewhere).
    let total_lines = count_rust_loc_in_crates(&repo_root);
    if total_lines < 500 {
        // Skip: trivial codebase, README phrasing about "no code yet"
        // would actually be true.
        return;
    }

    let forbidden = [
        // Doc finding F1 (P1) — README claimed design phase / no code.
        "no production code yet",
        // Doc finding F3 (P2) — stale placeholder wording.
        "to be authored",
        // Doc finding F1 — stale "private" framing.
        "private while it is design-phase",
    ];

    let lowered = text.to_lowercase();
    let mut hits: Vec<&str> = Vec::new();
    for needle in forbidden {
        if lowered.contains(needle) {
            hits.push(needle);
        }
    }
    assert!(
        hits.is_empty(),
        "README.md contains stale design-phase claim(s) — the v1.1 fresh-assessment doc \
         finding F1 ('the single biggest onboarding blocker' the audit identified) has \
         regressed.\nForbidden phrases found: {hits:?}\nCrates currently contain {total_lines} \
         lines of Rust source — the project is past design phase."
    );
}

/// Count total lines across all `.rs` files under `crates/` (excluding
/// `target/` and tests). Used as a heuristic "is there real code yet?"
/// signal that gates the design-phase-lies check.
fn count_rust_loc_in_crates(repo_root: &Path) -> usize {
    let crates_dir = repo_root.join("crates");
    let mut total: usize = 0;
    walk_rs_files(&crates_dir, &mut |path| {
        if path.components().any(|c| c.as_os_str() == "target") {
            return;
        }
        if let Ok(text) = fs::read_to_string(path) {
            total += text.lines().count();
        }
    });
    total
}

fn walk_rs_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, visit);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            visit(&path);
        }
    }
}

// ----------------------------------------------------------------
// Test 3 — Cargo workspace version matches latest git tag
// ----------------------------------------------------------------

#[test]
fn version_consistency() {
    let repo_root = find_repo_root();
    let cargo_version = read_workspace_version(&repo_root);

    // Resolve the latest annotated/lightweight tag, if any. When the
    // repo is pre-tag (Mimir's current state at time of writing),
    // `git describe --tags --abbrev=0` exits non-zero — we treat
    // that as "skip" rather than "fail," because version
    // consistency is only meaningful once a release has been cut.
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("describe")
        .arg("--tags")
        .arg("--abbrev=0")
        .output();
    let Ok(out) = output else {
        // git not available — skip rather than fail (some CI shapes don't ship git).
        return;
    };
    if !out.status.success() {
        // No tags yet. Skip — the gate becomes load-bearing at the
        // moment the release pipeline pushes the first `v*` tag
        // (roadmap Phase 4).
        return;
    }
    let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let tag_version = tag.strip_prefix('v').unwrap_or(&tag);

    assert_eq!(
        tag_version, cargo_version,
        "Cargo workspace version `{cargo_version}` does not match latest git tag \
         `{tag}`. Bump `Cargo.toml [workspace.package].version` and the tag in lockstep \
         (the release pipeline at `.github/workflows/release.yml` enforces this same \
         invariant on tag push — local enforcement here catches drift before push)."
    );
}

// ----------------------------------------------------------------
// Test 4 — workspace.dependencies mimir_core version pin matches
//          workspace.package version
// ----------------------------------------------------------------

#[test]
fn internal_workspace_dep_versions_consistency() {
    let repo_root = find_repo_root();
    let workspace_version = read_workspace_version(&repo_root);

    // Each internal crate that's published to crates.io must be
    // declared in [workspace.dependencies] with both `path` and
    // `version = "=X.Y.Z"`. The release pipeline needs the version
    // pin so `cargo publish -p <downstream>` can resolve the
    // upstream crate from crates.io once it's been published.
    // The exact-match `=` prefix is intentional — these crates are
    // co-released (one cargo publish per crate, all under the same
    // tag) so a `^X.Y.Z` range would let downstream resolve to a
    // future mimir_core that wasn't co-tested with this mimir-cli.
    for internal_dep in &["mimir-core", "mimir-cli", "mimir-librarian"] {
        let dep_version = read_workspace_dep_version(&repo_root, internal_dep)
            .unwrap_or_else(|err| panic!("workspace dep `{internal_dep}`: {err}"));
        let bare = dep_version.trim_start_matches('=');
        assert_eq!(
            bare, workspace_version,
            "[workspace.dependencies] `{internal_dep}` version `{dep_version}` does not \
             match [workspace.package].version `{workspace_version}`. The two must move in \
             lockstep — on every workspace version bump, also update each internal dep pin \
             under [workspace.dependencies] in the same commit (the `=` exact-match prefix \
             is intentional; mimir_core / mimir-cli / mimir-mcp are co-released)."
        );
    }
}

/// Naïve TOML scanner — locates `dep_name = { ..., version = "X.Y.Z", ... }`
/// inside `[workspace.dependencies]` and returns the version string.
/// Sufficient for the single line we care about; avoids a TOML
/// parser dep just for tests.
fn read_workspace_dep_version(repo_root: &Path, dep_name: &str) -> Result<String, String> {
    let cargo_toml = repo_root.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_toml)
        .map_err(|e| format!("read {}: {e}", cargo_toml.display()))?;

    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_block = trimmed == "[workspace.dependencies]";
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(dep_name) {
            // Make sure we matched the full identifier, not a prefix
            // (mimir_core vs. mimir_core_legacy etc.).
            if !rest.starts_with([' ', '=']) {
                continue;
            }
            let rest = rest.trim_start_matches([' ', '=']).trim();
            if let Some(idx) = rest.find("version") {
                let after = &rest[idx + "version".len()..];
                let after = after.trim_start_matches([' ', '=']).trim();
                if let Some(quoted) = after.strip_prefix('"') {
                    if let Some(end) = quoted.find('"') {
                        return Ok(quoted[..end].to_string());
                    }
                }
            }
            return Err(format!(
                "found `{dep_name}` in [workspace.dependencies] but no `version = \"...\"` field"
            ));
        }
    }
    Err(format!(
        "could not find `{dep_name}` in [workspace.dependencies]; expected an entry of the \
         form `{dep_name} = {{ path = \"...\", version = \"=X.Y.Z\" }}`"
    ))
}
