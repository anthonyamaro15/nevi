# Performance

How fast Nevi is, release to release. Every number comes from a benchmark in
this repository, so you can check it yourself. Each release adds a column even
when nothing got faster, so a slowdown can't hide.

Measured on an Apple M4 laptop (10 cores) with macOS 27, using release builds.
Both columns were run back to back on the same machine, and each number is the
median of several runs. Expect different numbers on other machines, especially
in the rows that open many files: those depend on the disk and on any security
software that scans files as they open.

| Scenario | 0.3.0 | main (unreleased) |
| --- | --- | --- |
| Live grep, rare query: first result | 5.33 s | 59 ms |
| Live grep, rare query: all results | 5.33 s | 1.70 s |
| Live grep, typing an identifier: final results | 16.91 s | 1.71 s |
| File picker: editor frozen while it opens | 407 ms | 13 µs |
| File picker: full list ready | 407 ms | 125 ms |
| Finder preview: load a hit deep in a large file | 34 ms | 11 ms |
| Editing: reparse after an edit, large Rust file | 27 ms | 3.5 ms |
| Editing: gg=G, file with scrambled indentation | 7.06 s | 1.23 s |
| Drawing: one frame, large multiline file | 88 µs | 89 µs |
| Drawing: one frame, minified one-line file | 988 µs | 909 µs |
| Drawing: one frame, grep preview of a minified one-line file | 29.62 s | 71 µs |

For reference, ripgrep 15.1 takes 1.67 s to search the same repo for a string
that isn't there, so live grep now searches at about the speed of the engine
it's built on.

## What each row measures

- **Live grep** rows search a generated repo of about 43,000 files (450 MB,
  deep iOS-style folder paths, a few large binaries). The rare query has 16
  hits. A search is timed from when it starts, after the picker's 150 ms typing
  delay, until the first result reaches the picker and until it's done. The
  typing row types a 23-character identifier one key every 200 ms, so every key
  starts its own search, and times the final results from the last search's
  start.
- **File picker** rows open the picker on the same repo, which lists the first
  10,000 files by default. "Editor frozen" is how long opening it blocks the
  editor, and "full list ready" is when the last file is in.
- **Finder preview** times loading the preview for a live grep hit on line
  999,990 of a generated 1,000,000-line text file (72 MB). The picker loads it
  once the selection has been still for 50 ms.
- **Editing** rows use generated Rust: an 18,060-line file for the reparse after
  typing one character into a name, and a 2,100-line file with scrambled
  indentation for `gg=G`.
- **Drawing** rows time one full frame in a 120 by 40 terminal: the middle of a
  100,000-line file, the start of a 1.6 MB minified file that is all one line,
  and the live grep picker previewing that same line with matches in view.

None of the times include drawing results on screen.

## Run it yourself

```sh
NEVI_PERF_BENCH=1 cargo test --release --lib -- --ignored --nocapture --test-threads=1 \
  live_grep_bench file_picker_bench large_file_edit_bench finder_preview_bench \
  render_frame_budget 2>&1 | grep -o 'perf | .*'
```

Each line it prints is one row of the table. The first run writes the test repo
and a large log file to `~/.cache/nevi-perf` (about 570 MB) and repeats full
searches until their times settle, which can take a while if antivirus software
is scanning the new files.

The 0.3.0 column comes from the same benchmarks applied to the v0.3.0 release.

## Keeping it current

At every release, run the command above on the release commit, rename the
`main (unreleased)` column to the new version (or add the column), and keep the
last three releases. `cargo test` checks that the table has a column for the
version in `Cargo.toml`, so a release can't ship without one, even when nothing
changed.

A pull request that makes something faster can update its rows in the
`main (unreleased)` column, and one that speeds up something new adds a row
along with its benchmark.
