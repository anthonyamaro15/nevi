# Windows support

Tracking issue: [#123](https://github.com/anthonyamaro15/nevi/issues/123).

CI builds Windows and runs only the `gx` URL opener tests there (the
`windows-url-open` job in `.github/workflows/rust.yml`). The full suite still
runs on macOS and Linux only, so most regressions here won't be caught
automatically.

## Windows-specific gaps found so far

- **Every keystroke ran twice.** crossterm's Windows console backend emits
  both a `Press` and a `Release` `Event::Key` per keystroke, unlike Unix.
  `read_event`/`read_key` (`src/terminal/mod.rs`) now drop `KeyEventKind::Release`.
- **Startup crashed pushing keyboard enhancement.** crossterm has no real Windows
  implementation of it: `PushKeyboardEnhancementFlags` always errors there with
  `Unsupported`. `enter_editor_screen` (`src/terminal/session.rs`) now stores
  whether the push succeeded instead of assuming it did, and `leave_editor_screen`
  pops only when it did. This also sidesteps `supports_keyboard_enhancement()`,
  which on Unix opens `/dev/tty` and blocks without a real terminal to answer.
- **Leaving the editor screen could panic.** Windows' `EnableMouseCapture`/
  `DisableMouseCapture` bypass the `Write` sink entirely and call
  `SetConsoleMode` on the real console handle. Disabling mouse capture before
  it was ever enabled in the process panics with `"Initial console modes not
  set"`. `leave_editor_screen` now only disables it when `mouse_capture_enabled`
  is actually true.

## Still open

- The Windows CI job runs only the URL opener tests, so the above was only
  verified by running the suite locally on Windows.
- Job control (`Ctrl-Z` suspend, `:stop`) is a no-op on Windows (no SIGTSTP);
  `:!` and lazygit-style external commands still work since they don't depend
  on it.
- Anything outside manual testing of normal/insert mode and the terminal
  hand-off above is unverified on Windows.
