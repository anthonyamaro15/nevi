//! Authentication token persistence for GitHub Copilot
//!
//! Tokens are stored in the standard location used by copilot.vim/copilot.lua:
//! ~/.config/github-copilot/apps.json (on macOS/Linux)
//!
//! The token file format is compatible with other Copilot clients.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// OAuth token information
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    /// The OAuth token string
    pub oauth_token: String,
    /// When the token expires (Unix timestamp)
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// User associated with the token
    #[serde(default)]
    pub user: Option<String>,
}

// Custom Debug implementation that redacts the token to prevent accidental exposure in logs
impl std::fmt::Debug for OAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthToken")
            .field("oauth_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("user", &self.user)
            .finish()
    }
}

/// Apps.json file structure (compatible with copilot.vim)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppsJson {
    /// Map of app identifiers to tokens
    /// Key is typically "github.com" or similar
    #[serde(flatten)]
    pub apps: HashMap<String, OAuthToken>,
}

/// Get the path to the Copilot config directory
pub fn copilot_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config/github-copilot"))
}

/// Get the path to the apps.json token file
pub fn apps_json_path() -> Option<PathBuf> {
    copilot_config_dir().map(|d| d.join("apps.json"))
}

/// Load the OAuth token from apps.json
pub fn load_token() -> Result<Option<OAuthToken>> {
    let path = apps_json_path().ok_or_else(|| anyhow!("Cannot determine config directory"))?;

    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)?;
    let apps: AppsJson = serde_json::from_str(&content)?;

    // Look for github.com token (the standard key)
    if let Some(token) = apps.apps.get("github.com") {
        return Ok(Some(token.clone()));
    }

    // Fallback: try any token in the file
    if let Some((_, token)) = apps.apps.into_iter().next() {
        return Ok(Some(token));
    }

    Ok(None)
}

/// Save the OAuth token to apps.json
pub fn save_token(token: &OAuthToken) -> Result<()> {
    let dir = copilot_config_dir().ok_or_else(|| anyhow!("Cannot determine config directory"))?;

    // Create directory if it doesn't exist
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }

    let path = dir.join("apps.json");

    // Load existing apps or create new
    let mut apps = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        AppsJson::default()
    };

    // Update the github.com token
    apps.apps.insert("github.com".to_string(), token.clone());

    // Write back with secure permissions (0600 on Unix)
    let content = serde_json::to_string_pretty(&apps)?;

    // Use atomic file creation with secure permissions on Unix to avoid TOCTOU race
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&path, &content)?;
    }

    Ok(())
}

/// Clear the stored token (sign out)
pub fn clear_token() -> Result<()> {
    let path = apps_json_path().ok_or_else(|| anyhow!("Cannot determine config directory"))?;

    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)?;
    let mut apps: AppsJson = serde_json::from_str(&content).unwrap_or_default();

    // Remove github.com token
    apps.apps.remove("github.com");

    // Write back
    let content = serde_json::to_string_pretty(&apps)?;
    std::fs::write(&path, content)?;

    Ok(())
}

/// Check if we have a stored token
pub fn has_token() -> bool {
    load_token().ok().flatten().is_some()
}

/// Get the GitHub hosts.json path (alternative token location)
pub fn hosts_json_path() -> Option<PathBuf> {
    copilot_config_dir().map(|d| d.join("hosts.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apps_json_parsing() {
        let json = r#"{
            "github.com": {
                "oauth_token": "gho_xxxxxxxxxxxx",
                "expires_at": 1234567890,
                "user": "testuser"
            }
        }"#;

        let apps: AppsJson = serde_json::from_str(json).unwrap();
        assert!(apps.apps.contains_key("github.com"));
        let token = apps.apps.get("github.com").unwrap();
        assert_eq!(token.user, Some("testuser".to_string()));
    }

    #[test]
    fn test_oauth_token_debug_redacts_secret() {
        let token = OAuthToken {
            oauth_token: "gho_secret_token_12345".to_string(),
            expires_at: Some(1234567890),
            user: Some("testuser".to_string()),
        };

        let debug_output = format!("{:?}", token);

        // The actual token should NOT appear in debug output
        assert!(
            !debug_output.contains("gho_secret_token_12345"),
            "Debug output should not contain the actual token"
        );
        // But [REDACTED] should appear
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output should show [REDACTED]"
        );
        // Other fields should still be visible
        assert!(
            debug_output.contains("testuser"),
            "Debug output should still show user"
        );
    }
}
