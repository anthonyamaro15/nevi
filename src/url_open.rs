//! Open a URL in the platform's default browser without going through a shell.
//!
//! Only `http://` and `https://` URLs are opened (the scheme is matched
//! case-insensitively, with nothing before it). Every opener used here hands
//! its argument to the OS "open this" service, which would also start a local
//! program or file such as `calc.exe` or `file:///...`, so anything else is
//! refused before a command is built.
//!
//! Uses only `std` so the Windows branch can be type-checked from another
//! platform by including this file in a scratch crate.

use std::process::Command;

/// Whether `url` is a web URL this module will open.
pub fn is_supported_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Build the command that opens `url`. The URL is never read by a shell, so
/// characters like `&`, `|`, `^` or `%` in it stay part of the URL.
pub fn command_for(url: &str) -> Result<Command, String> {
    if !is_supported_url(url) {
        return Err("Only http:// and https:// URLs can be opened".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        Ok(command)
    }

    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        Ok(command)
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        // `cmd /C start "" <url>` used to be used here, but cmd parses its own
        // command line, so `https://x/&calc` ran `calc`. rundll32 hands
        // everything after the entry point name to FileProtocolHandler as one
        // raw string, which then goes to ShellExecute. `raw_arg` keeps the URL
        // byte for byte: `arg` would quote and backslash-escape a `"` for the
        // MSVC argv rules, and rundll32 does not undo that, so the browser
        // would get a different URL.
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command.raw_arg(url);
        Ok(command)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        Err("Opening URLs is not supported on this platform".to_string())
    }
}

/// Open `url` in the default browser. Does not wait for the opener to exit.
pub fn open(url: &str) -> Result<(), String> {
    command_for(url)?
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Failed to open URL: {}", err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::Path;

    const SHELLS: &[&str] = &[
        "cmd",
        "powershell",
        "pwsh",
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
    ];

    fn program_stem(command: &Command) -> String {
        Path::new(command.get_program())
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn url_open_command_is_not_a_shell() {
        let command = command_for("https://example.invalid/").unwrap();
        let stem = program_stem(&command);
        assert!(
            !SHELLS.contains(&stem.as_str()),
            "opener is a shell: {stem}"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn url_open_shell_metacharacters_stay_one_argument() {
        let url = r#"https://example.invalid/a&b|c^d%PATH%e"f\"#;
        let command = command_for(url).unwrap();
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args.last(), Some(&OsStr::new(url)));

        #[cfg(target_os = "windows")]
        assert_eq!(
            args,
            [OsStr::new("url.dll,FileProtocolHandler"), OsStr::new(url)]
        );
        #[cfg(not(target_os = "windows"))]
        assert_eq!(args, [OsStr::new(url)]);
    }

    #[test]
    fn url_open_refuses_non_web_urls() {
        for url in [
            "calc.exe",
            r"C:\Windows\System32\calc.exe",
            "file:///etc/passwd",
            "ftp://example.invalid/",
            "",
            " https://example.invalid/",
        ] {
            assert!(command_for(url).is_err(), "accepted {url:?}");
        }

        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        assert!(command_for("HTTPS://example.invalid/").is_ok());
        assert!(is_supported_url("HTTPS://example.invalid/"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn url_open_macos_uses_open() {
        assert_eq!(
            command_for("https://example.invalid/")
                .unwrap()
                .get_program(),
            "open"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn url_open_linux_uses_xdg_open() {
        assert_eq!(
            command_for("https://example.invalid/")
                .unwrap()
                .get_program(),
            "xdg-open"
        );
    }

    /// Runs the injection for real on Windows: once through the pre-fix
    /// command shape to prove the payload works, once through `command_for`
    /// to prove it no longer does. Each run may open a browser tab.
    #[cfg(windows)]
    #[test]
    fn url_open_windows_payload_does_not_run_commands() {
        use std::path::PathBuf;
        use std::process::{Child, Stdio};
        use std::time::{Duration, Instant};

        const TIMEOUT: Duration = Duration::from_secs(20);

        // The pre-fix shape, kept only to show the payload is live.
        fn pre_fix_command(url: &str) -> Command {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", "", url]);
            command
        }

        fn spawn_quiet(mut command: Command) -> Child {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn opener")
        }

        // Waits on the spawned opener only, never on the browser it starts.
        fn wait_or_kill(mut child: Child, timeout: Duration) {
            let start = Instant::now();
            while start.elapsed() < timeout {
                if child.try_wait().expect("try_wait").is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }

        fn payload(marker: &Path) -> String {
            // `echo.>path` writes a file without needing a space, since the
            // URL token under the cursor can never contain whitespace.
            format!("https://example.invalid/&echo.>{}", marker.display())
        }

        let dir: PathBuf = std::env::temp_dir().join(format!(
            "nevi-url-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.display().to_string();
        assert!(
            !dir_str.chars().any(char::is_whitespace),
            "temp dir {dir_str:?} has whitespace, so the payload cannot be one URL token; set TEMP to a path without spaces"
        );

        let control_marker = dir.join("control");
        let control_start = Instant::now();
        let control = spawn_quiet(pre_fix_command(&payload(&control_marker)));
        while !control_marker.exists() && control_start.elapsed() < TIMEOUT {
            std::thread::sleep(Duration::from_millis(50));
        }
        let control_elapsed = control_start.elapsed();
        wait_or_kill(control, TIMEOUT);
        let control_injected = control_marker.exists();

        let fix_marker = dir.join("fixed");
        let fix_start = Instant::now();
        let fixed = spawn_quiet(command_for(&payload(&fix_marker)).unwrap());
        wait_or_kill(fixed, TIMEOUT);
        let floor = control_elapsed * 2 + Duration::from_secs(2);
        if let Some(rest) = floor.checked_sub(fix_start.elapsed()) {
            std::thread::sleep(rest);
        }
        let injected = fix_marker.exists();

        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            control_injected,
            "control: the pre-fix cmd shape did not run the injected command, so this test cannot detect injection"
        );
        assert!(
            !injected,
            "the opener ran a command injected through the URL"
        );
    }
}
