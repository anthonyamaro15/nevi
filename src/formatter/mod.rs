//! External formatter support
//!
//! Runs external formatters (biome, prettier, black, etc.) via stdin/stdout

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config::FormatterConfig;

/// Default timeout for formatters (5 seconds)
const DEFAULT_TIMEOUT_SECS: u64 = 5;

/// Errors that can occur during formatting
#[derive(Debug)]
pub enum FormatterError {
    /// Failed to spawn the formatter process
    SpawnFailed(std::io::Error),
    /// Formatter command not found
    CommandNotFound(String),
    /// Formatter process failed with non-zero exit code
    CommandFailed { stderr: String, exit_code: Option<i32> },
    /// Formatter output was not valid UTF-8
    InvalidUtf8,
    /// Formatter timed out
    Timeout,
    /// Failed to write to stdin
    StdinWriteFailed(std::io::Error),
}

impl std::fmt::Display for FormatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatterError::SpawnFailed(e) => write!(f, "Failed to start formatter: {}", e),
            FormatterError::CommandNotFound(cmd) => write!(f, "Formatter not found: {}", cmd),
            FormatterError::CommandFailed { stderr, exit_code } => {
                if let Some(code) = exit_code {
                    write!(f, "Formatter failed (exit {}): {}", code, stderr.trim())
                } else {
                    write!(f, "Formatter failed: {}", stderr.trim())
                }
            }
            FormatterError::InvalidUtf8 => write!(f, "Formatter output was not valid UTF-8"),
            FormatterError::Timeout => write!(f, "Formatter timed out"),
            FormatterError::StdinWriteFailed(e) => write!(f, "Failed to send content to formatter: {}", e),
        }
    }
}

/// External formatter that runs a command with stdin/stdout
pub struct ExternalFormatter {
    command: String,
    args: Vec<String>,
    timeout: Duration,
}

impl ExternalFormatter {
    /// Create a new external formatter from config
    pub fn from_config(config: &FormatterConfig) -> Self {
        Self {
            command: config.command.clone(),
            args: config.args.clone(),
            timeout: Duration::from_secs(config.timeout.max(1)),
        }
    }

    /// Create a new external formatter with default timeout
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self {
            command,
            args,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Format content using the external command
    ///
    /// The content is piped to the command's stdin, and the formatted
    /// output is read from stdout.
    ///
    /// The `{file}` placeholder in args is replaced with the file path.
    pub fn format(&self, content: &str, file_path: &str) -> Result<String, FormatterError> {
        // Replace {file} placeholder in args
        let args: Vec<String> = self
            .args
            .iter()
            .map(|arg| arg.replace("{file}", file_path))
            .collect();

        // Spawn the formatter process
        let mut child = Command::new(&self.command)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    FormatterError::CommandNotFound(self.command.clone())
                } else {
                    FormatterError::SpawnFailed(e)
                }
            })?;

        // Write content to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(content.as_bytes())
                .map_err(FormatterError::StdinWriteFailed)?;
            // stdin is dropped here, closing it so the formatter knows input is complete
        }

        // Wait for output (blocking)
        // TODO: Add timeout support in v2 if needed
        let output = child
            .wait_with_output()
            .map_err(FormatterError::SpawnFailed)?;

        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|_| FormatterError::InvalidUtf8)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(FormatterError::CommandFailed {
                stderr,
                exit_code: output.status.code(),
            })
        }
    }
}

/// Format a buffer using an external formatter if configured
///
/// Returns Some(formatted_content) if formatting succeeded,
/// None if no formatter is configured for this language.
pub fn format_with_external(
    content: &str,
    file_path: &str,
    formatter_config: &FormatterConfig,
) -> Result<String, FormatterError> {
    let formatter = ExternalFormatter::from_config(formatter_config);
    formatter.format(content, file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_replacement() {
        let formatter = ExternalFormatter::new(
            "echo".to_string(),
            vec!["--file".to_string(), "{file}".to_string()],
        );

        // Test that {file} is replaced
        let args: Vec<String> = formatter
            .args
            .iter()
            .map(|arg| arg.replace("{file}", "/path/to/file.ts"))
            .collect();

        assert_eq!(args, vec!["--file", "/path/to/file.ts"]);
    }
}
