# Nevi

[![CI](https://github.com/anthonyamaro15/nevi/actions/workflows/rust.yml/badge.svg)](https://github.com/anthonyamaro15/nevi/actions/workflows/rust.yml)
[![Release](https://img.shields.io/github/v/release/anthonyamaro15/nevi)](https://github.com/anthonyamaro15/nevi/releases/latest)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

A fast, Neovim-inspired terminal editor written in Rust.

*Your vim muscle memory, without the configuration overhead.*

![Nevi demo — file explorer with git status and diagnostic badges, LSP hover and goto-definition, vim motions, live grep, and live theme switching](nevi-demo.gif)

**372 keybinds implemented**, 206 of them verified against real Neovim on every
CI run. See the generated [parity scoreboard](PARITY.md).

- [Why Nevi](#why-nevi)
- [Features](#features)
- [Install](#install)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Themes](#themes)
- [Languages](#languages)
- [Keybindings](#keybindings)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)

## Why Nevi

I love Neovim and Zed. Zed is fast and modern but its vim mode is incomplete.
Neovim is powerful but gets slow once the plugin list grows. Helix asks you to
relearn Kakoune-style keys. Nevi is a native terminal editor where your existing
vim muscle memory just works, with LSP, tree-sitter, fuzzy finding, and git
built into the binary.

| Editor | Vim keybinds | Built-in features | Notes |
|--------|--------------|-------------------|-------|
| Neovim | Full | Via plugins | Powerful but plugin-dependent |
| Zed | Partial | Yes | Fast but vim mode incomplete |
| Helix | Kakoune-style | Yes | Different keybind philosophy |
| **Nevi** | 372 and counting | Yes | Test-backed vim compatibility |

## Features

- **Vim and Neovim keybindings**, including surround, comment, text objects,
  dot repeat, macros, and registers that survive restarts
- **Built-in LSP** for 12 languages, auto-detected on your PATH, with install
  hints when a server is missing
- **Tree-sitter highlighting** for 13 languages, with a large-file degradation
  mode
- **Telescope-style finder** for files, live grep, buffers, diagnostics, git
  changes, themes, and keymaps
- **File explorer** with git status tints and diagnostic badges
- **Floating terminal** sessions, lazygit, GitHub Copilot ghost text, Markdown
  preview, and previewed project-wide replace
- **Start screen** with recent files and harpoon pins
- **17 bundled themes** and a simple TOML config

> macOS and Linux today. Windows is planned
> ([#123](https://github.com/anthonyamaro15/nevi/issues/123)).

## Install

### Homebrew (macOS)

```bash
brew install anthonyamaro15/nevi/nevi
```

### Cargo (macOS and Linux)

```bash
cargo install --git https://github.com/anthonyamaro15/nevi
```

<details>
<summary>Linux build dependencies</summary>

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install -y build-essential cmake pkg-config libssl-dev zlib1g-dev \
  libx11-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev

# Fedora
sudo dnf install -y gcc gcc-c++ make cmake pkgconf-pkg-config openssl-devel \
  zlib-devel libX11-devel libxcb-devel

# Arch
sudo pacman -S --needed base-devel cmake pkgconf openssl zlib libx11 libxcb
```

Linux binary tarballs and distro packages (`.deb`, `.rpm`, AUR) are not
published yet.

</details>

### From source

```bash
git clone https://github.com/anthonyamaro15/nevi.git && cd nevi
cargo build --release
cp target/release/nevi ~/.local/bin/
```

### Update

```bash
brew update && brew upgrade nevi                                     # Homebrew
cargo install --git https://github.com/anthonyamaro15/nevi --force   # Cargo
nevi --version
```

Rust 1.85 or newer is required to build. Nothing else is required at runtime:
the file finder, live grep, fuzzy matching, and syntax highlighting are built
into the binary, so `ripgrep`, `fzf`, `fd`, and the `tree-sitter` CLI are not
needed. Optional tools such as language servers and `lazygit` are listed under
[Languages](#languages).

## Quick start

```bash
nevi .                        # open a directory
nevi src/main.rs              # open a file
nevi file1.rs file2.rs        # open several files
nevi view README.md           # read-only viewer
nevi diff before.rs after.rs  # side-by-side diff (stacked on narrow terminals)
nevi pick .                   # pick a path; Enter prints it, Esc cancels
nevi --help                   # usage for all of the above
```

| Key | Does |
|-----|------|
| `<Space>ff` | Find files |
| `<Space>fg` | Live grep |
| `<Space>e` | File explorer |
| `<Space>fk` | Search every keybinding with its description |
| `:w` / `:q` / `:wq` | Save, quit, save and quit, exactly like Vim |
| `:checkhealth` | Config paths, LSP status, missing tools |

Press `<Space>` on its own to see every leader shortcut.

## Configuration

| Path | What lives there |
|------|------------------|
| `~/.config/nevi/config.toml` | Editor, UI, keymaps, LSP, Copilot. Created with a commented template on first run. |
| `~/.config/nevi/languages.toml` | Per-language formatters and tab width |
| `~/.config/nevi/themes/` | Custom themes; a commented `_template.toml` is generated for you |
| `~/.local/state/nevi/state.json` | Macros, registers, global marks, search history, jumplist. Like Vim's shada. |
| `~/.local/state/nevi/recent_files.json` | Recent files for the start screen |
| `.nevi/harpoon.json` | Harpoon pins, at the repository root |

```toml
[editor]
tab_width = 2
relative_numbers = true
format_on_save = true

[theme]
colorscheme = "onedark"

# Remap a key: H to the first non-blank character
[[keymap.normal]]
from = "H"
to = "^"

# Add a leader shortcut: <Space>s saves every buffer
[[keymap.leader_mappings]]
key = "s"
action = ":wa"
desc = "Save all"
```

`:config` opens your config from inside Nevi and `:ConfigDefaults` shows the
current template. Nevi never rewrites a config file you own. Every option,
keymap action, and formatter setting is documented in
[**CONFIGURATION.md**](CONFIGURATION.md).

## Themes

`:theme <name>` sets a theme and `<Space>ft` opens a live picker. Bundled:
`onedark`, `onedark-darker`, `dracula`, `gruvbox`, `nord`, `tokyonight`,
`catppuccin-mocha`, `rose-pine`, `solarized-dark`, `kanagawa`, `monokai`,
`everforest`, `github-dark`, `github-light`, `ayu-dark`, `palenight`,
`nightfox`.

Drop a `.toml` file into `~/.config/nevi/themes/` and the filename becomes the
theme name. See [Custom themes](CONFIGURATION.md#custom-themes) in
CONFIGURATION.md.

## Languages

| Language | Highlighting | LSP server | Install |
|----------|--------------|------------|---------|
| Rust | Yes | rust-analyzer | `rustup component add rust-analyzer` |
| TypeScript / JavaScript | Yes | typescript-language-server | `npm install -g typescript typescript-language-server` |
| Python | Yes | pyright | `npm install -g pyright` |
| Go | Yes | gopls | `go install golang.org/x/tools/gopls@latest` |
| Ruby | Yes | ruby-lsp | `gem install ruby-lsp` |
| PHP | Yes | phpactor | PHP 8.2+, then see `:ToolInstall` |
| CSS / SCSS | Yes | vscode-css-language-server | `npm install -g vscode-langservers-extracted` |
| JSON | Yes | vscode-json-language-server | same package as CSS |
| HTML | Yes | vscode-html-language-server | same package as CSS |
| TOML | Yes | taplo | `cargo install taplo-cli --locked` |
| Shell / Bash | Yes | bash-language-server | `npm install -g bash-language-server` |
| Markdown | Yes | marksman (off by default) | |

Servers are detected on PATH. `:ToolInstall` prints the exact install commands
for anything missing, and `:checkhealth` shows what was found. `:LazyGit`
(`<Space>gg`) needs `lazygit` installed; git signs and `:GitChanges` need only
a Git repository.

External formatters such as biome, prettier, black, and gofmt are configured
in `languages.toml`. `:Format` and format-on-save use the formatter when one
is configured for the buffer's language and fall back to the LSP otherwise.

> **Missing a language?** Open a
> [GitHub issue](https://github.com/anthonyamaro15/nevi/issues).

## Keybindings

If it works in Vim, it should work here: motions, operators, text objects,
counts, registers, marks, macros, visual block, dot repeat, and `Ctrl+w`
window commands. On top of that:

- **LSP**: `gd` `gD` `gI` `gr` `K` `]d` `[d` `<Space>ca` `<Space>rn`
- **Surround and comment**: `ds` `cs` `ys` `gcc` `gc{motion}`
- **Leader**: `<Space>` then `ff` `fg` `fb` `e` `tt` `gg` `m` `h` and more

Full references:

- [KEYBINDINGS.md](KEYBINDINGS.md): every key and ex command
- [KEYBINDS_ROADMAP.md](KEYBINDS_ROADMAP.md): planned Vim defaults and
  deliberate deviations
- [PARITY.md](PARITY.md): which keys are verified against real Neovim

## Troubleshooting

- **Boxes instead of icons?** Install a [Nerd Font](https://www.nerdfonts.com),
  or set `[ui] style = "minimal"` for a plain ASCII UI with the same layout.
- **Can't select text with the mouse?** Nevi captures the mouse like Neovim's
  `mouse=nvi`. Hold Option (iTerm2) or Shift (most terminals) to select with
  the terminal, run `:set mouse=` for the session, or set `mouse = false`
  under `[editor]`. Why this is the default:
  [ADR 0001](docs/adr/0001-mouse-capture-on-by-default.md).
- **LSP not starting?** `:checkhealth` shows what was found on PATH and
  `:ToolInstall` shows the install commands. Note that rust-analyzer's cold
  index takes minutes on a large crate and is not Nevi latency.
- **Feels slow?** `:WhySlow` opens recent in-memory timings. For a full
  profile, `NEVI_PROFILE=1 nevi path/to/file` writes `/tmp/nevi_profile.log`
  on exit.
- **Very large file?** The statusline shows `[large]` when highlighting is
  degraded, and `:checkhealth` reports the threshold.

## Contributing

Issues and PRs are welcome. Found a key that behaves differently from Neovim?
Open a
[Vim parity report](https://github.com/anthonyamaro15/nevi/issues/new?template=vim-parity.yml)
and it becomes a test case. Setup, test gates, and how to add a keybind are in
[**CONTRIBUTING.md**](CONTRIBUTING.md).

## License

MIT License. Inspired by [Neovim](https://neovim.io/),
[Helix](https://helix-editor.com/), and [Zed](https://zed.dev/).

Built with [ropey](https://github.com/cessen/ropey),
[tree-sitter](https://tree-sitter.github.io/tree-sitter/),
[crossterm](https://github.com/crossterm-rs/crossterm),
[nucleo](https://github.com/helix-editor/nucleo), and
[git2](https://github.com/rust-lang/git2-rs).
