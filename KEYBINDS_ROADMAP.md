# Keybind Roadmap

Nevi aims for full vim/neovim keybind compatibility. Defaults follow Neovim, and
keybinds are configurable — sensible defaults out of the box, overridable to your
own taste.

**Status: 311 keybinds implemented, 56 planned Vim/Neovim parity defaults.**

This file tracks what's **planned** (not yet implemented). For the full list of
keybinds that already work, see [KEYBINDINGS.md](KEYBINDINGS.md).

> **Missing a keybind?** If you do not see a keybind you want to use, or you
> notice one that is missing, please [open an issue](https://github.com/anthonyamaro15/nevi/issues)
> and tell me which one your hands reach for — or grab one from the list below and
> send a PR. Contributions are very welcome.

---

## Planned

The defaults below should behave like Vim/Neovim out of the box. Users can still
override them through configuration, but the shipped/default keybind behavior should
remain Vim-compatible unless Nevi intentionally documents a difference.

### Command-Line Mode Defaults

These apply while editing the command prompt after `:`.

| Keybind | Planned behavior |
|---------|------------------|
| `q:` | Open command-line history in the command-line window |
| `q/` | Open `/` search history in the command-line window |
| `q?` | Open `?` search history in the command-line window |

### Window Management Extras

Nevi already has the core split/navigation defaults. These cover the remaining
Vim window sizing and window-moving defaults.

| Keybind | Planned behavior |
|---------|------------------|
| `Ctrl+w H` | Move current window to the far left |
| `Ctrl+w J` | Move current window to the bottom |
| `Ctrl+w K` | Move current window to the top |
| `Ctrl+w L` | Move current window to the far right |

### Advanced Motions

These are Vim/Neovim defaults that are useful but lower priority than command-line
mode parity.

| Keybind | Planned behavior |
|---------|------------------|
| `\|` | Go to a screen column |
| `g_` | Go to the last non-blank character of the line |
| `gm` | Go to the middle of the screen line |
| `gM` | Go to the middle of the text line |
| `go` | Go to a byte offset |
| `[[` | Go to previous section start |
| `]]` | Go to next section start |
| `[]` | Go to previous section end |
| `][` | Go to next section end |
| `[{` | Go to previous unmatched `{` |
| `]}` | Go to next unmatched `}` |
| `[(` | Go to previous unmatched `(` |
| `])` | Go to next unmatched `)` |
| `[m` | Go to previous method/function start |
| `]m` | Go to next method/function start |
| `[M` | Go to previous method/function end |
| `]M` | Go to next method/function end |

### Larger Feature Areas

These are Vim/Neovim defaults, but each likely needs supporting editor
infrastructure rather than just a key handler.

| Area | Planned defaults |
|------|------------------|
| Tabs | `gt`, `gT`, `{n}gt`, `:tabnew`, `:tabclose`, `:tabnext`, `:tabprev` |
| Folds | `za`, `zo`, `zc`, `zO`, `zC`, `zM`, `zR`, `zf`, `zd`, `zE`, `zj`, `zk` |
| Tags / tag stack | `Ctrl+]`, `Ctrl+t`, `:tag`, `:tags` |
| Quickfix-style lists | `:copen`, `:cclose`, `:cnext`, `:cprev`, `[q`, `]q` |
| Introspection commands | `:jumps`, `:registers`, `:history` |

---

*Everything already implemented is documented in [KEYBINDINGS.md](KEYBINDINGS.md).
This roadmap is updated as planned keybinds land.*
