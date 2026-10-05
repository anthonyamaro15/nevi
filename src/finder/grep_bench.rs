//! Live grep latency bench (issue #354), and `live_grep_budget`, the small
//! version of it that CI runs.
//!
//! The bench prints numbers to compare before and after a change. It is a
//! measuring tool, not a pass/fail gate, so CI skips it (see `opt_in_gates`).
//! It only runs when asked to, in release:
//!
//! ```text
//! NEVI_GREP_BENCH=1 cargo test --release live_grep_bench -- --ignored --nocapture
//! ```
//!
//! The first run writes a deterministic iOS-style repo (about 43k files and
//! 450 MB, paths around 120 characters) to `~/.cache/nevi-perf/` and keeps
//! it. `NEVI_GREP_BENCH_ROOT` points the bench at another directory, which
//! it only reads. Keeping the corpus matters: antivirus scanners such as
//! Microsoft Defender inspect freshly written files and make full scans 20x
//! slower until they finish, so the bench also repeats full scans until
//! three in a row agree and the antivirus scanner is idle before it measures
//! anything.
//!
//! Every scenario drives `FuzzyFinder` the way the main loop does: set the
//! query, `execute_grep_search`, then `poll_grep_search` until the search
//! reports it finished. Polling runs every millisecond, so the numbers leave
//! out the main loop's own tick (up to 16 ms) and the redraw.

use super::{FuzzyFinder, GrepSearcher};
use crate::config::FinderSettings;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Planted in one Swift file in 2,400: a handful of hits, so the search has
/// to scan the whole repo.
const SPARSE: &str = "termsOfUseConsentBanner";
/// In every 8th feature's translations: 1,680 hits, capped at 1,000.
const MEDIUM: &str = "account_registration_paragraph_terms";
/// Matches almost every file.
const DENSE: &str = "te";
/// Matches nothing.
const NO_MATCH: &str = "zzq_nomatch_qq";
/// Time between keystrokes when typing. The main loop starts a search 150 ms
/// after a keystroke, so at this pace every keystroke starts its own search.
const KEYSTROKE_GAP: Duration = Duration::from_millis(200);
/// How long a search runs before Esc closes the picker.
const ESC_AFTER: Duration = Duration::from_millis(300);
/// No scenario should come close to this; past it the bench calls it a hang.
const TIMEOUT: Duration = Duration::from_secs(300);

struct Run {
    results: usize,
    first_visible: Option<Duration>,
    finished: Duration,
}

fn grep_finder(root: &Path) -> FuzzyFinder {
    let mut finder = FuzzyFinder::from_settings(&FinderSettings::default());
    finder.open_grep(root);
    finder
}

/// Start a search the way the main loop does once its debounce expires.
fn start_search(finder: &mut FuzzyFinder, query: &str) -> Instant {
    finder.query = query.to_string();
    finder.grep_search_pending = true;
    let started = Instant::now();
    finder.execute_grep_search();
    started
}

/// Apply arriving batches for a while, like the main loop between keys.
fn poll_for(finder: &mut FuzzyFinder, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        finder.poll_grep_search();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Poll until the current search reports that it finished.
fn wait_for_results(finder: &mut FuzzyFinder, started: Instant) -> Run {
    let mut first_visible = None;
    loop {
        finder.poll_grep_search();
        if first_visible.is_none() && !finder.items.is_empty() {
            first_visible = Some(started.elapsed());
        }
        if !finder.grep_search_running {
            return Run {
                results: finder.items.len(),
                first_visible,
                finished: started.elapsed(),
            };
        }
        assert!(
            started.elapsed() < TIMEOUT,
            "search did not finish within {TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Threads alive in this process. A search the finder stopped listening to
/// keeps its threads until it notices, so this is how the bench sees
/// leftover work. None where the count can't be read.
fn thread_count() -> Option<usize> {
    if let Ok(tasks) = fs::read_dir("/proc/self/task") {
        return Some(tasks.count());
    }
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-M", "-p", &pid])
        .output()
        .ok()?;
    // macOS prints a header line, then one line per thread.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .count()
        .checked_sub(1)
}

/// Wait until no more than `baseline` threads are left and return how long
/// that took.
fn wait_for_idle(baseline: Option<usize>) -> Option<Duration> {
    let baseline = baseline?;
    let started = Instant::now();
    while thread_count()? > baseline {
        assert!(
            started.elapsed() < TIMEOUT,
            "background searches still running after {TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    Some(started.elapsed())
}

fn isolated(root: &Path, query: &str) -> Run {
    let mut finder = grep_finder(root);
    let started = start_search(&mut finder, query);
    wait_for_results(&mut finder, started)
}

/// Type `query` one key per KEYSTROKE_GAP, starting a search per key like
/// the main loop does (one character doesn't search). Timed from the last
/// search's start, so it is the wait for final results after typing stops.
fn typing(root: &Path, query: &str) -> (Run, usize) {
    let mut finder = grep_finder(root);
    let chars: Vec<char> = query.chars().collect();
    let mut searches = 0;
    let mut started = Instant::now();
    for len in 2..=chars.len() {
        if searches > 0 {
            poll_for(&mut finder, KEYSTROKE_GAP.saturating_sub(started.elapsed()));
        }
        let prefix: String = chars[..len].iter().collect();
        started = start_search(&mut finder, &prefix);
        searches += 1;
    }
    (wait_for_results(&mut finder, started), searches)
}

/// Start a search with few hits, close the picker after ESC_AFTER, and
/// return how long the search's threads keep running after that.
fn esc_leftover(root: &Path, baseline: Option<usize>) -> Option<Duration> {
    let mut finder = grep_finder(root);
    start_search(&mut finder, SPARSE);
    poll_for(&mut finder, ESC_AFTER);
    // This is what close_finder does to the search.
    finder.cancel_background_work();
    wait_for_idle(baseline)
}

/// CPU use of Microsoft Defender's scanning engine, in percent of one core.
/// It works through a backlog for many minutes after files are written, and
/// full scans read 20x slower meanwhile. It stays near zero once the files
/// are known; the per-open cost every scan keeps paying shows up in other
/// Defender processes, and is part of what users see too.
fn antivirus_scan_cpu() -> f64 {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pcpu=,comm="])
        .output()
    else {
        return 0.0;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.ends_with("wdavdaemon_unprivileged"))
        .filter_map(|line| line.split_whitespace().next()?.parse::<f64>().ok())
        .sum()
}

/// Repeat full scans until three in a row agree within 10% and antivirus
/// software isn't scanning. Agreement alone isn't enough: on a fresh corpus
/// Defender's scanner ran at 700%+ CPU for many minutes, and full scans held
/// steady at 31s instead of ~1.8s.
fn warm_up(root: &Path, baseline: Option<usize>) {
    print!("warm-up full scans:");
    let mut passes: Vec<f64> = Vec::new();
    for _ in 0..60 {
        let secs = isolated(root, NO_MATCH).finished.as_secs_f64();
        wait_for_idle(baseline);
        let scanning = antivirus_scan_cpu();
        print!(" {secs:.2}s");
        if scanning >= 50.0 {
            print!(" (antivirus scanning at {scanning:.0}% CPU)");
        }
        let _ = std::io::stdout().flush();
        passes.push(secs);
        if let [.., a, b, c] = passes[..] {
            if a.max(b).max(c) <= 1.1 * a.min(b).min(c) && scanning < 50.0 {
                println!(" (settled)");
                return;
            }
        }
    }
    println!(" (never settled, expect noisy numbers)");
}

fn ms(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn report(label: &str, runs: &[Run]) {
    let finished: Vec<Duration> = runs.iter().map(|run| run.finished).collect();
    let first: Vec<Duration> = runs.iter().filter_map(|run| run.first_visible).collect();
    let first = if first.is_empty() {
        "none".to_string()
    } else {
        ms(median(first))
    };
    let min = *finished.iter().min().unwrap();
    let max = *finished.iter().max().unwrap();
    println!(
        "{label:<54} results={:<5} first_visible={first:>9}  finished={:>9} [{}-{}]",
        runs[0].results,
        ms(median(finished)),
        ms(min),
        ms(max),
    );
}

#[test]
#[ignore = "measuring tool: NEVI_GREP_BENCH=1, release, --nocapture; see the module docs"]
fn live_grep_bench() {
    // A plain `cargo test -- --ignored` shouldn't write 450 MB into the home
    // directory and run for minutes.
    if std::env::var_os("NEVI_GREP_BENCH").is_none() {
        println!("live_grep_bench skipped: set NEVI_GREP_BENCH=1 to run it");
        return;
    }
    let runs = std::env::var("NEVI_GREP_BENCH_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    let baseline = thread_count();
    let root = corpus_root();
    let build = if cfg!(debug_assertions) {
        "DEBUG build, numbers are not meaningful"
    } else {
        "release"
    };
    println!(
        "live grep bench: {} ({build}, median of {runs} [min-max])",
        root.display()
    );
    warm_up(&root, baseline);

    for query in [DENSE, MEDIUM, SPARSE, NO_MATCH] {
        let samples: Vec<Run> = (0..runs)
            .map(|_| {
                let run = isolated(&root, query);
                wait_for_idle(baseline);
                run
            })
            .collect();
        report(&format!("isolated  {query}"), &samples);
    }

    for query in [SPARSE, NO_MATCH] {
        let mut samples = Vec::new();
        let mut leftovers = Vec::new();
        let mut searches = 0;
        for _ in 0..runs {
            let (run, count) = typing(&root, query);
            searches = count;
            samples.push(run);
            leftovers.extend(wait_for_idle(baseline));
        }
        report(
            &format!("typing    {query} ({searches} searches)"),
            &samples,
        );
        if !leftovers.is_empty() {
            println!(
                "          old searches still running after the final results: {}",
                ms(median(leftovers))
            );
        }
    }

    let leftovers: Vec<Duration> = (0..runs)
        .filter_map(|_| esc_leftover(&root, baseline))
        .collect();
    if leftovers.is_empty() {
        println!("esc       (thread count unavailable on this platform)");
    } else {
        let min = *leftovers.iter().min().unwrap();
        let max = *leftovers.iter().max().unwrap();
        println!(
            "esc       {SPARSE}, closed after {}: search threads gone {} later [{}-{}]",
            ms(ESC_AFTER),
            ms(median(leftovers)),
            ms(min),
            ms(max),
        );
    }
}

/// Planted once in each of the budget repo's 30 folders: fewer hits than one
/// 50-result batch, spread so some worker meets one early.
const BUDGET_HIT: &str = "needle_for_the_budget";

/// The bench's three findings as a CI check, on a repo small enough to write
/// every run. Each limit is relative to a full scan timed in the same run, so
/// a slow or noisy machine can't fail it, while undoing a fix does: results
/// handed over 50 at a time (1), no stop check inside reads (2, the wiring
/// no unit test pins), a replaced search left running (3). It runs in its
/// own CI step so no other test shares the machine while it measures.
#[test]
#[ignore = "perf budget guard: CI runs it with --ignored in its own step"]
fn live_grep_budget() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nevi_live_grep_budget_{}_{nanos}",
        std::process::id()
    ));
    let small = dir.join("small");
    let big = dir.join("big");
    write_budget_repo(&small);
    fs::create_dir_all(&big).unwrap();
    // No NUL bytes (they end the read at once) and nothing that matches.
    let line = "plain text in a big dump that never matches the query\n";
    fs::write(big.join("dump.txt"), line.repeat((32 << 20) / line.len())).unwrap();
    // Untimed passes, so the timed ones read warm files.
    GrepSearcher::new().search(&small, NO_MATCH);
    GrepSearcher::new().search(&big, NO_MATCH);

    // 1. A search with few hits shows the first one long before it ends.
    let run = isolated(&small, BUDGET_HIT);
    let first = run.first_visible.expect("the planted hits are found");
    println!(
        "live_grep_budget 1: first of {} results after {} of a {} scan",
        run.results,
        ms(first),
        ms(run.finished)
    );
    assert_eq!(run.results, 30);
    assert!(
        first * 2 <= run.finished,
        "the first result waited for most of the scan: {first:?} of {:?}",
        run.finished
    );

    // 2. Cancelling a quarter of the way into one big file stops within a
    // quarter of the full read, not at the end of the file.
    let started = Instant::now();
    GrepSearcher::new().search(&big, NO_MATCH);
    let full_read = started.elapsed();
    let cancel = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let search_cancel = Arc::clone(&cancel);
    let search_root = big.clone();
    std::thread::spawn(move || {
        GrepSearcher::new().search_stream(&search_root, NO_MATCH, 50, &search_cancel, |_| true);
        let _ = done_tx.send(Instant::now());
    });
    std::thread::sleep(full_read / 4);
    let cancelled_at = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    let stopped = done_rx
        .recv()
        .unwrap()
        .saturating_duration_since(cancelled_at);
    // The floor keeps a release build, where the whole read takes a few ms,
    // from failing on scheduling noise.
    let allowed = (full_read / 4).max(Duration::from_millis(20));
    println!(
        "live_grep_budget 2: cancelled read stopped after {} (full read {}, allowed {})",
        ms(stopped),
        ms(full_read),
        ms(allowed)
    );
    assert!(
        stopped <= allowed,
        "a cancelled search kept reading the big file: {stopped:?}, allowed {allowed:?}"
    );

    // 3. Eight searches replacing each other every quarter scan: the last
    // finishes in about the time of one, because the others stopped.
    let single = median(
        (0..3)
            .map(|_| isolated(&small, NO_MATCH).finished)
            .collect(),
    );
    // Best of three: one search can be slow for reasons that have nothing to
    // do with us, while searches that pile up are slow every time.
    let last = (0..3)
        .map(|_| {
            let mut finder = grep_finder(&small);
            let mut last_started = Instant::now();
            for i in 0..8 {
                if i > 0 {
                    poll_for(
                        &mut finder,
                        (single / 4).saturating_sub(last_started.elapsed()),
                    );
                }
                last_started = start_search(&mut finder, &format!("{NO_MATCH}{i}"));
            }
            wait_for_results(&mut finder, last_started).finished
        })
        .min()
        .unwrap();
    println!(
        "live_grep_budget 3: last of 8 replaced searches took {} at best of 3 (one search {})",
        ms(last),
        ms(single)
    );
    assert!(
        last * 2 <= single * 5,
        "replaced searches piled up: the last took {last:?}, one search takes {single:?}"
    );

    let _ = fs::remove_dir_all(dir);
}

/// 30 folders of 100 files with no matches, plus BUDGET_HIT in one file of
/// each folder.
fn write_budget_repo(root: &Path) {
    let filler = "let value = compute(input); // nothing to find here\n".repeat(80);
    for folder in 0..30 {
        let dir = root.join(format!("module_{folder}"));
        fs::create_dir_all(&dir).unwrap();
        for file in 0..100 {
            let mut text = filler.clone();
            if file == 37 {
                text.push_str("// needle_for_the_budget\n");
            }
            fs::write(dir.join(format!("file_{file}.rs")), text).unwrap();
        }
    }
}

/// The directory to search: NEVI_GREP_BENCH_ROOT as is, or the generated
/// corpus, written on first use.
fn corpus_root() -> PathBuf {
    if let Some(root) = std::env::var_os("NEVI_GREP_BENCH_ROOT") {
        return PathBuf::from(root);
    }
    // Bump the version whenever write_corpus changes, so old copies aren't
    // compared against new ones.
    let root = dirs::home_dir()
        .expect("home directory")
        .join(".cache/nevi-perf/grep-corpus-v1");
    let done = root.with_extension("done");
    if !done.exists() {
        // Left over from an interrupted run: start over.
        let _ = fs::remove_dir_all(&root);
        let started = Instant::now();
        let (files, bytes) = write_corpus(&root);
        fs::write(&done, format!("{files} files, {bytes} bytes\n")).expect("write marker");
        println!(
            "wrote {files} files ({} MB) to {} in {:.1}s; expect slow warm-up passes while antivirus scans them",
            bytes / 1_000_000,
            root.display(),
            started.elapsed().as_secs_f64()
        );
    }
    root
}

const WORDS: &[&str] = &[
    "account",
    "registration",
    "paragraph",
    "terms",
    "privacy",
    "policy",
    "user",
    "profile",
    "settings",
    "network",
    "request",
    "response",
    "session",
    "token",
    "payment",
    "card",
    "checkout",
    "order",
    "history",
    "notification",
    "badge",
    "message",
    "thread",
    "inbox",
    "search",
    "result",
    "filter",
    "sort",
    "view",
    "model",
    "controller",
    "coordinator",
    "service",
    "repository",
    "cache",
    "store",
    "reducer",
    "action",
    "state",
    "onboarding",
    "banner",
    "consent",
    "analytics",
    "tracking",
    "experiment",
    "flag",
    "remote",
    "config",
];

/// SplitMix64, so every machine generates the same corpus without a crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in lo..=hi.
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo + 1) as u64) as usize
    }

    fn word(&mut self) -> &'static str {
        WORDS[self.range(0, WORDS.len() - 1)]
    }

    /// e.g. `payment_card_order`
    fn ident(&mut self) -> String {
        let count = self.range(2, 4);
        (0..count)
            .map(|_| self.word())
            .collect::<Vec<_>>()
            .join("_")
    }

    /// e.g. `PaymentCardOrder`
    fn camel(&mut self) -> String {
        let count = self.range(2, 4);
        (0..count)
            .map(|_| {
                let word = self.word();
                word[..1].to_uppercase() + &word[1..]
            })
            .collect()
    }

    fn sentence(&mut self, words: usize) -> String {
        (0..words)
            .map(|_| self.word())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn swift_line(rng: &mut Rng) -> String {
    match rng.range(0, 5) {
        0 => format!(
            "    let {} = {}({}: {})",
            rng.ident(),
            rng.camel(),
            rng.ident(),
            rng.range(0, 999)
        ),
        1 => format!(
            "    func {}(_ {}: {}) -> {} {{",
            rng.ident(),
            rng.ident(),
            rng.camel(),
            rng.camel()
        ),
        2 => format!("        return {}.{}", rng.ident(), rng.ident()),
        3 => format!("    // {}", rng.sentence(8)),
        4 => "    }".to_string(),
        _ => format!("    private var {}: {}?", rng.ident(), rng.camel()),
    }
}

/// Deep iOS-style paths (`domain/<Area>/<Feature>/Sources/<Layer>/<Group>/
/// <Name>.swift`), Localizable.strings in 12 locales for every 4th feature,
/// two 3 MB one-line JSON files, and four 25 MB extension-less binaries like
/// vendored frameworks. Returns (files, bytes).
fn write_corpus(root: &Path) -> (usize, u64) {
    const LAYERS: [(&str, &[&str]); 3] = [
        ("Presentation", &["Views", "ViewModels", "Coordinators"]),
        ("Domain", &["UseCases", "Entities"]),
        ("Data", &["Repositories", "Network", "Persistence"]),
    ];
    const LOCALES: [&str; 12] = [
        "en", "de", "fr", "es", "it", "ja", "ko", "pt-BR", "zh-Hans", "nl", "sv", "pl",
    ];

    let mut rng = Rng(7);
    let pool: Vec<String> = (0..30_000).map(|_| swift_line(&mut rng)).collect();
    let mut files = 0;
    let mut bytes = 0u64;
    let mut write = |path: PathBuf, contents: &[u8]| {
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create corpus dir");
        fs::write(&path, contents).expect("write corpus file");
        files += 1;
        bytes += contents.len() as u64;
    };

    let mut swift_files = 0;
    for _ in 0..20 {
        let area = format!("{}Domain", rng.camel());
        for feature_idx in 0..50 {
            let feature = root
                .join("domain")
                .join(&area)
                .join(format!("{}Feature", rng.camel()));
            for (layer, groups) in LAYERS {
                for group in groups {
                    for _ in 0..rng.range(3, 7) {
                        let name = format!("{}{}", rng.camel(), group.trim_end_matches('s'));
                        let mut body = format!("import Foundation\n\nfinal class {name} {{\n");
                        for _ in 0..rng.range(20, 200) {
                            body.push_str(&pool[rng.range(0, pool.len() - 1)]);
                            body.push('\n');
                        }
                        body.push_str("}\n");
                        swift_files += 1;
                        if swift_files % 2_400 == 0 {
                            body.push_str("// termsOfUseConsentBanner shown after registration\n");
                        }
                        let path = feature
                            .join("Sources")
                            .join(layer)
                            .join(group)
                            .join(format!("{name}.swift"));
                        write(path, body.as_bytes());
                    }
                }
            }
            if feature_idx % 4 == 0 {
                for locale in LOCALES {
                    let mut lines: Vec<String> = (0..200)
                        .map(|_| format!("\"{}\" = \"{}\";", rng.ident(), rng.sentence(6)))
                        .collect();
                    if feature_idx % 8 == 0 {
                        let at = rng.range(0, lines.len());
                        lines.insert(
                            at,
                            "\"account_registration_paragraph_termsOfUse\" = \"By registering you agree to the Terms of Use\";"
                                .to_string(),
                        );
                    }
                    let path = feature
                        .join("Resources")
                        .join(format!("{locale}.lproj"))
                        .join("Localizable.strings");
                    write(path, (lines.join("\n") + "\n").as_bytes());
                }
            }
        }
    }

    // Minified JSON: one line, megabytes long.
    for i in 0..2 {
        let mut json = String::from("[");
        while json.len() < 3_000_000 {
            if json.len() > 1 {
                json.push(',');
            }
            json.push_str(&format!("{{\"{}\":\"{}\"}}", rng.ident(), rng.sentence(5)));
        }
        json.push_str("]\n");
        let path = root
            .join("domain/Shared/Fixtures")
            .join(format!("fixture_{i}.min.json"));
        write(path, json.as_bytes());
    }

    // Extension-less vendored binaries, like Foo.xcframework/.../Foo.framework/Foo.
    for name in ["Analytics", "Payments", "Maps", "Video"] {
        let data: Vec<u8> = (0..25 * 1024 * 1024 / 8)
            .flat_map(|_| rng.next().to_le_bytes())
            .collect();
        let path = root.join("domain/Vendor").join(format!(
            "{name}.xcframework/ios-arm64/{name}.framework/{name}"
        ));
        write(path, &data);
    }

    (files, bytes)
}
