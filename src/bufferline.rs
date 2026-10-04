use crate::editor::Buffer;

pub struct BufferlineEntry {
    pub label: String,
    pub dirty: bool,
    pub active: bool,
}

pub fn build(buffers: &[Buffer], active_buffer_idx: usize) -> Vec<BufferlineEntry> {
    buffers
        .iter()
        .enumerate()
        .map(|(idx, item)| BufferlineEntry {
            label: item.display_name(),
            dirty: item.dirty,
            active: idx == active_buffer_idx,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{Buffer, bufferline::build};

    fn build_buffers(items: usize) -> Vec<Buffer> {
        let mut buffers: Vec<Buffer> = Vec::new();
        for _ in 0..items {
            buffers.push(Buffer::new());
        }
        buffers
    }

    #[test]
    fn index_out_of_bound() {
        let buffers = build_buffers(1);

        let bufferline_entries = build(&buffers, 2);

        assert!(!bufferline_entries[0].active);
        assert_eq!(bufferline_entries[0].label, "[No Name]");
        assert!(!bufferline_entries[0].dirty);
    }

    #[test]
    fn single_active_buffer() {
        let buffers = build_buffers(2);

        let bufferline_entries = build(&buffers, 1);

        assert!(!bufferline_entries[0].active);
        assert!(bufferline_entries[1].active);
    }

    #[test]
    fn no_name_filename() {
        let buffers = build_buffers(1);

        let bufferline_entries = build(&buffers, 0);

        assert!(bufferline_entries[0].active);
        assert_eq!(bufferline_entries[0].label, "[No Name]");
    }
}
