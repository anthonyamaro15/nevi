# Keybind Roadmap

Nevi aims for full vim/neovim keybind compatibility. Defaults follow Neovim, and
keybinds are configurable — sensible defaults out of the box, overridable to your
own taste.

**Status: 238 keybinds implemented, 49 planned.**

This file tracks what's **planned** (not yet implemented). For the full list of
keybinds that already work, see [KEYBINDINGS.md](KEYBINDINGS.md).

> **Missing a keybind?** [Open an issue](https://github.com/anthonyamaro15/nevi/issues)
> and tell me which one your hands reach for — or grab one from the list below and
> send a PR. Contributions are very welcome.

---

## Planned

### Motions
- `(` / `)` — move to previous / next sentence
- `+` / `-` — move to first non-blank of next / previous line
- `gj` / `gk` — move down / up by *display* line (for wrapped lines)
- `g0` / `g$` / `g^` — start / end / first non-blank of the display line
- `''` / `` `` `` — jump to line / exact position before the last jump
- `'.` / `` `. `` — jump to line / exact position of the last change
- `'^` / `` `^ `` — jump to line / exact position of the last insert

### Text objects
- `ip` / `ap` — inner / around paragraph
- `is` / `as` — inner / around sentence
- `it` / `at` — inner / around HTML/XML tag

### Editing
- `gp` / `gP` — paste and leave the cursor after the pasted text
- `==` / `={motion}` — auto-indent current line / with a motion
- `gn` / `gN` — search forward / backward and select the match

### LSP / navigation
- `gD` — go to declaration
- `gI` — go to implementation
- `gf` — open the file under the cursor
- `gx` — open the URL under the cursor

### Window management
- `<C-w>=` — make all windows equal size
- `<C-w>r` / `<C-w>R` — rotate windows down-right / up-left
- `<C-w>x` — exchange the current window with the next

### Insert mode
- `<C-o>` — run one normal-mode command, then return to insert
- `<C-r>{reg}` — insert the contents of a register
- `<C-t>` / `<C-d>` — increase / decrease indent of the current line
- `<C-a>` — insert the previously inserted text

### Visual block
- `O` — move to the other corner of the block

### Finder preview
- `<C-d>` / `<C-u>` — scroll the preview down / up

### Registers (read-only)
- `".` — last inserted text
- `"%` — current filename
- `":` — last command
- `"#` — alternate filename
- `"=` — expression register

### Buffers
- `:bd` — delete (close) a buffer *(command parsing not wired up yet)*

---

*Everything already implemented is documented in [KEYBINDINGS.md](KEYBINDINGS.md).
This roadmap is updated as planned keybinds land.*
