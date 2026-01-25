# Nevi Keybindings

Complete reference for all keybindings in Nevi. If you're familiar with Vim/Neovim, most of these will feel natural.

## Customization

All available keybindings in this doc work out of the box, but you can customize them in `~/.config/nevi/config.toml`.

### Changing the Leader Key

The leader key is `Space` by default. To change it:

```toml
[keymap]
leader = ","  # Use comma as leader instead
```

### Remapping Keys in Normal Mode

Want `H` to go to the start of the line and `L` to go to the end? Add this:

```toml
[[keymap.normal]]
from = "H"
to = "^"

[[keymap.normal]]
from = "L"
to = "$"
```

Now pressing `H` does what `^` normally does (jump to first non-blank character).

### Adding Leader Shortcuts

Create your own `<leader>` + key combinations:

```toml
[[keymap.leader_mappings]]
key = "s"
action = ":wa"
desc = "Save all files"
```

Now `<Space>s` saves all open files.

### Key Notation

When specifying keys, use these formats:
- Regular keys: `"a"`, `"H"`, `";"`, `"0"`
- Control: `"<C-s>"` (Ctrl+s)
- Special keys: `"<CR>"` (Enter), `"<Esc>"`, `"<Tab>"`, `"<Space>"`, `"<BS>"` (Backspace)

> **Tip:** The config file at `~/.config/nevi/config.toml` is auto-generated on first run and contains a full reference with all options commented out.

---

## Table of Contents

- [Normal Mode](#normal-mode)
  - [Basic Movement](#basic-movement)
  - [Word Movement](#word-movement)
  - [Line Movement](#line-movement)
  - [File Movement](#file-movement)
  - [Screen Movement](#screen-movement)
  - [Find Character](#find-character)
  - [Scrolling](#scrolling)
  - [Jump List](#jump-list)
  - [Change List](#change-list)
- [Editing](#editing)
  - [Operators](#operators)
  - [Delete/Change/Yank](#deletechangeyank)
  - [Undo/Redo](#undoredo)
  - [Indent](#indent)
  - [Case](#case)
  - [Join Lines](#join-lines)
- [Search](#search)
- [Marks](#marks)
- [Macros](#macros)
- [Insert Mode](#insert-mode)
- [Visual Mode](#visual-mode)
- [Text Objects](#text-objects)
- [Registers](#registers)
- [LSP](#lsp)
- [Surround](#surround)
- [Comment](#comment)
- [Window Management](#window-management)
- [Leader Key Mappings](#leader-key-mappings)
- [Finder/Picker (Telescope-like)](#finderpicker-telescope-like)
- [Harpoon-like Quick Files](#harpoon-like-quick-files)
- [Commands](#commands)

---

## Normal Mode

### Basic Movement

| Key | Action |
|-----|--------|
| `h` | Move cursor left |
| `j` | Move cursor down |
| `k` | Move cursor up |
| `l` | Move cursor right |

### Word Movement

| Key | Action |
|-----|--------|
| `w` | Move to start of next word |
| `W` | Move to start of next WORD (whitespace-delimited) |
| `b` | Move to start of previous word |
| `B` | Move to start of previous WORD |
| `e` | Move to end of word |
| `E` | Move to end of WORD |
| `ge` | Move to end of previous word |
| `gE` | Move to end of previous WORD |

> **Word vs WORD:** A "word" is letters/numbers/underscores. A "WORD" is anything separated by whitespace. For example, in `foo-bar`, `w` stops at `-`, but `W` jumps over the whole thing.

### Line Movement

| Key | Action |
|-----|--------|
| `0` | Move to start of line (column 0) |
| `^` | Move to first non-blank character |
| `$` | Move to end of line |
| `{` | Move to previous blank line (paragraph) |
| `}` | Move to next blank line (paragraph) |

### File Movement

| Key | Action |
|-----|--------|
| `gg` | Move to start of file |
| `G` | Move to end of file |
| `{n}G` | Move to line n (e.g., `50G` goes to line 50) |
| `%` | Jump to matching bracket `()`, `{}`, `[]` |

### Screen Movement

| Key | Action |
|-----|--------|
| `H` | Move to top of visible screen |
| `M` | Move to middle of visible screen |
| `L` | Move to bottom of visible screen |

### Find Character

| Key | Action |
|-----|--------|
| `f{char}` | Find character forward on current line |
| `F{char}` | Find character backward on current line |
| `t{char}` | Move till (before) character forward |
| `T{char}` | Move till (after) character backward |
| `;` | Repeat last `f`/`F`/`t`/`T` |
| `,` | Repeat last `f`/`F`/`t`/`T` in reverse |

> **Example:** `fa` moves cursor to the next `a`. `ta` moves cursor to just before the next `a`.

### Scrolling

| Key | Action |
|-----|--------|
| `Ctrl+f` | Scroll page down |
| `Ctrl+b` | Scroll page up |
| `Ctrl+d` | Scroll half page down |
| `Ctrl+u` | Scroll half page up |
| `zz` | Center cursor on screen |
| `zt` | Move cursor line to top of screen |
| `zb` | Move cursor line to bottom of screen |

### Jump List

Nevi tracks where you jump from, so you can navigate back and forth.

| Key | Action |
|-----|--------|
| `Ctrl+o` | Jump to older position |
| `Ctrl+i` | Jump to newer position |

### Change List

Navigate through positions where you made edits.

| Key | Action |
|-----|--------|
| `g;` | Jump to older change position |
| `g,` | Jump to newer change position |
| `gi` | Go to last insert position and enter insert mode |

---

## Editing

### Operators

Operators are commands that wait for a motion. For example, `d` (delete) + `w` (word) = `dw` (delete word).

| Operator | Action |
|----------|--------|
| `d` | Delete |
| `c` | Change (delete and enter insert mode) |
| `y` | Yank (copy) |
| `>` | Indent right |
| `<` | Indent left |
| `gu` | Lowercase |
| `gU` | Uppercase |
| `g~` | Toggle case |

### Delete/Change/Yank

| Key | Action |
|-----|--------|
| `dd` | Delete entire line |
| `D` | Delete from cursor to end of line |
| `cc` | Change entire line |
| `C` | Change from cursor to end of line |
| `yy` | Yank entire line |
| `Y` | Yank entire line |
| `x` | Delete character under cursor |
| `X` | Delete character before cursor |
| `p` | Paste after cursor |
| `P` | Paste before cursor |
| `r{char}` | Replace character under cursor |
| `.` | Repeat last change |

> **Examples:**
> - `dw` - Delete from cursor to start of next word
> - `d$` - Delete from cursor to end of line
> - `diw` - Delete inner word (the word cursor is on)
> - `ci"` - Change inside quotes
> - `yap` - Yank around paragraph

### Undo/Redo

| Key | Action |
|-----|--------|
| `u` | Undo |
| `Ctrl+r` | Redo |

### Indent

| Key | Action |
|-----|--------|
| `>>` | Indent current line |
| `<<` | Dedent current line |
| `>{motion}` | Indent with motion (e.g., `>j` indents current and next line) |
| `<{motion}` | Dedent with motion |

### Case

| Key | Action |
|-----|--------|
| `~` | Toggle case of character under cursor |
| `gu{motion}` | Lowercase with motion |
| `guu` | Lowercase entire line |
| `gU{motion}` | Uppercase with motion |
| `gUU` | Uppercase entire line |
| `g~{motion}` | Toggle case with motion |
| `g~~` | Toggle case of entire line |

### Join Lines

| Key | Action |
|-----|--------|
| `J` | Join current line with line below (with space) |
| `gJ` | Join lines without adding space |

---

## Search

| Key | Action |
|-----|--------|
| `/` | Search forward |
| `?` | Search backward |
| `n` | Go to next match |
| `N` | Go to previous match |
| `*` | Search word under cursor forward |
| `#` | Search word under cursor backward |

> **Tip:** Search supports regex. Use `\c` at the start for case-insensitive search (e.g., `/\cfoo`).

---

## Marks

Marks let you save positions and jump back to them later.

| Key | Action |
|-----|--------|
| `m{a-z}` | Set local mark (buffer-specific) |
| `m{A-Z}` | Set global mark (works across files) |
| `'{a-z}` | Jump to line of local mark |
| `` `{a-z} `` | Jump to exact position of local mark |
| `'{A-Z}` | Jump to line of global mark |
| `` `{A-Z} `` | Jump to exact position of global mark |

**Commands:**
- `:marks` - Show all marks in interactive picker
- `:delmarks a` - Delete mark `a`
- `:delmarks a-d` - Delete marks `a` through `d`
- `:delmarks!` - Delete all lowercase marks in current buffer

> **Tip:** Use lowercase marks (`a-z`) for positions within a file, uppercase marks (`A-Z`) for jumping between files.

---

## Macros

Record and replay sequences of commands.

| Key | Action |
|-----|--------|
| `q{a-z}` | Start recording macro into register |
| `q` | Stop recording (when recording) |
| `@{a-z}` | Play macro from register |
| `@@` | Replay last executed macro |
| `{n}@{a-z}` | Play macro n times |

> **Example:** `qa` starts recording into register `a`. Make your edits, press `q` to stop. Then `@a` replays it. `5@a` replays it 5 times.

---

## Insert Mode

| Key | Action |
|-----|--------|
| `i` | Insert before cursor |
| `a` | Insert after cursor |
| `I` | Insert at first non-blank of line |
| `A` | Insert at end of line |
| `o` | Open new line below |
| `O` | Open new line above |
| `gi` | Go to last insert position and enter insert mode |

**While in Insert Mode:**

| Key | Action |
|-----|--------|
| `Esc` or `Ctrl+[` | Exit insert mode |
| `Backspace` | Delete character before cursor |
| `Ctrl+w` | Delete word before cursor |
| `Ctrl+u` | Delete to start of line |

**Copilot (if enabled):**

| Key | Action |
|-----|--------|
| `Ctrl+l` | Accept Copilot suggestion |
| `Alt+]` | Next Copilot suggestion |
| `Alt+[` | Previous Copilot suggestion |

---

## Visual Mode

| Key | Action |
|-----|--------|
| `v` | Enter character-wise visual mode |
| `V` | Enter line-wise visual mode |
| `Ctrl+v` | Enter block visual mode |

**While in Visual Mode:**

| Key | Action |
|-----|--------|
| `Esc` | Exit visual mode |
| `d` | Delete selection |
| `c` | Change selection |
| `y` | Yank selection |
| `p` | Paste over selection |
| `o` | Swap to other end of selection |
| `>` | Indent selection |
| `<` | Dedent selection |
| `gc` | Toggle comment on selection |
| `S{char}` | Surround selection with character |
| `gv` | Reselect last visual selection (from normal mode) |

---

## Text Objects

Text objects define regions of text. Use them with operators (`d`, `c`, `y`, etc.).

**Inner vs Around:**
- `i` = inner (just the content)
- `a` = around (content + delimiters/whitespace)

| Text Object | Description |
|-------------|-------------|
| `iw` / `aw` | Inner/around word |
| `iW` / `aW` | Inner/around WORD |
| `i"` / `a"` | Inner/around double quotes |
| `i'` / `a'` | Inner/around single quotes |
| `` i` `` / `` a` `` | Inner/around backticks |
| `i(` / `a(` | Inner/around parentheses |
| `ib` / `ab` | Inner/around parentheses (alias) |
| `i{` / `a{` | Inner/around braces |
| `iB` / `aB` | Inner/around braces (alias) |
| `i[` / `a[` | Inner/around brackets |
| `i<` / `a<` | Inner/around angle brackets |
| `ip` / `ap` | Inner/around paragraph |

> **Examples:**
> - `ci"` - Change inside double quotes
> - `da(` - Delete around parentheses (including the parens)
> - `yiw` - Yank inner word
> - `vip` - Select inner paragraph

---

## Registers

Registers are like named clipboards. Prefix operations with `"{register}`.

| Register | Description |
|----------|-------------|
| `"a` - `"z` | Named registers |
| `"A` - `"Z` | Append to named registers |
| `"+` | System clipboard |
| `"*` | Selection clipboard (same as `+` on macOS) |
| `"_` | Black hole (delete without saving) |
| `"0` | Last yank |

> **Examples:**
> - `"ayy` - Yank line into register `a`
> - `"ap` - Paste from register `a`
> - `"+y` - Yank to system clipboard
> - `"+p` - Paste from system clipboard
> - `"_dd` - Delete line without saving to any register

---

## LSP

Language Server Protocol features for code intelligence.

| Key | Action |
|-----|--------|
| `gd` | Go to definition |
| `gr` | Find references |
| `K` | Show hover documentation |
| `gl` | Show diagnostic in floating window |
| `]d` | Go to next diagnostic |
| `[d` | Go to previous diagnostic |

**Via Leader:**
| Key | Action |
|-----|--------|
| `<leader>ca` | Code actions |
| `<leader>rn` | Rename symbol |
| `<leader>d` | Search all diagnostics |
| `<leader>D` | Show line diagnostic |

---

## Surround

Add, change, or delete surrounding pairs (quotes, brackets, etc.).

| Key | Action |
|-----|--------|
| `ds{char}` | Delete surrounding pair |
| `cs{old}{new}` | Change surrounding pair |
| `ys{motion}{char}` | Add surrounding pair |

**In Visual Mode:**
| Key | Action |
|-----|--------|
| `S{char}` | Surround selection |

> **Examples:**
> - `ds"` - Delete surrounding double quotes
> - `cs"'` - Change double quotes to single quotes
> - `ysiw"` - Surround word with double quotes
> - `yss)` - Surround entire line with parentheses
> - (Visual) `S]` - Surround selection with brackets

---

## Comment

Toggle comments on code.

| Key | Action |
|-----|--------|
| `gcc` | Toggle comment on current line |
| `gc{motion}` | Toggle comment with motion |

**In Visual Mode:**
| Key | Action |
|-----|--------|
| `gc` | Toggle comment on selection |

> **Examples:**
> - `gcc` - Comment/uncomment current line
> - `gcj` - Comment/uncomment current and next line
> - `gcip` - Comment/uncomment inner paragraph

---

## Window Management

Split and navigate between windows.

| Key | Action |
|-----|--------|
| `Ctrl+w v` | Split window vertically |
| `Ctrl+w s` | Split window horizontally |
| `Ctrl+w c` | Close current window |
| `Ctrl+w o` | Close all other windows |
| `Ctrl+w q` | Quit current window |
| `Ctrl+w w` | Move to next window |
| `Ctrl+w p` | Move to previous window |
| `Ctrl+w h` | Move to window on the left |
| `Ctrl+w j` | Move to window below |
| `Ctrl+w k` | Move to window above |
| `Ctrl+w l` | Move to window on the right |

> **Note:** Currently all splits share the same orientation (all vertical OR all horizontal). Mixed layouts like having one vertical split with a horizontal split inside it are not yet supported.

---

## Leader Key Mappings

The leader key is `Space` by default. Press `Space` followed by these keys:

### Files & Navigation (Telescope-like)

| Key | Action |
|-----|--------|
| `<leader>w` | Save file |
| `<leader>q` | Quit |
| `<leader>e` | Toggle file explorer |
| `<leader>ff` | Find files (fuzzy finder) |
| `<leader>fg` | Live grep (search in files) |
| `<leader>sw` | Search word under cursor |
| `<leader>fb` | Find buffers |
| `<leader>ft` | Theme picker |

> **Tip:** In the file finder or grep, press `Ctrl+t` to toggle a preview panel showing file contents.

### LSP

| Key | Action |
|-----|--------|
| `<leader>ca` | Code actions |
| `<leader>rn` | Rename symbol |
| `<leader>d` | Search diagnostics |
| `<leader>D` | Show line diagnostic |

### Git

| Key | Action |
|-----|--------|
| `<leader>gg` | Open lazygit |

### Harpoon-like Quick Files

| Key | Action |
|-----|--------|
| `<leader>m` | Add current file to harpoon |
| `<leader>h` | Open harpoon menu |
| `<leader>1` | Jump to harpoon slot 1 |
| `<leader>2` | Jump to harpoon slot 2 |
| `<leader>3` | Jump to harpoon slot 3 |
| `<leader>4` | Jump to harpoon slot 4 |

---

## Finder/Picker (Telescope-like)

When a finder popup is open (file finder, grep, buffers, etc.):

### Insert Mode (typing in search)

| Key | Action |
|-----|--------|
| `Ctrl+j` | Move to next result |
| `Ctrl+k` | Move to previous result |
| `Enter` | Open selected file |
| `Esc` | Switch to normal mode |
| `Ctrl+t` or `p` | Toggle preview panel |
| `Ctrl+d` | Scroll preview down |
| `Ctrl+u` | Scroll preview up |

### Normal Mode (navigating results)

| Key | Action |
|-----|--------|
| `j` / `k` | Move down/up |
| `g` | Go to first result |
| `G` | Go to last result |
| `Enter` | Open selected file |
| `Esc` | Close finder |
| `i` | Enter insert mode |
| `p` | Toggle preview |

---

## Harpoon-like Quick Files

Quick file switching for frequently used files (inspired by [harpoon.nvim](https://github.com/ThePrimeagen/harpoon)).

| Key | Action |
|-----|--------|
| `]h` | Go to next harpoon file |
| `[h` | Go to previous harpoon file |

See [Leader Key Mappings](#leader-key-mappings) for adding files and jumping to slots.

---

## Commands

Type `:` to enter command mode. Common commands:

### File Operations

| Command | Action |
|---------|--------|
| `:w` | Save file |
| `:wa` | Save all files |
| `:q` | Quit |
| `:q!` | Force quit (discard changes) |
| `:qa` | Quit all |
| `:qa!` | Force quit all |
| `:wq` | Save and quit |
| `:wqa` | Save all and quit |
| `:x` | Save (if modified) and quit |
| `:e {file}` | Edit/open a file |

### Navigation

| Command | Action |
|---------|--------|
| `:{n}` | Go to line n |
| `:FindFiles` | Open file finder |
| `:LiveGrep` | Search in files |
| `:FindBuffers` | Open buffer finder |
| `:FindDiagnostics` | Open diagnostics finder |
| `:Explorer` | Toggle file explorer |

### Buffers

| Command | Action |
|---------|--------|
| `:bn` | Next buffer |
| `:bp` | Previous buffer |
| `:bd` | Close buffer |

### Splits

| Command | Action |
|---------|--------|
| `:vs` | Vertical split |
| `:sp` | Horizontal split |

### LSP

| Command | Action |
|---------|--------|
| `:rn` | Rename symbol |
| `:codeaction` | Show code actions |

### Other

| Command | Action |
|---------|--------|
| `:Themes` | Open theme picker |
| `:theme {name}` | Set theme |
| `:LazyGit` | Open lazygit |
| `:marks` | Show marks picker |
| `:delmarks {m}` | Delete marks |
| `:HarpoonAdd` | Add to harpoon |
| `:HarpoonMenu` | Open harpoon menu |

---

## Missing a keybind?

If there's a vim keybind you use that's not implemented, please [open an issue](https://github.com/anthonyamaro15/nevi/issues) and we'll prioritize adding it!
