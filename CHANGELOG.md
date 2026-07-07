# Changelog

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
