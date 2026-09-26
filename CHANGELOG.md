# Changelog

## Unreleased

### Performance

- Live grep applies incoming result batches in about 0.1ms instead of 2.3ms and no longer redoes highlight work for results already on the list, so the finder stays responsive while matches stream in. (#301)
- Typing in the file, buffer, and git changes pickers now computes match highlights only for the rows on screen instead of every match, roughly halving keystroke cost in large projects. (#302)
- Document changes go to the LSP once per input batch instead of once per keystroke, with the serialized text shared with Copilot, so fast input bursts cost one document pass instead of one per key. (#303)
- The file picker no longer blocks the editor while scanning the project. The walk runs on parallel background workers and streams results in as they are found, so the picker opens instantly with a scanning counter, and the walk itself is about 5x faster. (#305)
- Live grep walks and searches files with parallel workers instead of one file at a time. Results are unchanged; only the order files appear in can differ between runs. (#306)
- The full repo git status scan runs in the background instead of blocking startup, saving, focus changes, and opening the explorer. Explorer markers apply when the scan finishes, a frame or two later. Opening a 15k file repo went from about 130ms to about 30ms. (#308)
- Syntax highlighting now reparses incrementally when an edit leaves the file syntactically valid: the edit is applied to the previous tree instead of reparsing the whole file. That covers undo and redo, paste, line deletes, re-indents, and typing inside identifiers, strings, and comments. While the file has a syntax error, such as a half typed statement, the editor still does a full parse, because tree-sitter's error recovery can pick a different tree when it reuses nodes. Buffer switches, reloads, formatter output, and shell files also stay on the full parse. A reparse of an 18k line file after a valid edit went from 43.8ms to 5.4ms, and gg=G on a 2.1k line file that parses cleanly from 8 seconds to about 1.3. Set NEVI_INCREMENTAL_PARSE=0 to force the old full reparse path for every edit. (#309)

### Vim Compatibility

- Left-click dragging now selects text in characterwise Visual mode. Releasing the mouse keeps the selection for Visual operators, and selections started in Insert mode return to Insert after an operator or Escape. Selection follows wrapped/scrolled text and stays in the pane where the drag began. (#345)

- Macros, named and unnamed registers, global marks, and search history now survive restarts, like Vim's shada. State is stored in `~/.local/state/nevi/state.json`, following nvim's `stdpath('state')` convention, and `$XDG_STATE_HOME` is respected. Macros are saved as readable key notation, so the file can be inspected or hand-edited, and a corrupt file never blocks startup. The frecency database and command history moved to the same directory; data in the old location is found automatically and migrates on its next save.
- Added `&` to repeat the last `:s` on the current line, with a count for that many lines, and `g&` to repeat it on every line. Both keep the flags of the original `:s`, matching Neovim, whose default `&` is `:&&`. Neither is picked up by `.`, same as any `:` command. Along the way, `:s` and `:%s` now leave the cursor on the first non-blank of the last line they changed, as Vim does, instead of where it was. Verified against real Neovim.
- Operators with `j`, `k`, `G`, `gg`, `{n}G`, `H`, `M`, and `L` now work on whole lines like Vim. `dj` deletes the current and next line, `cj` changes them into one line that keeps the first line's indent, and `yj` yanks them linewise; before, these ran characterwise and `dj` left an empty line behind. `dj` on the last line and `dk` on the first do nothing, while `dG` and `dgg` still delete the line they are on. `H`, `M`, and `L` under an operator now count from the lines on screen rather than from the top of the buffer, and reach the true top and bottom lines regardless of scrolloff. `M` on a buffer shorter than the window goes to the middle of the lines shown instead of the last line. Verified against real Neovim.
- Added the special marks `'[` `']` `'<` `'>` and their backtick forms. The change marks bracket the last changed or yanked text: a yank or put brackets the text, an insert runs from where typing began to where it stopped, a delete leaves both marks at the deleted spot, `J` spans the joined line, and undo brackets the restored lines. A freshly opened file is bracketed whole, as in Vim. The visual marks hold the last selection in buffer order, and a linewise selection spans its whole lines. Yanking a selection now also records it for `gv`. Verified against real Neovim.
- Added the rest of the `z` scroll family. `z<CR>`, `z.`, and `z-` work like `zt`, `zz`, and `zb` but also move the cursor to the first non-blank, and all six now take a count that goes to that line first, so `5zt` puts line 5 at the top. With wrap off, `zh` and `zl` scroll the view sideways by a count of columns, `zH` and `zL` by half a screen, and `zs` and `ze` put the cursor column at the left or right edge. The cursor is dragged along to stay visible, as in Vim. Verified against real Neovim where the snapshot can see it; the horizontal offset itself is pinned by native tests. Along the way, relative line numbers, the cursor-line highlight, and the view itself no longer lag one key behind after `gp` of a multi-line register, after cancelling a search with Esc, or after the visual-mode operators. They were drawn from a stale copy of the cursor; every key now refreshes that copy.
- Saving a file opened through a symlink now writes to the link's target and keeps the link, following Vim's `backupcopy=auto` rule. The atomic save used to replace the link itself with a regular file, which quietly disconnected symlinked dotfiles from the repository they point into. A dangling link gets its target created.
- `:new` and `:touch` on a file that already exists now open it, like Vim's `:new file`, instead of truncating it to zero bytes on the way to opening it. New files are still created on disk immediately.
- Added `]p` and `[p`, plus the `[P` and `]P` spellings, to paste with the indent adjusted to the current line. The first pasted line takes the current line's indent and the others keep their indent relative to it; empty lines stay empty and an indent never goes negative. A characterwise register keeps its first line in place and adjusts the lines after it, like Neovim. Works with counts, named registers, undo, and `.`. Along the way, a plain `p` of a multi-line characterwise register now leaves the cursor on the first pasted character like Vim instead of one column to its right. Verified against real Neovim.
- Added the insert-mode `Ctrl+e` and `Ctrl+y` to insert the character below or above the cursor. The match is by screen column, so tabs and wide characters copy what you see under the cursor, and a press with nothing to copy does nothing. The copied character goes into the redo buffer like Vim, so `.` inserts the same text rather than copying again. While the completion popup is open, `Ctrl+y` accepts the selected item and `Ctrl+e` closes the popup. Verified against real Neovim.
- Added the Neovim 0.11 defaults `[<Space>` and `]<Space>` to add empty lines above or below the cursor line, with a count, dot repeat, and one-step undo, and `[b` and `]b` to move through buffers with a count. A key that is waiting for its second half (`[`, `d`, `g`) now keeps the next press, so the Space leader no longer swallows `[<Space>`. Blank lines verified against real Neovim.
- Session persistence now also covers the numbered delete-history registers `"1`-`"9` and the jumplist. After a restart, `"1p` recovers the last session's delete and `Ctrl+o` walks back through its navigation history. Jumps in unsaved scratch buffers stay session-only, and the jumplist is capped at 100 entries like the in-editor list.
- Added `g*` and `g#`, the word searches that also match inside longer words. The plain word goes into the pattern and the history, so `n`, `N`, and `/` then Up all keep the looser match. Verified against real Neovim.
- Added the visual-mode operators `gu`, `gU`, `g~`, `r{char}`, `J`, `gJ`, and `=` on a selection. `r{char}` replaces every selected character, `J` and `gJ` join the selected lines (at least two), and `=` re-indents them. Along the way, the existing `u`, `U`, and `~` on a block selection now change only the block instead of everything between its corners, and all case operators leave the cursor at the start of the selection like Vim. Verified against real Neovim in a new visual oracle category, except `=`, which follows Nevi's own indenter.
- `:q!`, `:wq`, `:x`, `ZZ`, and `ZQ` no longer exit while another buffer still has unsaved changes. Like Vim, they finish with the current buffer (saved or discarded as asked) and then show the first buffer that still needs a write, with the `No write since last change for buffer` message. `:qa!` remains the way to discard everything. `:wq`, `:x`, and `ZZ` in a split now close just that pane instead of exiting the editor. (#316)
- Added `ZQ` to quit without saving, the same as `:q!`, and `Ctrl+^` to switch to the alternate buffer, the one the window showed before the current one. `Ctrl+^` reopens the file if its buffer was closed, and reports "No alternate file" when there is none. `Ctrl+6` works too, since that is the byte most terminals send for `Ctrl+^`.
- Added `Ctrl+a` and `Ctrl+x` to add to or subtract from the number at or after the cursor, with a count. Follows Neovim's default `nrformats=bin,hex`: decimal with a minus sign, `0x` hex, `0b` binary, leading zeros keep their width, hex digits keep their case. Works with `.` and undoes in one step. Verified against real Neovim in a new oracle category.
- A count before `i` or `a` now repeats the typed text like Vim, so `3ix<Esc>` gives `xxx`. `I`, `A`, `o`, and `O` already did this; the count is now set in one place for all six. (#296)
- `*` and `#` now search for the whole word only, like Vim's `\<word\>`, so `*` on `abc` no longer stops inside `abcdef`. Everything that reuses the pattern follows along: `n`, `N`, `gn`, `gN`, the highlights, the match counter, and an empty `/` prompt. The pattern also goes into the search history, so `/` then Up recalls it. Typing `\<` and `\>` in a search works the same way; plain patterns still match inside words. (#310)
- Search now anchors at the position where `/` or `?` was pressed, like Vim. Every keystroke evaluates the whole pattern from that origin instead of chasing the cursor around, so editing the pattern with backspace cannot land on an earlier match than Vim would. Escape cancels back to the original cursor and view, and a pattern with no match leaves the cursor where the search started.
- Repeating a search with an empty prompt (`/` or `?` then Enter) now moves off a match under the cursor instead of standing still, and it updates the direction that `n` and `N` follow, so `n` after `?` and Enter keeps searching backward.
- `n` and `N` now take a count, so `3n` jumps three matches ahead.
- `gN` now leaves the cursor on the start of the selected match like Vim. It used to sit on the end, same as `gn`.
- Extended Vim oracle coverage to the search family: `/`, `?`, `n`, `N`, `*`, `#`, `gn`, `gN`, and the search prompt editing keys (`Backspace`, `Ctrl+w`, `Ctrl+u`, `Ctrl+b`, `Ctrl+e`, `Ctrl+r`, history recall with `Up`), all verified against real Neovim and tracked in `PARITY.md`.
- Fixed the quote text objects (`i"`, `a"`, `i'`, `a'`, `` i` ``, `` a` ``) doing nothing when the cursor sits before the first quote. They now seek forward on the line, as in Vim, so `ci"` works from the start of the line.
- Fixed `a"` and friends not spanning surrounding whitespace. They now take the trailing whitespace after the closing quote, or the leading whitespace when there is none trailing.
- Fixed `ip` and `ap` on a blank line operating on the next paragraph instead of the blank lines themselves, and `ap` after a paragraph now takes all trailing blank lines instead of just one.
- Extended Vim oracle coverage to macros (record, replay, `@@`, counts), the register family (named, append, black hole, `"0`, `".`), insert-mode editing keys (`Backspace`, `Ctrl+w`, `Ctrl+a`, `Ctrl+r`, `Ctrl+[`), and the visual-mode basics including `gv`, all verified against real Neovim and tracked in `PARITY.md`.
- Extended Vim oracle coverage with a dedicated text-objects category: words, WORDs, all quote styles, all bracket pairs and their aliases, paragraphs, sentences, and tags, verified against real Neovim and tracked in `PARITY.md`. Also fixed the oracle harness dropping Vim's `<lt>` notation on the Neovim side.
- Added insert-mode `Ctrl+v` (and its `Ctrl+q` alias) to insert the next key literally, so `i` `Ctrl+v` `Ctrl+y` puts the raw `0x19` byte in the file like Vim instead of typing `vy`. The literal key bypasses auto-pairs and insert-mode remaps, and a literal `Tab` inserts a real tab character even with space indentation. Unhandled control chords in insert mode no longer type their bare letter; like Vim they now do nothing. Verified against real Neovim in the oracle suite. (#281)
- The mouse now works like nvim with `mouse=nvi`: the wheel scrolls the file instead of the terminal scrollback, targeting the pane under the pointer, 3 lines per tick. Horizontal wheel scrolls 6 columns with wrap off. Left click focuses a pane and moves the cursor (insert mode stays insert, a plain click drops visual mode), the wheel and clicks also drive the explorer selection, and the wheel scrolls the finder and markdown previews. On by default; `mouse = false` under `[editor]` or `:set nomouse` / `:set mouse=` turns it off, which also makes `mouse` the first option `:set` actually applies. While captured, terminal-native selection needs the terminal's bypass key (Option in iTerm2, Shift elsewhere). (#274)
- Added `Ctrl+e` / `Ctrl+y` to scroll the view one line (or a count of lines) without moving the cursor until it would leave the screen. Verified against real Neovim in the oracle suite, including the scrolloff edge cases at the top and bottom of the file.
- `cw` and `cW` now follow Vim's special case (`:h cw`): on a non-blank they change only up to the end of the word, leaving the trailing whitespace in place, and on the last character of a word they change just that character. On whitespace they still behave like `dw` plus insert. (#272)
- Added the method motions `[m`, `]m`, `[M`, and `]M`. They jump between tree-sitter function boundaries instead of using Vim's brace heuristic, so they land on real functions and methods in Rust-style code. They work with operators (`d]m`, `y[m`) and counts, and do nothing in files without tree-sitter support.
- Added dot repeat. `.` replays the last change at the cursor: operators with motions, character edits, insert sessions with their typed text, joins, pastes, and replace mode. A count replaces the change's original count (`3.`), the new count is remembered for the next repeat, and one `u` reverts a whole repeated change. Changes made through visual mode are not repeated yet.
- Fixed `w` and `W` at the last word of the buffer. They jumped to column 0 of the last line instead of stopping on the buffer's last character, and `dw` there deleted one character short of the word end.
- Extended Vim oracle coverage to paragraph and sentence motions, counted `G`, `-`, local marks, the jump and change lists (`Ctrl+o`, `Ctrl+i`, `''`, `g;`, `g,`, `'.`, `'^`), and `gi`, all now individually tracked in `PARITY.md`.
- Fixed `{` overshooting: from inside a paragraph it jumped past the blank line directly above to an earlier one. It now stops at the nearest blank line, as in Vim.
- Fixed `gi` resuming insert one column left of where the last insert session stopped. It now continues exactly where typing ended, including after `A` at the end of a line.
- Fixed `~` to advance the cursor past the last toggled character, as in Vim.
- Extended Vim oracle coverage to the editing core: the case operators (`gu`, `gU`, `g~` and their line forms), `~`, `X`, `s`, `S`, `gp`, `gP`, `J`, and `gJ` are now verified against real Neovim and individually tracked in `PARITY.md`.
- Corrected the docs: `.` (repeat last change) was listed as implemented but only shows a status note today. It moved to the roadmap.

### Interface

- Added `nevi --help` and `nevi -h`. They print the usage for the plain, `view`, `diff`, and `pick` forms and exit. Before this, `--help` was taken as a filename and opened an empty buffer called `--help`.
- Completion, hover, signature help, code action, and diagnostic popups now open next to the cursor's real screen row when soft wrap is on. They used to count buffer lines instead of screen rows, so in a narrow split with wrapped lines above the cursor the completion menu could open on top of the line being typed. The cursor and every popup now share one screen position calculation. (#333)
- `:rename` and `:mv` now refuse a destination that already exists instead of silently replacing it. A case-only rename such as `Notes.txt` to `notes.txt` still works on case-insensitive filesystems.
- The `:Keymaps` cheatsheet caught up with the editor. It had quietly stopped being updated in June and was missing everything added since: the method motions, section and unmatched-bracket motions, `gm`, `go`, `s`, several window and terminal session keys, and more. A new test now fails whenever a key documented in KEYBINDINGS.md is missing from the cheatsheet, so it cannot drift again.

- Rich mode no longer draws the `~` column on rows past the end of the buffer, like Vim's `fillchars=eob:' '`. Minimal mode keeps the tildes.
- The statusline now shows a `[3/12]` match counter on the right side after a search (`/`, `?`, `n`, `N`, `*`, `#`), in both rich and minimal layouts. It clears together with the search highlights when the cursor moves.
- Unified the remaining floating windows under the rich chrome. The leader/which-key popup and the command suggestions/history popup are now rounded-corner boxes with icon border titles and right-aligned key hints, and the command popup marks the selected row with the finder's accent bar instead of `>`. The floating terminal's corners now come from the shared glyph table (square in minimal mode, matching the finder) and its title carries a terminal icon. Minimal mode keeps the previous flat headers throughout.
- Added a start screen. Launching `nevi` with nothing to edit now shows recent files with their project names, harpoon pins, startup time, and key hints; `1`–`9` opens the numbered recent file and `h` + `1`–`9` jumps to that harpoon slot. Recently opened files persist to `~/.local/state/nevi/recent_files.json`, alongside the other shada-lite state. The screen is a pure render condition — it disappears as soon as any real buffer, edit, or mode change happens, and costs nothing afterward.
- Opening a file now scopes the project to its repository root (nearest ancestor with `.git`) instead of the file's own directory. Harpoon pins land in one `.nevi/harpoon.json` at the repo root regardless of which file you opened, and the explorer and floating terminal start at the repo root too. Outside a repository the old parent-directory behavior is unchanged.
- Added an optional bufferline. With `bufferline = true` under `[editor]` (default off), a tab strip on the top row lists every open buffer by file name, highlights the buffer shown in the active pane, and marks unsaved buffers with the same modified glyph the statusline uses. The strip is derived from the buffer list, so opening, closing, and `:bd` update it immediately, and navigation stays on the existing `]b`/`[b` and `:bn`/`:bp`. When enabled, buffer text starts one row lower to make room, in both split orientations.

### Configuration

- Added `sign_column` to `[editor]` with the same values as nvim's `signcolumn`. `auto` shows the two-cell git/diagnostic gutter only while the buffer has something to mark, so a plain file renders flush left like `nvim --clean`. `yes` (the default, unchanged behavior) always reserves it, and `no` hides it. `:set signcolumn=auto` switches it at runtime. Completion, hover, signature help, and code action popups now share the cursor's gutter math instead of their own, so they line up with the cursor whether the sign column or line numbers are on or off, and signature help and code actions now account for the pane's position in a split. (#330)
- Fixed `[ruby]` in `languages.toml` being ignored. Ruby files (`.rb`, `.rake`, `.gemspec`, `.ru`, `.podspec`) resolved to their raw extension instead of the `ruby` key, so formatter and tab width settings never applied. The generated `languages.toml` template now includes a commented Ruby example. (#273)

### Fixed

- After `:bd`, highlighting and `=` could keep using the closed buffer's syntax tree when the two buffers happened to share a version number. Closing a buffer now reparses the one that becomes current. (#309)

### Documentation

- Reorganized the docs on GitHub. The README is now a short front door with a table of contents, one install section, a six-command quick start, a table of every file Nevi reads or writes, and a troubleshooting section. New `CONFIGURATION.md` documents every config option, keymap action, and formatter setting in one place (#207), and new `CONTRIBUTING.md` walks through adding a keybind: the oracle case, the coverage entry, the docs that tests check, and the PR checklist. Keybind customization moved from `KEYBINDINGS.md` to `CONFIGURATION.md`, and the command tables gained the ex commands they were missing. A Vim parity issue template asks for exactly the fields an oracle case needs.


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
