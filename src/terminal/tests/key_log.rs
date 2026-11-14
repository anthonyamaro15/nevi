//! Key handling must not write what the user types to disk.

use std::path::{Path, PathBuf};

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_source_file_writes_the_slow_key_log() {
    // Slow keys used to be appended, along with the key typed, to a file in
    // /tmp that every user on the machine can read. Key timings belong in the
    // in-memory flight recorder. The name is split so this file passes.
    let log_name = concat!("nevi_", "debug.log");
    let mut sources = Vec::new();
    rust_sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(!sources.is_empty());

    let offenders: Vec<PathBuf> = sources
        .into_iter()
        .filter(|path| std::fs::read_to_string(path).unwrap().contains(log_name))
        .collect();
    assert!(
        offenders.is_empty(),
        "{log_name} is written by {offenders:?}"
    );
}
