# Nevi

A fast, Neovim-inspired terminal editor written in Rust.

*Your vim muscle memory, without the configuration overhead.*

## Why Nevi?

**The Problem:**
I love Neovim and Zed. Zed is an amazing editor - fast, modern, and beautiful. But it doesn't have all the vim keybinds I use daily, which stopped me from fully migrating to it. Neovim is powerful but can become slow with plugins. Helix uses Kakoune-style keybindings which require relearning muscle memory.

**Nevi's Goal:**
A fast, native terminal editor where your existing vim/neovim muscle memory just works. No relearning keybindings, no plugin configuration overhead, no compromise on modern features like LSP and tree-sitter.

| Editor | Vim Keybinds | Built-in Features | Notes |
|--------|--------------|-------------------|-------|
| Neovim | Full | Via plugins | Powerful but plugin-dependent |
| Zed | Partial | Yes | Fast but vim mode incomplete |
| Helix | Kakoune-style | Yes | Different keybind philosophy |
| **Nevi** | In Progress | Yes | Aiming for full vim compatibility |

> **Note:** Nevi is under active development. Most common vim keybindings are implemented, with more being added regularly. See [Keybindings](#keybindings) for current status.

## Features

> **Note:** Currently macOS only. Linux and Windows support planned.

- **Vim/neovim keybindings** - Most common keybinds implemented, more being added regularly
- **Built-in LSP** - rust-analyzer, typescript-language-server, pyright, and more
- **Tree-sitter syntax highlighting** - Fast, accurate highlighting for Rust, TypeScript, JavaScript, Python, CSS, JSON, TOML, HTML, Markdown
- **Theme selection** - Multiple built-in colorschemes with easy switching
- **Fuzzy file finder** - Telescope-style file and content search
- **GitHub Copilot integration** - AI-powered completions
- **File explorer** - Built-in tree view
- **Git signs** - Gutter indicators for added/modified/deleted lines
- **Harpoon-style quick file switching** - Pin and jump to frequently used files
- **External formatter support** - Biome, Prettier, and other formatters
- **Split windows** - Vertical and horizontal splits
- **Configurable via TOML** - Simple, readable configuration

## Installation

### From Source

```bash
git clone https://github.com/anthonyamaro15/nevi.git
cd nevi
cargo build --release
./target/release/nevi
```

### Add to PATH (optional)

```bash
cp target/release/nevi ~/.local/bin/
# or
sudo cp target/release/nevi /usr/local/bin/
```

## Quick Start

```bash
# Open current directory
nevi .

# Open a file
nevi src/main.rs

# Open multiple files
nevi file1.rs file2.rs
```

**Basic commands:**
- `:w` - Save
- `:q` - Quit
- `:wq` - Save and quit
- `<Space>ff` - Find files
- `<Space>fg` - Live grep
- `<Space>e` - File explorer

## Configuration

Config location: `~/.config/nevi/config.toml`

A template config file is created automatically on first run. Here's an example:

```toml
[editor]
tab_width = 2
format_on_save = true
relative_numbers = true
scroll_off = 8

[theme]
colorscheme = "onedark"

[lsp]
enabled = true

[copilot]
enabled = true
```

See the generated config file at `~/.config/nevi/config.toml` for all available options with documentation.

## Custom Themes

Nevi comes with 15+ built-in themes. You can also create your own custom themes.

### Built-in Themes

Select a theme with `:theme <name>` or `<Space>ft` to open the theme picker.

Available themes: `onedark`, `dracula`, `gruvbox`, `nord`, `tokyonight`, `catppuccin-mocha`, `rose-pine`, `solarized-dark`, `kanagawa`, `monokai`, `everforest`, `github-dark`, `ayu-dark`, `palenight`, `nightfox`

### Creating Custom Themes

1. **Location:** Place `.toml` files in `~/.config/nevi/themes/`
2. **Template:** A commented template is auto-generated at `~/.config/nevi/themes/_template.toml`
3. **Naming:** The filename becomes the theme name (e.g., `mytheme.toml` → `:theme mytheme`)

Copy the template to get started:

```bash
cp ~/.config/nevi/themes/_template.toml ~/.config/nevi/themes/mytheme.toml
```

### Theme Structure

```toml
# Define reusable colors
[palette]
red = "#e06c75"
blue = "#61afef"
bg = "#282c34"

# Syntax highlighting (can reference palette or use hex)
[syntax]
keyword = { fg = "purple" }
string = { fg = "green" }
comment = { fg = "gray", italic = true }

# UI elements
[ui]
background = "bg"
foreground = "#abb2bf"
cursor_line = "#2c313c"

[ui.statusline]
mode_normal = "blue"
mode_insert = "green"

# And more: [ui.completion], [ui.finder], [diagnostic], [git]
```

See `~/.config/nevi/themes/_template.toml` for the complete reference with all available options.

## Keybindings

Nevi aims for full vim/neovim keybind compatibility. Most common keybindings are already implemented:

### Movement
`h/j/k/l`, `w/b/e/W/B/E`, `0/^/$`, `gg/G`, `{/}`, `f/F/t/T`, `;/,`, `%`, `H/M/L`

### Editing
`d/c/y` + motions, `dd/cc/yy`, `p/P`, `x/X`, `r`, `J`, `.`, `u/Ctrl+r`

### Text Objects
`iw/aw`, `iW/aW`, `i"/a"`, `i'/a'`, `i(/a(`, `i{/a{`, `i[/a[`, `i</a<`

### Search
`/`, `?`, `n/N`, `*/#`

### LSP
`gd` (definition), `gr` (references), `K` (hover), `gl` (diagnostic), `]d/[d` (next/prev diagnostic)

### Surround
`ds{char}` (delete), `cs{old}{new}` (change), `ys{motion}{char}` (add)

### Comment
`gcc` (line), `gc{motion}` (motion)

### Leader (`<Space>`)
| Key | Action |
|-----|--------|
| `ff` | Find files |
| `fg` | Live grep |
| `fb` | Find buffers |
| `ft` | Theme picker |
| `e` | File explorer |
| `ca` | Code actions |
| `rn` | Rename symbol |
| `d` | Search diagnostics |
| `D` | Line diagnostic |
| `w` | Save file |
| `q` | Quit |
| `gg` | Open lazygit |
| `m` | Add to harpoon |
| `h` | Harpoon menu |
| `1-4` | Jump to harpoon slot |

### Window Management
`Ctrl+w v` (vsplit), `Ctrl+w s` (hsplit), `Ctrl+w c` (close), `Ctrl+w h/j/k/l` (navigate)

### And More
Visual mode (`v/V/Ctrl+v`), macros (`q{a-z}/@{a-z}`), marks (`m{a-z}/'`), registers (`"{a-z}/"+`), replace mode (`R`)

> **Missing a keybind you use?** Open a [GitHub issue](https://github.com/anthonyamaro15/nevi/issues) and we'll prioritize adding it!

## Language Support

### LSP Servers

| Language | Server | Status |
|----------|--------|--------|
| Rust | rust-analyzer | Supported |
| TypeScript/JavaScript | typescript-language-server | Supported |
| Python | pyright | In Progress |
| CSS/SCSS | vscode-css-language-server | Supported |
| JSON | vscode-json-language-server | Supported |
| TOML | taplo | Supported |
| HTML | vscode-html-language-server | Supported |

LSP servers are auto-detected when installed. See [`~/.config/nevi/config.toml`](#configuration) for LSP configuration options.

> **Missing a language?** Open a [GitHub issue](https://github.com/anthonyamaro15/nevi/issues) and we'll work on adding support!

### External Formatters

Configure formatters in `~/.config/nevi/languages.toml`:

```toml
[typescript]
formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

- [Report a bug](https://github.com/anthonyamaro15/nevi/issues)
- [Request a feature](https://github.com/anthonyamaro15/nevi/issues)

## License

MIT License

## Acknowledgments

Inspired by [Neovim](https://neovim.io/), [Helix](https://helix-editor.com/), and [Zed](https://zed.dev/).

Built with:
- [ropey](https://github.com/cessen/ropey) - Rope data structure for text
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/) - Syntax highlighting
- [crossterm](https://github.com/crossterm-rs/crossterm) - Terminal handling
- [nucleo](https://github.com/helix-editor/nucleo) - Fuzzy matching
- [git2](https://github.com/rust-lang/git2-rs) - Git integration
