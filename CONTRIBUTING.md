# Contributing to Nevi

Thanks for helping. The fastest way to contribute is to report a key that
behaves differently from Neovim; the second fastest is to fix one.

- [Build and test](#build-and-test)
- [Where things live](#where-things-live)
- [Adding a keybind, step by step](#adding-a-keybind-step-by-step)
- [What every change needs](#what-every-change-needs)
- [Pull requests](#pull-requests)
- [Profiling](#profiling)
- [Design decisions](#design-decisions)

## Build and test

```bash
cargo build                       # debug build
cargo run -- path/to/file         # run against a file
cargo fmt --all -- --check        # CI gate
cargo test --quiet                # CI gate, the full suite
cargo test editor::               # one module
cargo test some_test_name         # by name substring
cargo test -- --nocapture         # show println!/dbg! output
```

Rust 1.85 or newer. All tests live inside `src/` as `#[cfg(test)] mod tests`,
so a name filter is how you scope a run.

CI also runs two opt-in gates on macOS and Linux:

```bash
# Frame budget guard: fails if a frame exceeds its budget.
cargo test render_frame_budget -- --ignored --nocapture

# Vim parity against a real headless Neovim. Needs nvim on PATH; CI pins v0.11.3.
NEVI_VIM_ORACLE=1 cargo test vim_oracle_smoke -- --ignored --nocapture
```

## Where things live

| You want to change | Look in |
|--------------------|---------|
| How a key sequence is parsed: counts, operators, pending keys | `src/input/mod.rs` |
| What a motion does to the buffer | `src/input/motion.rs`, pure functions that are easy to unit test |
| Key dispatch, modes, overlays, rendering | `src/terminal/mod.rs` (`handle_key`) |
| Editor state: buffers, panes, registers, marks | `src/editor/mod.rs`. Put new logic in a focused module and call it from here. |
| Ex commands (`:Something`) | `src/commands/mod.rs` |
| Config options and the generated template | `src/config/mod.rs`, `src/config/keymap.rs`, `src/config/languages.rs` |
| The `:Keymaps` cheatsheet | `src/finder/keybinds.toml` |
| LSP, syntax, finder, git, terminal, Copilot, themes | `src/lsp/`, `src/syntax/`, `src/finder/`, `src/git/`, `src/floating_terminal.rs`, `src/copilot/`, `src/theme/` |

The library (`src/lib.rs`) owns all editor state and rendering; the binary
(`src/main.rs`) owns the event loop and the async subsystems.

## Adding a keybind, step by step

Worked example: adding `U`, Vim's "undo all changes on the last changed line",
which is on the roadmap today.

### 1. Decide what kind of keybind it is

| Kind | Meaning | Test that protects it |
|------|---------|-----------------------|
| **Vim default** | Neovim has this key and Nevi should match it exactly | An oracle case, compared against real Neovim |
| **Nevi-native** | Nevi's own behavior, such as LSP keys, the finder, harpoon | A focused regression test in the owning module |
| **Deliberate deviation** | The key exists in Vim but Nevi does something better on purpose, like `]m` using tree-sitter | A regression test, plus a row in the Deliberate Deviations table in `KEYBINDS_ROADMAP.md` |

`U` is a Vim default, so it gets an oracle case.

### 2. Write the failing test first

**Vim default.** Add a case to the file in `src/vim_oracle/` that matches the
key's family. Each case is a starting buffer plus a literal key sequence; the
harness replays it in Nevi and in `nvim --headless` and compares buffer text,
cursor position, viewport top, and mode.

| Family | File |
|--------|------|
| Motions, viewport, undo and redo | `src/vim_oracle.rs` (`MOTION_CASES`, `UNDO_REDO_CASES`, the viewport and wrap groups) |
| Operators and edits | `src/vim_oracle/editing_cases.rs` |
| Search | `src/vim_oracle/search_cases.rs` |
| Text objects | `src/vim_oracle/text_object_cases.rs` |
| Visual mode | `src/vim_oracle/visual_cases.rs` |
| Insert entry, open line, replace, increment | `insert_entry_cases.rs`, `open_line_cases.rs`, `replace_cases.rs`, `increment_cases.rs` |

```rust
// src/vim_oracle.rs, in UNDO_REDO_CASES
OracleCase {
    name: "undo line restores the last changed line",
    initial_text: "alpha\nbeta\n",
    keys: "jxxU",
},
```

- Pick a `name` that reads as a sentence. It is the id the coverage inventory
  and `PARITY.md` refer to.
- Keys are literal Vim notation: `<Esc>`, `<CR>`, `<C-r>`. Something that is
  not visible in the buffer, like a yank, is made observable by pasting it.
- Run `cargo test vim_oracle --quiet`. The case fails now, and the report
  prints both editors' snapshots side by side.
- Run it against real Neovim once before you open the PR:
  `NEVI_VIM_ORACLE=1 cargo test vim_oracle_smoke -- --ignored --nocapture`.

**Nevi-native.** Write a `#[test]` in the module that owns the behavior,
inside its `#[cfg(test)] mod tests`. Name it for the behavior, not the key,
because the name becomes the `test_id` in step 4. An example from the repo:

```rust
// src/terminal/mod.rs
#[test]
fn visual_equals_reindents_selection_like_double_equals() {
    // build an Editor, send the keys, assert on the buffer
}
```

Rendering tests use `render_editor_to_string(&editor)` and assert on the
emitted ANSI sequences. That is how highlight and theme regressions are caught.

### 3. Implement it

Add the key to the grammar in `src/input/mod.rs`, or to `handle_key` in
`src/terminal/mod.rs` if it is mode or overlay specific, and put the actual
logic in a focused module rather than growing `src/editor/mod.rs`. Then make
sure of three things:

- A count works, or is explicitly ignored the way Vim ignores it.
- It participates in dot repeat and one-step undo if it changes text.
- Comments explain the Vim rule or the edge case, not what the code does.
  Cite the help tag (`:h U`) when a rule is subtle.

If the change touches per-pane state, remember that `Editor` mirrors the
active pane's `cursor`, `viewport_offset`, `h_offset`, and buffer index. Any
new per-pane field must be added to `Pane` and synced in `save_pane_state()`
and `load_pane_state()`.

### 4. Register it in the coverage inventory

`src/keybind_coverage.rs` maps every default keybind to the test that protects
it. Tests enforce that entries are unique and complete and that oracle names
exist.

```rust
// Vim default, normal mode
vim_oracle("U", "Undo all changes on the last changed line", "undo line restores the last changed line"),

// Nevi-native, normal mode
nevi_regression("<leader>ca", "Code actions", "leader_ca_opens_code_actions"),

// Any other mode uses the struct form
KeybindCoverage {
    mode: KeybindMode::Visual,
    key: "=",
    description: "Re-indent selected lines",
    kind: CoverageKind::NeviRegression,
    state: CoverageState::Protected {
        test_id: "visual_equals_reindents_selection_like_double_equals",
    },
},
```

If you truly cannot test it yet, use
`CoverageState::NeedsCoverage { reason: "..." }`. A gap without a written
reason fails the suite, and the reason shows up in `PARITY.md`.

### 5. Update the docs that tests check

| File | What to do | Test that fails if you forget |
|------|------------|-------------------------------|
| `KEYBINDINGS.md` | Add a row to the right table, in Vim notation | `documented_rows_parse_and_match_inventory_notation` |
| `src/finder/keybinds.toml` | Add the `:Keymaps` cheatsheet entry | `cheatsheet_covers_documented_keybinds` |
| `KEYBINDS_ROADMAP.md` | Remove the key from Planned and bump the `**Status:**` counts. That line is the source `PARITY.md` reads its totals from. | `parity_md_is_up_to_date` |
| `PARITY.md` | Never edit by hand. Regenerate and commit: `NEVI_UPDATE_PARITY=1 cargo test parity_report` | `parity_md_is_up_to_date` |

```toml
# src/finder/keybinds.toml
[[normal.editing]]
key = "U"
action = "undo_line"
desc = "Undo all changes on the last changed line"
status = "implemented"
vim_default = true
```

### 6. Update the docs people read

- `CHANGELOG.md`, under Unreleased, in the matching group (Vim Compatibility,
  Interface, Performance, Configuration). Say what changed from the user's
  side and end with "Verified against real Neovim" when an oracle case backs
  it. Reference the issue number when there is one.
- `README.md` only if the change alters a headline feature, install steps, or
  the Quick Start.
- `CONFIGURATION.md` if the key is configurable or you added a config option.

### 7. Run the gates

```bash
cargo fmt --all -- --check
cargo test --quiet
cargo test render_frame_budget -- --ignored --nocapture        # if you touched rendering
NEVI_VIM_ORACLE=1 cargo test vim_oracle_smoke -- --ignored     # if you added oracle cases
```

## What every change needs

| Change | Tests | Docs |
|--------|-------|------|
| Vim-default keybind | Oracle case, coverage entry | `KEYBINDINGS.md`, `keybinds.toml`, roadmap counts, `PARITY.md` regen, `CHANGELOG.md` |
| Nevi-native keybind | Regression test, coverage entry | `KEYBINDINGS.md`, `keybinds.toml`, `PARITY.md` regen, `CHANGELOG.md` |
| Bug fix | A regression test that fails before the fix. For Vim differences, an oracle case | `CHANGELOG.md`, with the issue number |
| New ex command | Test in `src/commands/mod.rs` | `KEYBINDINGS.md` Commands table, `CHANGELOG.md` |
| Config option | Parse test in `src/config/`; add it to the generated template | `CONFIGURATION.md`, `CHANGELOG.md` |
| Rendering or statusline | `render_editor_to_string` test, frame budget gate | `CHANGELOG.md` under Interface |
| Language or LSP server | Detection test | `README.md` Languages table, `CHANGELOG.md` |
| Theme | Loads without warnings | `README.md` theme list, `CHANGELOG.md` |
| Performance | A measurement in the PR description; frame budget gate | `CHANGELOG.md` under Performance, with before and after numbers |
| A decision with tradeoffs users notice | | An ADR in `docs/adr/` |

Hot paths have their own rules: no blocking work on keypress or render, scan
visible ranges rather than whole buffers, and keep background work bounded.

## Pull requests

For anything larger than a bug fix, open an issue first so the approach is
agreed before the code exists. Then:

- **One behavior per PR.** A fix plus a refactor plus a feature is three PRs.
  Small PRs merge the same day.
- **Branch and commit names** use the prefixes the history already uses:
  `feat/`, `fix/`, `docs/`, `test/`, `refactor/`, `perf/`. Commit subjects the
  same way: `feat: add U to undo the last changed line`.
- **Describe it from the user's side.** What was wrong or missing, what it
  does now, and how you verified it. Paste the oracle output or the before and
  after numbers.
- **CI must be green on both jobs**, macOS and Linux: fmt, the full suite, the
  frame budget guard, and the real-Neovim oracle smoke.

Checklist to paste into the description:

```markdown
- [ ] Failing test written first (oracle case or regression test)
- [ ] Coverage entry added in src/keybind_coverage.rs (keybinds only)
- [ ] KEYBINDINGS.md row and src/finder/keybinds.toml entry (keybinds only)
- [ ] KEYBINDS_ROADMAP.md updated and PARITY.md regenerated (keybinds only)
- [ ] CHANGELOG.md entry under Unreleased
- [ ] README.md / CONFIGURATION.md updated if behavior, install, or config changed
- [ ] cargo fmt --all -- --check and cargo test --quiet pass locally
- [ ] Verified against real Neovim if an oracle case was added
```

## Profiling

`:WhySlow` (or `:FlightRecorder`) opens a read-only buffer with recent render,
input, syntax, LSP, finder, and terminal timings from an in-memory flight
recorder.

For a full profile:

```bash
NEVI_PROFILE=1 cargo run --release -- path/to/file
```

On exit Nevi writes raw timing events and a summary to `/tmp/nevi_profile.log`
with count, total, average, p50, p95, and max microseconds for key handling,
syntax updates, full renders, and terminal-only renders. `:checkhealth` shows
the last summary.

## Design decisions

Decisions with tradeoffs are recorded under [`docs/adr/`](docs/adr/). Read
[0001, mouse capture on by default](docs/adr/0001-mouse-capture-on-by-default.md)
for the format. Add one when a change picks between options users will notice.
