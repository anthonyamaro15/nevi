# Changelog

## Unreleased

### Vim Compatibility

- The mouse now works like nvim with `mouse=nvi`: the wheel scrolls the file instead of the terminal scrollback, targeting the pane under the pointer, 3 lines per tick. Horizontal wheel scrolls 6 columns with wrap off. Left click focuses a pane and moves the cursor (insert mode stays insert, a plain click drops visual mode), the wheel and clicks also drive the explorer selection, and the wheel scrolls the finder and markdown previews. On by default; `mouse = false` under `[editor]` or `:set nomouse` / `:set mouse=` turns it off, which also makes `mouse` the first option `:set` actually applies. While captured, terminal-native selection needs the terminal's bypass key (Option in iTerm2, Shift elsewhere). (#274)
- Added `Ctrl+e` / `Ctrl+y` to scroll the view one line (or a count of lines) without moving the cursor until it would leave the screen. Verified against real Neovim in the oracle suite, including the scrolloff edge cases at the top and bottom of the file.
- `cw` and `cW` now follow Vim's special case (`:h cw`): on a non-blank they change only up to the end of the word, leaving the trailing whitespace in place, and on the last character of a word they change just that character. On whitespace they still behave like `dw` plus insert. (#272)
- Added the method motions `[m`, `]m`, `[M`, and `]M`. They jump between tree-sitter function boundaries instead of using Vim's brace heuristic, so they land on real functions and methods in Rust-style code. They work with operators (`d]m`, `y[m`) and counts, and do nothing in files without tree-sitter support.
- Fixed `~` to advance the cursor past the last toggled character, as in Vim.
- Extended Vim oracle coverage to the editing core: the case operators (`gu`, `gU`, `g~` and their line forms), `~`, `X`, `s`, `S`, `gp`, `gP`, `J`, and `gJ` are now verified against real Neovim and individually tracked in `PARITY.md`.
- Corrected the docs: `.` (repeat last change) was listed as implemented but only shows a status note today. It moved to the roadmap.

### Interface

- Unified the remaining floating windows under the rich chrome. The leader/which-key popup and the command suggestions/history popup are now rounded-corner boxes with icon border titles and right-aligned key hints, and the command popup marks the selected row with the finder's accent bar instead of `>`. The floating terminal's corners now come from the shared glyph table (square in minimal mode, matching the finder) and its title carries a terminal icon. Minimal mode keeps the previous flat headers throughout.
- Added a start screen. Launching `nevi` with nothing to edit now shows recent files with their project names, harpoon pins, startup time, and key hints; `1`–`9` opens the numbered recent file and `h` + `1`–`9` jumps to that harpoon slot. Recently opened files persist to `~/.local/state/nevi/recent_files.json`, alongside the other shada-lite state. The screen is a pure render condition — it disappears as soon as any real buffer, edit, or mode change happens, and costs nothing afterward.
- Opening a file now scopes the project to its repository root (nearest ancestor with `.git`) instead of the file's own directory. Harpoon pins land in one `.nevi/harpoon.json` at the repo root regardless of which file you opened, and the explorer and floating terminal start at the repo root too. Outside a repository the old parent-directory behavior is unchanged.

### Vim Compatibility

- Macros, named and unnamed registers, global marks, and search history now survive restarts, like Vim's shada. State is stored in `~/.local/state/nevi/state.json`, following nvim's `stdpath('state')` convention, and `$XDG_STATE_HOME` is respected. Macros are saved as readable key notation, so the file can be inspected or hand-edited, and a corrupt file never blocks startup. The frecency database and command history moved to the same directory; data in the old location is found automatically and migrates on its next save.

### Configuration

- Fixed `[ruby]` in `languages.toml` being ignored. Ruby files (`.rb`, `.rake`, `.gemspec`, `.ru`, `.podspec`) resolved to their raw extension instead of the `ruby` key, so formatter and tab width settings never applied. The generated `languages.toml` template now includes a commented Ruby example. (#273)

## 0.3.0 - 2026-08-25

Nevi 0.3.0 rebuilds the statusline, finder, and explorer, closes a long list of
Vim compatibility gaps, and adds PHP and shell language support.

Existing config files keep working. The one thing to know before upgrading is
that the new interface is on by default. Set `[ui] style = "minimal"` in
config.toml if you want the 0.2.0 appearance back.

### Interface

- Rebuilt the statusline from mode-colored segments. It now shows the git branch and diff stats, diagnostic counts, an LSP activity indicator, and a Vim-style `Top` / `Bot` / percent ruler. SEARCH mode has its own badge color.
- Gave the finder rounded corners, icon border titles, per-filetype devicons in both the result list and the preview title, and an accent bar on the selected row.
- Gave the explorer the same accent bar, tinted file names by git status, dot markers on changed files, and a folder icon in the header. Files and folders now carry diagnostic badges, so a folder containing an error reports the count without being expanded.
- Buffer gutter diagnostic signs use Nerd Font glyphs.
- Added `[ui] style`, which takes `rich` (the default) or `minimal`. Minimal keeps the 0.2.0 appearance: plain ASCII statusline, square finder corners, two-letter file chips, explorer letters, and `● ▲ ■ ○` gutter signs. Configs that already set `use_nerd_font_icons = false` get minimal automatically.
- Added an optional `[ui.statusline] section_bg` theme key for the middle statusline segments. It falls back to `cursor_line`, so existing themes need no changes.
- Added a bundled `github-light` theme.
- Moved the raw LSP status string out of the statusline. `:checkhealth` reports it instead, along with the active UI mode and a Nerd Font glyph probe.
- Fixed statusline width math to account for double-width characters. Filenames containing CJK text were pushing the right-hand segments out of alignment.
- Fixed the hover, completion, diagnostic, and finder popups ignoring theme colors, which left them unreadable on light themes.
- Fixed the floating terminal drawing partial frames.

### Vim Compatibility

- Added the motions `g_`, `|`, `gM`, `gm`, `go`, `[[`, `]]`, `][`, `[]`, `[{`, `]}`, `[(`, and `])`.
- `j` and `k` now keep the preferred column across short and blank lines, matching Vim's `curswant`. After `$`, vertical motion sticks to line ends.
- Fixed `J` and `gJ` on the last line quietly deleting the file's trailing newline. Both are now a no-op, as in Vim.
- Files without a final newline gain one on load and on save, matching Neovim's `fixendofline` default. This also fixes the cursor landing on a phantom line when opening a line at the end of a file.
- Fixed `r`, counted `r`, and `R` replace mode to match Vim, including multibyte characters, line boundaries, and undo and redo of counted replaces.
- Fixed counted `o` and `O`, `I` and `A` insert positioning, the editing operators, and linewise edits in new files to match Neovim.
- Fixed page scrolling and the screen position motions `H`, `M`, and `L`. They were ignoring the active pane's row count and mispacking the last screen of a wrapped buffer.

### Languages And Tooling

- Added PHP support.
- Added shell support: tree-sitter highlighting for `.sh`, `.bash`, and `.zsh`, the common rc and profile names (`.bashrc`, `.bash_profile`, `.zshrc`, `PKGBUILD`, and similar), and shebang detection for scripts without an extension.
- Added `[lsp.servers.shell]` for `bash-language-server`, using the same config shape as Go and Ruby.
- `:Format` now runs the external formatter configured in `languages.toml` when the current language has one, and falls back to the LSP otherwise.
- Added `:Macros` and `:MacroEdit` for reviewing and editing recorded macros.
- Fixed clipboard support on Wayland.

### Parity And Testing

- Added `PARITY.md`, a scoreboard generated from the same inventories the test suite enforces, so it cannot drift from what is actually verified. It currently reports 329 keybinds implemented and 39 planned.
- Added a keybind coverage inventory mapping every default keybind to the test that protects it, plus a check that the documented keybinds and the inventory agree.
- Added a key-sequence fuzz harness and a guard against pane state falling out of sync.
- Extended Vim oracle coverage to WORD motions, find-char motions, matching brackets, and the display-line motion family. CI now pins the Neovim version used for parity checks.

### Performance

- Shebang detection reads at most the first 256 characters of the first line instead of copying the whole line, which matters on minified and generated files.

### Contributors

Thanks to @krawitzzZ for shell highlighting and LSP support, the external
formatter in `:Format`, Wayland clipboard support, and the first-line
allocation fix.

### Install And Upgrade

Homebrew users can upgrade after this release with:

```bash
brew update
brew upgrade nevi
```

## 0.2.0 - 2026-07-07

Nevi 0.2.0 is a feature and performance release focused on making the editor
feel faster, safer, and easier to adopt.

### Highlights

- Added damage-aware partial rendering for common cursor movement and edit paths.
- Improved long-line and large-file responsiveness, including clearer large-file mode visibility.
- Added render regression coverage and a frame budget guard to catch future UI regressions earlier.
- Added an in-memory `:FlightRecorder` / `:WhySlow` performance report for debugging latency.
- Added Vim oracle parity coverage and macOS/Linux CI validation.
- Added labeled jump navigation with `:Jump` and `<Space>j`.
- Added Swiss-army CLI modes: `nevi view`, `nevi diff`, and `nevi pick`.
- Added previewed project-wide replace with an explicit apply step.
- Added `:ToolInstall`, `:ConfigDefaults`, and expanded `:checkhealth` reporting.
- Added Go and Ruby language support.
- Added more Vim/Neovim-compatible keybindings, including window movement/resizing, visual block insert/append, `ZZ`, and normal-mode Enter motion.
- Improved Homebrew, Linux/source install, and update documentation.

### Performance

- Partially repaint only affected editor rows for many normal and insert-mode operations.
- Limit search highlights and labeled-jump scans to visible rows.
- Optimize long-line rendering for minified and very wide files.
- Throttle LSP status redraws and hide benign LSP request errors.
- Add input event coalescing coverage to guard responsiveness.

### Safety And Diagnostics

- Guard saves against overwriting files changed externally on disk.
- Open health, config defaults, and generated reports in read-only buffers.
- Add keymap health checks and external tool checks.
- Add project replace safeguards for preview/apply workflows.

### Install And Upgrade

Homebrew users can upgrade after this release with:

```bash
brew update
brew upgrade nevi
```

If installed with the fully qualified formula name:

```bash
brew upgrade anthonyamaro15/nevi/nevi
```

Verify the installed version:

```bash
nevi --version
```

## 0.1.0 - Initial Release

- Initial public release of Nevi.
