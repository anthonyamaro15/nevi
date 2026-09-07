//! Inventory check for the opt-in test gates (`#[ignore]` tests).
//!
//! Every ignored test is either named by a `cargo test ... --ignored` line in
//! the CI workflow or listed in [`NOT_IN_CI`] with a written reason, so a
//! guard cannot be added and then quietly never run. The reverse is checked
//! too: `cargo test <filter> -- --ignored` passes with zero tests when the
//! filter matches nothing, so a renamed guard would leave CI green while
//! running nothing.

use std::fs;
use std::path::{Path, PathBuf};

const CI_WORKFLOW: &str = ".github/workflows/rust.yml";

/// Ignored tests CI deliberately does not run, each with the reason.
const NOT_IN_CI: &[(&str, &str)] = &[
    (
        "fuzz_key_sequences_extended",
        "20k-seed soak that takes minutes; run before releases",
    ),
    (
        "fuzz_minimize",
        "developer tool driven by NEVI_FUZZ_MINIMIZE, not a pass/fail check",
    ),
];

fn manifest_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Names of every `#[ignore]` test function under src/.
fn ignored_tests() -> Vec<String> {
    let mut files = Vec::new();
    rust_sources(&manifest_path("src"), &mut files);
    files.sort();

    let mut names = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read source file");
        let mut pending_ignore = false;
        for line in text.lines() {
            let line = line.trim_start();
            if line.starts_with("#[ignore") {
                pending_ignore = true;
            } else if pending_ignore && !line.starts_with("#[") {
                if let Some(rest) = line.strip_prefix("fn ") {
                    if let Some(name) = rest.split('(').next() {
                        names.push(name.trim().to_string());
                    }
                }
                pending_ignore = false;
            }
        }
    }
    names
}

/// Test-name filters from every `cargo test ... --ignored` line in CI.
fn ci_ignored_filters() -> Vec<String> {
    let text = fs::read_to_string(manifest_path(CI_WORKFLOW)).expect("read CI workflow");
    let mut filters = Vec::new();
    for line in text.lines() {
        if !line.contains("cargo test") || !line.contains("--ignored") {
            continue;
        }
        let after_test = line
            .split_whitespace()
            .skip_while(|token| *token != "test")
            .skip(1);
        filters.extend(
            after_test
                .filter(|token| !token.starts_with('-'))
                .map(str::to_string),
        );
    }
    filters
}

#[test]
fn every_ignored_test_is_run_by_ci_or_listed_with_a_reason() {
    let tests = ignored_tests();
    let filters = ci_ignored_filters();
    assert!(!tests.is_empty(), "no #[ignore] tests found under src/");
    assert!(
        !filters.is_empty(),
        "no cargo test --ignored lines in {CI_WORKFLOW}"
    );

    let run_by_ci = |name: &str| filters.iter().any(|filter| name.contains(filter.as_str()));
    let excused = |name: &str| NOT_IN_CI.iter().any(|(excused, _)| *excused == name);

    let never_run: Vec<&String> = tests
        .iter()
        .filter(|name| !run_by_ci(name) && !excused(name))
        .collect();
    assert!(
        never_run.is_empty(),
        "ignored tests that CI never runs: {never_run:?}\n\
         Add each to a `cargo test ... --ignored` line in {CI_WORKFLOW}, \
         or list it in NOT_IN_CI with a reason."
    );

    let stale_filters: Vec<&String> = filters
        .iter()
        .filter(|filter| !tests.iter().any(|name| name.contains(filter.as_str())))
        .collect();
    assert!(
        stale_filters.is_empty(),
        "CI filters that match no ignored test, so that step runs nothing: {stale_filters:?}"
    );

    let stale_excuses: Vec<&str> = NOT_IN_CI
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !tests.iter().any(|test| test == name) || run_by_ci(name))
        .collect();
    assert!(
        stale_excuses.is_empty(),
        "NOT_IN_CI entries that no longer match an ignored test outside CI: {stale_excuses:?}"
    );
}
