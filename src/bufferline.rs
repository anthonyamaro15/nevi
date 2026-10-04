use crate::editor::Buffer;
use std::collections::HashMap;
use std::path::Path;

pub struct BufferlineEntry {
    pub label: String,
    pub dirty: bool,
    pub active: bool,
}

pub fn build(buffers: &[Buffer], active_buffer_idx: usize) -> Vec<BufferlineEntry> {
    let names: Vec<String> = buffers.iter().map(Buffer::display_name).collect();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, name) in names.iter().enumerate() {
        by_name.entry(name).or_default().push(idx);
    }

    let mut labels = names.clone();
    for indices in by_name.values().filter(|indices| indices.len() > 1) {
        let paths: Vec<Option<Vec<String>>> = indices
            .iter()
            .map(|&idx| buffers[idx].path.as_deref().map(parent_components))
            .collect();
        let max_depth = paths.iter().flatten().map(Vec::len).max().unwrap_or(0);

        for depth in 1..=max_depth {
            let candidates: Vec<String> = indices
                .iter()
                .zip(&paths)
                .map(|(&idx, parts)| match parts {
                    Some(parts) if !parts.is_empty() => {
                        let start = parts.len().saturating_sub(depth);
                        format!("{}/{}", parts[start..].join("/"), names[idx])
                    }
                    _ => names[idx].clone(),
                })
                .collect();
            if candidates.iter().enumerate().all(|(i, label)| {
                candidates
                    .iter()
                    .enumerate()
                    .all(|(j, other)| i == j || label != other)
            }) {
                for (&idx, label) in indices.iter().zip(candidates) {
                    labels[idx] = label;
                }
                break;
            }
        }
    }

    buffers
        .iter()
        .enumerate()
        .map(|(idx, item)| BufferlineEntry {
            label: labels[idx].clone(),
            dirty: item.dirty,
            active: idx == active_buffer_idx,
        })
        .collect()
}

fn parent_components(path: &Path) -> Vec<String> {
    path.parent()
        .into_iter()
        .flat_map(|parent| parent.components())
        .filter_map(|component| {
            let value = component.as_os_str().to_string_lossy();
            if value.is_empty() || value == "." || value == "/" {
                None
            } else {
                Some(value.into_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{Buffer, bufferline::build};
    use std::path::PathBuf;

    fn buffers_with_paths(paths: &[&str]) -> Vec<Buffer> {
        paths
            .iter()
            .map(|path| {
                let mut buffer = Buffer::new();
                buffer.path = Some(PathBuf::from(path));
                buffer
            })
            .collect()
    }

    fn build_buffers(items: usize) -> Vec<Buffer> {
        (0..items).map(|_| Buffer::new()).collect()
    }

    #[test]
    fn index_out_of_bound() {
        let buffers = build_buffers(1);
        let entries = build(&buffers, 2);
        assert!(!entries[0].active);
        assert_eq!(entries[0].label, "[No Name]");
        assert!(!entries[0].dirty);
    }

    #[test]
    fn single_active_buffer() {
        let buffers = build_buffers(2);
        let entries = build(&buffers, 1);
        assert!(!entries[0].active);
        assert!(entries[1].active);
    }

    #[test]
    fn no_name_filename() {
        let buffers = build_buffers(1);
        let entries = build(&buffers, 0);
        assert!(entries[0].active);
        assert_eq!(entries[0].label, "[No Name]");
    }

    #[test]
    fn duplicate_names_use_nearest_distinguishing_parent() {
        let buffers =
            buffers_with_paths(&["src/editor/mod.rs", "src/terminal/mod.rs", "src/lib.rs"]);
        let entries = build(&buffers, 0);
        assert_eq!(entries[0].label, "editor/mod.rs");
        assert_eq!(entries[1].label, "terminal/mod.rs");
        assert_eq!(entries[2].label, "lib.rs");
    }

    #[test]
    fn duplicate_names_expand_parent_until_unique() {
        let buffers = buffers_with_paths(&["a/src/mod.rs", "b/src/mod.rs"]);
        let entries = build(&buffers, 0);
        assert_eq!(entries[0].label, "a/src/mod.rs");
        assert_eq!(entries[1].label, "b/src/mod.rs");
    }
}
