use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use globset::{GlobBuilder, GlobMatcher};
use lsp_types::{
    DidChangeWatchedFilesRegistrationOptions, FileChangeType, GlobPattern, OneOf,
    RegistrationParams, RelativePattern, UnregistrationParams, WatchKind,
};

pub(crate) const WATCHED_FILES_METHOD: &str = "workspace/didChangeWatchedFiles";

#[derive(Debug)]
pub(crate) enum WatcherRequestError {
    InvalidParams(String),
    #[allow(dead_code)] // Used by later registration routing when watcher setup can fail.
    Setup(String),
}

impl std::fmt::Display for WatcherRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(message) | Self::Setup(message) => formatter.write_str(message),
        }
    }
}

#[derive(Debug)]
pub(crate) struct WatchedFileRegistration {
    #[allow(dead_code)] // Read by later unregister/register lifecycle routing.
    pub(crate) id: String,
    #[allow(dead_code)] // Compiled into watchers when registration lifecycle lands.
    pub(crate) options: DidChangeWatchedFilesRegistrationOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchInput {
    Relative,
    Absolute,
}

#[derive(Clone)]
struct CompiledWatcher {
    base: PathBuf,
    #[allow(dead_code)] // Read by registration lifecycle when syncing watch roots.
    root: PathBuf,
    matcher: GlobMatcher,
    input: MatchInput,
    kind: WatchKind,
}

fn normalized_match_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
            Component::RootDir => Some(String::new()),
            Component::CurDir => None,
            Component::ParentDir => Some("..".to_string()),
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn watch_kind_allows(kind: WatchKind, change: FileChangeType) -> bool {
    match change {
        FileChangeType::CREATED => kind.contains(WatchKind::Create),
        FileChangeType::CHANGED => kind.contains(WatchKind::Change),
        FileChangeType::DELETED => kind.contains(WatchKind::Delete),
        _ => false,
    }
}

fn contains_glob_meta(segment: &str) -> bool {
    segment
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{'))
}

fn absolute_pattern_root(pattern: &str) -> PathBuf {
    let path = Path::new(pattern);
    let mut prefix = PathBuf::new();

    for component in path.components() {
        let text = component.as_os_str().to_string_lossy();
        if contains_glob_meta(&text) {
            break;
        }
        prefix.push(component.as_os_str());
    }

    if prefix == path {
        prefix.parent().unwrap_or(Path::new("/")).to_path_buf()
    } else if prefix.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        prefix
    }
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .with_context(|| format!("invalid LSP glob pattern: {pattern}"))?
        .compile_matcher())
}

fn relative_pattern_base(pattern: &RelativePattern) -> Result<PathBuf> {
    let uri = match &pattern.base_uri {
        OneOf::Left(folder) => &folder.uri,
        OneOf::Right(uri) => uri,
    };
    uri.to_file_path()
        .map_err(|_| anyhow!("watched-file base URI is not a file URI: {uri}"))
}

pub(crate) fn parse_register_params(
    params: Option<serde_json::Value>,
) -> std::result::Result<Vec<WatchedFileRegistration>, WatcherRequestError> {
    let params: RegistrationParams = serde_json::from_value(params.ok_or_else(|| {
        WatcherRequestError::InvalidParams("missing registration params".to_string())
    })?)
    .map_err(|error| WatcherRequestError::InvalidParams(error.to_string()))?;

    params
        .registrations
        .into_iter()
        .filter(|registration| registration.method == WATCHED_FILES_METHOD)
        .map(|registration| {
            let options = serde_json::from_value(registration.register_options.ok_or_else(
                || {
                    WatcherRequestError::InvalidParams(format!(
                        "watched-file registration {} has no options",
                        registration.id
                    ))
                },
            )?)
            .map_err(|error| WatcherRequestError::InvalidParams(error.to_string()))?;
            Ok(WatchedFileRegistration {
                id: registration.id,
                options,
            })
        })
        .collect()
}

pub(crate) fn parse_unregister_params(
    params: Option<serde_json::Value>,
) -> std::result::Result<Vec<String>, WatcherRequestError> {
    let params: UnregistrationParams = serde_json::from_value(params.ok_or_else(|| {
        WatcherRequestError::InvalidParams("missing unregistration params".to_string())
    })?)
    .map_err(|error| WatcherRequestError::InvalidParams(error.to_string()))?;

    Ok(params
        .unregisterations
        .into_iter()
        .filter(|registration| registration.method == WATCHED_FILES_METHOD)
        .map(|registration| registration.id)
        .collect())
}

fn compile_watcher(
    watcher: &lsp_types::FileSystemWatcher,
    workspace_root: &Path,
) -> Result<CompiledWatcher> {
    let kind = watcher
        .kind
        .unwrap_or(WatchKind::Create | WatchKind::Change | WatchKind::Delete);

    match &watcher.glob_pattern {
        GlobPattern::Relative(pattern) => {
            let base = relative_pattern_base(pattern)?;
            Ok(CompiledWatcher {
                root: base.clone(),
                base,
                matcher: compile_glob(&pattern.pattern)?,
                input: MatchInput::Relative,
                kind,
            })
        }
        GlobPattern::String(pattern) if Path::new(pattern).is_absolute() => Ok(CompiledWatcher {
            base: PathBuf::from("/"),
            root: absolute_pattern_root(pattern),
            matcher: compile_glob(pattern)?,
            input: MatchInput::Absolute,
            kind,
        }),
        GlobPattern::String(pattern) => Ok(CompiledWatcher {
            base: workspace_root.to_path_buf(),
            root: workspace_root.to_path_buf(),
            matcher: compile_glob(pattern)?,
            input: MatchInput::Relative,
            kind,
        }),
    }
}

impl CompiledWatcher {
    fn matches(&self, path: &Path, change: FileChangeType) -> bool {
        if !watch_kind_allows(self.kind, change) {
            return false;
        }

        let candidate = match self.input {
            MatchInput::Relative => match path.strip_prefix(&self.base) {
                Ok(relative) => normalized_match_path(relative),
                Err(_) => return false,
            },
            MatchInput::Absolute => normalized_match_path(path),
        };

        self.matcher.is_match(candidate)
    }
}

// Task 2 intentionally lands parser/matcher foundations before Tasks 4 and 5
// wire them into the runtime watcher and dynamic-registration routing.
const _: fn(
    Option<serde_json::Value>,
) -> std::result::Result<Vec<WatchedFileRegistration>, WatcherRequestError> =
    parse_register_params;
const _: fn(Option<serde_json::Value>) -> std::result::Result<Vec<String>, WatcherRequestError> =
    parse_unregister_params;
const _: fn(&lsp_types::FileSystemWatcher, &Path) -> Result<CompiledWatcher> = compile_watcher;
const _: fn(&CompiledWatcher, &Path, FileChangeType) -> bool = CompiledWatcher::matches;

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{
        FileSystemWatcher, Registration, RegistrationParams, Unregistration, UnregistrationParams,
        Url, WorkspaceFolder,
    };
    use serde_json::json;

    fn workspace_folder(path: &Path) -> WorkspaceFolder {
        WorkspaceFolder {
            uri: Url::from_file_path(path).expect("workspace URI"),
            name: "workspace".to_string(),
        }
    }

    fn register_params(registrations: Vec<Registration>) -> Option<serde_json::Value> {
        Some(serde_json::to_value(RegistrationParams { registrations }).unwrap())
    }

    fn unregister_params(unregisterations: Vec<Unregistration>) -> Option<serde_json::Value> {
        Some(serde_json::to_value(UnregistrationParams { unregisterations }).unwrap())
    }

    fn watched_file_registration(id: &str, register_options: serde_json::Value) -> Registration {
        Registration {
            id: id.to_string(),
            method: WATCHED_FILES_METHOD.to_string(),
            register_options: Some(register_options),
        }
    }

    #[test]
    fn parse_register_params_returns_watched_file_registration_options() {
        let params = register_params(vec![watched_file_registration(
            "rust-files",
            json!({
                "watchers": [
                    {
                        "globPattern": "**/*.rs",
                        "kind": 3
                    }
                ]
            }),
        )]);

        let registrations = parse_register_params(params).expect("registration params");

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].id, "rust-files");
        assert_eq!(registrations[0].options.watchers.len(), 1);
        assert_eq!(
            registrations[0].options.watchers[0].glob_pattern,
            GlobPattern::String("**/*.rs".to_string())
        );
        assert_eq!(
            registrations[0].options.watchers[0].kind,
            Some(WatchKind::Create | WatchKind::Change)
        );
    }

    #[test]
    fn parse_register_params_filters_unrelated_registration_methods() {
        let params = register_params(vec![Registration {
            id: "configuration".to_string(),
            method: "workspace/didChangeConfiguration".to_string(),
            register_options: None,
        }]);

        let registrations = parse_register_params(params).expect("registration params");

        assert!(registrations.is_empty());
    }

    #[test]
    fn parse_register_params_rejects_missing_params_or_malformed_watched_file_options() {
        assert!(matches!(
            parse_register_params(None),
            Err(WatcherRequestError::InvalidParams(message))
                if message == "missing registration params"
        ));

        let malformed = register_params(vec![watched_file_registration(
            "rust-files",
            json!({ "watchers": "not an array" }),
        )]);

        assert!(matches!(
            parse_register_params(malformed),
            Err(WatcherRequestError::InvalidParams(_))
        ));
    }

    #[test]
    fn parse_unregister_params_filters_to_watched_file_registration_ids() {
        let params = unregister_params(vec![
            Unregistration {
                id: "rust-files".to_string(),
                method: WATCHED_FILES_METHOD.to_string(),
            },
            Unregistration {
                id: "configuration".to_string(),
                method: "workspace/didChangeConfiguration".to_string(),
            },
        ]);

        let registration_ids = parse_unregister_params(params).expect("unregistration params");

        assert_eq!(registration_ids, vec!["rust-files".to_string()]);
    }

    #[test]
    fn relative_pattern_matches_only_under_its_base_uri() {
        let root = PathBuf::from("/tmp/nevi-watch-root");
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::Relative(RelativePattern {
                base_uri: OneOf::Left(workspace_folder(&root)),
                pattern: "**/*.rs".to_string(),
            }),
            kind: None,
        };

        let compiled = compile_watcher(&watcher, &root).expect("compiled watcher");

        assert!(compiled.matches(&root.join("src/main.rs"), FileChangeType::CHANGED));
        assert!(!compiled.matches(&root.join("README.md"), FileChangeType::CHANGED));
        assert!(!compiled.matches(
            Path::new("/tmp/other/src/main.rs"),
            FileChangeType::CHANGED
        ));
    }

    #[test]
    fn relative_pattern_accepts_base_uri_as_url() {
        let workspace_root = PathBuf::from("/tmp/nevi-watch-workspace");
        let base = PathBuf::from("/tmp/nevi-watch-base");
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::Relative(RelativePattern {
                base_uri: OneOf::Right(Url::from_file_path(&base).expect("base URI")),
                pattern: "src/**/*.rs".to_string(),
            }),
            kind: None,
        };

        let compiled = compile_watcher(&watcher, &workspace_root).expect("compiled watcher");

        assert_eq!(compiled.root, base);
        assert!(compiled.matches(
            Path::new("/tmp/nevi-watch-base/src/nested/lib.rs"),
            FileChangeType::CHANGED
        ));
        assert!(!compiled.matches(
            Path::new("/tmp/nevi-watch-workspace/src/nested/lib.rs"),
            FileChangeType::CHANGED
        ));
    }

    #[test]
    fn string_pattern_uses_workspace_root_when_relative() {
        let root = PathBuf::from("/tmp/nevi-watch-root");
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::String("**/Cargo.{toml,lock}".to_string()),
            kind: None,
        };

        let compiled = compile_watcher(&watcher, &root).expect("compiled watcher");

        assert!(compiled.matches(&root.join("crates/app/Cargo.toml"), FileChangeType::CREATED));
        assert!(compiled.matches(&root.join("Cargo.lock"), FileChangeType::CHANGED));
        assert!(!compiled.matches(&root.join("src/main.rs"), FileChangeType::CHANGED));
    }

    #[test]
    fn absolute_string_pattern_watches_parent_and_matches_exact_file() {
        let root = std::env::temp_dir().join("nevi-watch-root");
        let manifest = root.join("Cargo.toml");
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::String(normalized_match_path(&manifest)),
            kind: None,
        };

        let compiled = compile_watcher(&watcher, &root).expect("compiled watcher");

        assert_eq!(compiled.root, root);
        assert!(compiled.matches(&manifest, FileChangeType::CHANGED));
        assert!(!compiled.matches(&root.join("Cargo.lock"), FileChangeType::CHANGED));
    }

    #[test]
    fn absolute_string_glob_pattern_watches_static_prefix_and_matches_descendants() {
        let root = std::env::temp_dir().join("nevi-watch-root");
        let src = root.join("src");
        let pattern = format!("{}/**/*.rs", normalized_match_path(&src));
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::String(pattern),
            kind: None,
        };

        let compiled = compile_watcher(&watcher, &root).expect("compiled watcher");

        assert_eq!(compiled.root, src);
        assert!(compiled.matches(&root.join("src/nested/lib.rs"), FileChangeType::CHANGED));
        assert!(!compiled.matches(&root.join("tests/nested/lib.rs"), FileChangeType::CHANGED));
        assert!(!compiled.matches(&root.join("src/nested/lib.toml"), FileChangeType::CHANGED));
    }

    #[test]
    fn watcher_kind_filters_unrequested_events() {
        let root = PathBuf::from("/tmp/nevi-watch-root");
        let watcher = FileSystemWatcher {
            glob_pattern: GlobPattern::String("**/*.rs".to_string()),
            kind: Some(WatchKind::Create | WatchKind::Delete),
        };

        let compiled = compile_watcher(&watcher, &root).expect("compiled watcher");
        let path = root.join("src/lib.rs");

        assert!(compiled.matches(&path, FileChangeType::CREATED));
        assert!(!compiled.matches(&path, FileChangeType::CHANGED));
        assert!(compiled.matches(&path, FileChangeType::DELETED));
    }
}
