# Keybind Roadmap

Nevi aims for full vim/neovim keybind compatibility. Defaults follow Neovim, and
keybinds are configurable — sensible defaults out of the box, overridable to your
own taste.

**Status: 372 keybinds implemented, 55 planned Vim/Neovim parity defaults.**

The Status line above is the hand-maintained source that the parity report
reads to build [PARITY.md](PARITY.md). When a planned keybind lands, remove it
from the tables below, adjust both numbers, and regenerate the scoreboard with
`NEVI_UPDATE_PARITY=1 cargo test parity_report`.

This file tracks what's **planned** (not yet implemented). For the full list of
keybinds that already work, see [KEYBINDINGS.md](KEYBINDINGS.md). This list comes
from a full audit against Vim/Neovim's default keybinds, so an empty roadmap
means "no known missing default" — if a default is absent from both documents,
that is a bug in the audit, not a silent omission.

> **Missing a keybind?** If you do not see a keybind you want to use, or you
> notice one that is missing, please [open an issue](https://github.com/anthonyamaro15/nevi/issues)
> and tell me which one your hands reach for — or grab one from the list below and
> send a PR. Contributions are very welcome.

---

## Planned

The defaults below should behave like Vim/Neovim out of the box. Users can still
override them through configuration, but the shipped/default keybind behavior should
remain Vim-compatible unless Nevi intentionally documents a difference.

**Suggested priority.** There is no usage telemetry for Vim keybinds, so this
ordering leans on the common proxies (vimtutor and cheat-sheet staples, what
Neovim itself promotes to a default, plugin popularity like vim-unimpaired) and
on what Nevi users actually ask for in issues — user reports move a key up
immediately. The keys most hands reach for first: `gq` / `gw`, and among
the larger areas, folds and quickfix before tabs and tags.

### Editing

| Keybind | Planned behavior |
|---------|------------------|
| `U` | Undo all latest changes on the last changed line |
| `g-` | Go to older text state (undo over time) |
| `g+` | Go to newer text state |
| `gq{motion}` | Format the lines the motion moves over |
| `gqq` | Format the current line |
| `gw{motion}` | Format like `gq` but keep the cursor position |
| `gww` | Format the current line, keeping the cursor position |

### Command-Line Mode Defaults

These apply while editing the command prompt after `:`.

| Keybind | Planned behavior |
|---------|------------------|
| `q:` | Open command-line history in the command-line window |
| `q/` | Open `/` search history in the command-line window |
| `q?` | Open `?` search history in the command-line window |

### Larger Feature Areas

These are Vim/Neovim defaults, but each likely needs supporting editor
infrastructure rather than just a key handler.

| Area | Planned defaults |
|------|------------------|
| Tabs | `gt`, `gT`, `{n}gt`, `:tabnew`, `:tabclose`, `:tabnext`, `:tabprev` |
| Folds | `za`, `zo`, `zc`, `zO`, `zC`, `zM`, `zR`, `zf`, `zd`, `zE`, `zj`, `zk` |
| Tags / tag stack | `Ctrl+]`, `Ctrl+t`, `:tag`, `:tags` |
| Quickfix-style lists | `:copen`, `:cclose`, `:cnext`, `:cprev`, `[q`, `]q` |
| Spell checking | `[s`, `]s`, `z=`, `zg`, `zw` (needs a spell engine) |
| Introspection commands | `:jumps`, `:registers`, `:history` |

---

## Deliberate Deviations

These keys exist in Nevi but intentionally do something different from stock
Vim/Neovim. They are not planned for change; they are documented so the
difference is a decision, not an accident.

| Keybind | Nevi behavior | Stock Vim behavior |
|---------|---------------|--------------------|
| `]m` `[m` `]M` `[M` | Tree-sitter function boundaries | Brace-scanning heuristic that misfires in Rust-style code |
| `gI` | LSP go-to-implementation | Insert at column 1 |
| `S{char}` (visual) | Surround the selection | Substitute the selected lines |
| `gd` / `gr` | LSP definition / references | Local declaration / none |
| `Ctrl+n` / `Ctrl+p` (insert) | Navigate the completion popup | Keyword completion |

## Niche Defaults, Noted But Not Planned

Audited and deliberately left off the planned list. If your hands actually
reach for one of these, open an issue and it moves up.

`g?{motion}` / `g??` (ROT13), `ga` (char value), `g8` (UTF-8 bytes), `gF`
(open file at line number), `['` / `]'` / `` [` `` / `` ]` `` (previous/next
lowercase mark), `Ctrl+k` digraphs (insert), the `Ctrl+x` completion sub-mode
(insert; LSP completion covers this), `Ctrl+v` character-code entry
(`<C-v>123`, `<C-v>u1f600`; the literal-next-key form is implemented),
operator-forced motions (`dvj`, `dVj`), and Neovim 0.11's
`gri`/`grn`/`grr`/`gra`/`gO` LSP maps (covered by Nevi's own LSP keys above).

---

*Everything already implemented is documented in [KEYBINDINGS.md](KEYBINDINGS.md).
This roadmap is updated as planned keybinds land.*
