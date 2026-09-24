# Configuring Nevi

Everything Nevi reads from disk, in one place. This file is written so that a
person, or a coding assistant you hand it to, can customize a Nevi setup
without guessing where anything lives.

- [Files](#files)
- [config.toml](#configtoml)
  - [\[editor\]](#editor)
  - [\[theme\], \[ui\], \[terminal\], \[finder\], \[explorer\]](#theme-ui-terminal-finder-explorer)
  - [\[keymap\]](#keymap)
  - [\[lsp\]](#lsp)
  - [\[copilot\]](#copilot)
- [languages.toml](#languagestoml)
- [Custom themes](#custom-themes)
- [Verify](#verify)

## Files

| Path | Purpose | Created |
|------|---------|---------|
| `~/.config/nevi/config.toml` | All settings and keymaps | First run, fully commented |
| `~/.config/nevi/languages.toml` | Per-language formatters and indentation | First run, with commented examples |
| `~/.config/nevi/themes/*.toml` | Custom themes; the filename is the theme name | `_template.toml` on first run |
| `~/.local/state/nevi/state.json` | Macros, registers, global marks, search history, jumplist | Automatically; safe to delete |
| `~/.local/state/nevi/recent_files.json` | Recent files for the start screen | Automatically; safe to delete |
| `.nevi/harpoon.json` | Harpoon pins, at the repository root | When you add the first pin |

The state directory follows nvim's `stdpath('state')` convention and respects
`$XDG_STATE_HOME`. Config paths are always `~/.config/nevi/`.

Nevi never rewrites a config file you own. When a release adds a new option, it
appears in `:ConfigDefaults`, not in your file, and the built-in default applies
until you set it. Every key below is optional.

## config.toml

### [editor]

| Key | Default | Notes |
|-----|---------|-------|
| `tab_width` | `4` | Spaces per tab. Override per language in `languages.toml`. |
| `line_numbers` | `true` | Show line numbers |
| `relative_numbers` | `false` | Relative line numbers |
| `sign_column` | `"yes"` | Git and diagnostic gutter. `"auto"` shows it only while a sign exists, like nvim. `"no"` hides it. `:set signcolumn=auto` switches it at runtime. |
| `cursor_line` | `false` | Highlight the cursor row |
| `scroll_off` | `8` | Lines kept visible above and below the cursor |
| `auto_indent` | `true` | Smart indentation on new lines |
| `wrap` | `false` | Soft wrap |
| `wrap_width` | `80` | Column to wrap at |
| `auto_pairs` | `true` | Auto-close brackets and quotes |
| `format_on_save` | `false` | Uses the `languages.toml` formatter, otherwise the LSP |
| `autosave` | `"off"` | `"off"`, `"after_delay"`, or `"on_focus_change"` |
| `autosave_delay_ms` | `1000` | Delay for `after_delay` |
| `mouse` | `true` | Wheel scrolls the buffer and clicks move the cursor, like nvim's `mouse=nvi`. `:set mouse=` or `:set nomouse` turns it off for the session. |
| `use_nerd_font_icons` | `true` | Setting this to `false` selects the minimal UI style automatically |

### [theme], [ui], [terminal], [finder], [explorer]

```toml
[theme]
colorscheme = "onedark"      # any bundled theme or a file in ~/.config/nevi/themes/

[ui]
style = "rich"               # "rich" needs a Nerd Font; "minimal" is plain ASCII

[terminal]
popup_width_ratio = 0.9      # floating terminal size as a fraction of the screen (0.2 to 1.0)
popup_height_ratio = 0.9

[terminal.shortcuts]
new_session = "<C-S-t>"      # "none" disables a shortcut
next_session = "<C-Tab>"
previous_session = "<C-S-Tab>"
close_session = "<C-S-w>"

[finder]
max_files = 10000            # cap on files scanned by the file picker
max_grep_results = 1000      # cap on live grep results
ignore_patterns = ["dist", "*.generated.ts"]   # on top of .gitignore

[explorer]
width = 35                   # sidebar width in columns
```

### [keymap]

```toml
[keymap]
leader = " "                 # Space by default
timeoutlen = 1000            # ms to wait for the rest of a leader sequence
show_leader_popup = true     # show available continuations after pressing the leader
```

**Remap a normal-mode or visual-mode key.** `from` is the key you press, `to`
is the key sequence Nevi runs instead, so anything that works when typed works
here, including ex commands.

```toml
[[keymap.normal]]
from = "H"
to = "^"

[[keymap.normal]]
from = "L"
to = "$"

[[keymap.normal]]
from = "s"
to = ":Jump<CR>"             # replaces Vim's s (substitute character)

[[keymap.visual]]
from = "s"
to = "S"                     # s surrounds the selection
```

**Add or change a leader shortcut.** `action` is an ex command. User mappings
replace the shipped default for the same key.

```toml
[[keymap.leader_mappings]]
key = "s"
action = ":wa"
desc = "Save all files"

[[keymap.leader_mappings]]
key = "tt"
action = ":Terminals"
desc = "Terminal picker"
```

**Remap explorer keys.** These apply only while the file explorer has focus.

```toml
[[keymap.explorer]]
key = "o"
action = "toggle_or_open"
desc = "Toggle directory or open file"

[[keymap.explorer]]
key = "<C-r>"
action = "refresh"
desc = "Refresh explorer"
```

Explorer actions: `close`, `move_down`, `move_up`, `move_to_top`,
`move_to_bottom`, `half_page_down`, `half_page_up`, `page_down`, `page_up`,
`toggle_or_open`, `expand_or_open`, `collapse_or_parent`, `toggle_expand`,
`collapse_all`, `refresh`, `show_explorer_keymaps`, `go_to_parent`,
`focus_editor`, `widen_sidebar`, `narrow_sidebar`, `reset_sidebar_width`,
`create`, `rename`, `delete`, `copy`, `cut`, `paste`, `search`, `next_match`,
`previous_match`.

**Remap command-line keys.** These apply while typing after `:`.

```toml
[[keymap.command_mappings]]
key = "<A-r>"
action = "history_toggle"
desc = "Open command history with Alt+r"

[[keymap.command_mappings]]
key = "<C-j>"
action = "popup_next"
desc = "Next completion"
```

**Key notation.** Regular keys are written as-is: `"a"`, `"H"`, `";"`, `"0"`.
Modifiers: `"<C-s>"` (Ctrl+s), `"<A-r>"` (Alt+r), `"<C-S-t>"` (Ctrl+Shift+t),
`"<C-Tab>"`. Special keys: `"<CR>"`, `"<Esc>"`, `"<Tab>"`, `"<Space>"`,
`"<BS>"`.

`:checkhealth` lists every override and warns about conflicts. `:Keymaps`
shows the result, including your remaps.

### [lsp]

Servers are auto-detected on PATH and enabled by default. See the
[Languages](README.md#languages) table for the server each language uses.

```toml
[lsp]
enabled = true               # false disables LSP entirely

# Per-server overrides. The section name is the language.
[lsp.servers.rust]
enabled = true
command = "rust-analyzer"
args = []

[lsp.servers.typescript]
preset = "biome"             # or "deno"; presets pick command, args, and root markers
root_patterns = ["biome.json", "package.json"]

[lsp.servers.markdown]
enabled = true               # marksman is off by default
```

Section names are the language ids listed under [languages.toml](#languagestoml).
Available presets: `typescript`, `biome`, `deno`, `eslint`, `rust_analyzer`,
`pyright`, `pylsp`.

### [copilot]

```toml
[copilot]
enabled = true
debounce_ms = 150            # delay before requesting a completion
auto_trigger = true          # request while typing in insert mode
hide_during_completion = true   # hide ghost text while the LSP popup is open
disabled_languages = ["markdown"]
```

Sign in with `:CopilotAuth`; `:CopilotStatus` and `:CopilotToggle` do what they
say.

## languages.toml

One section per language. `formatter` runs an external command with the buffer
on stdin and replaces the buffer with its stdout; `{file}` in `args` is the
full file path. `:Format` and format-on-save use it, and fall back to the LSP
when a language has no formatter.

```toml
[typescript]
formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
tab_width = 2

[python]
formatter = { command = "black", args = ["-", "--stdin-filename", "{file}"], timeout = 10 }
tab_width = 4

[go]
formatter = { command = "gofmt", args = [] }

[ruby]
formatter = { command = "rubocop", args = ["--stdin", "{file}", "-a", "--stderr", "--format", "quiet", "--fail-level", "fatal"] }
```

`command` and `args` are required; `timeout` is in seconds and defaults to 5.

Section names are the language ids: `rust`, `typescript`, `javascript`, `tsx`,
`jsx`, `python`, `go`, `ruby`, `php`, `css`, `json`, `toml`, `html`,
`markdown`, `shell`.

## Custom themes

```bash
cp ~/.config/nevi/themes/_template.toml ~/.config/nevi/themes/mytheme.toml
# then :theme mytheme, or colorscheme = "mytheme" under [theme]
```

The template is fully commented and lists every key. The shape:

```toml
# Reusable colors
[palette]
red = "#e06c75"
blue = "#61afef"
bg = "#282c34"

# Syntax highlighting; values can name a palette color or use hex
[syntax]
keyword = { fg = "purple" }
string = { fg = "green" }
comment = { fg = "gray", italic = true }

# Interface
[ui]
background = "bg"
foreground = "#abb2bf"
cursor_line = "#2c313c"

[ui.statusline]
mode_normal = "blue"
mode_insert = "green"
section_bg = "#21252b"      # optional; falls back to cursor_line

# Also: [ui.completion], [ui.finder], [diagnostic], [git]
```

`<Space>ft` previews themes live, so you can edit the file and switch back and
forth while tuning it.

## Verify

| Command | Shows |
|---------|-------|
| `:checkhealth` | Config paths, keymap overrides and conflicts, LSP and tool detection, formatter commands, UI style and a Nerd Font glyph probe |
| `:Keymaps` / `<Space>fk` | Every active binding with its description, including your remaps |
| `:ConfigDefaults` | The current built-in template, read-only |
| `:config` | Opens your `config.toml` |
| `:ToolInstall` | Install commands for any missing server or formatter |
