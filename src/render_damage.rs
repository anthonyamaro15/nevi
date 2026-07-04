use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDamage {
    full: bool,
    statusline: bool,
    command_line: bool,
    editor_rows: BTreeSet<usize>,
}

impl RenderDamage {
    pub fn full() -> Self {
        Self {
            full: true,
            statusline: false,
            command_line: false,
            editor_rows: BTreeSet::new(),
        }
    }

    pub fn clean() -> Self {
        Self {
            full: false,
            statusline: false,
            command_line: false,
            editor_rows: BTreeSet::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        !self.full && !self.statusline && !self.command_line && self.editor_rows.is_empty()
    }

    pub fn requires_full_render(&self) -> bool {
        self.full
    }

    pub fn statusline(&self) -> bool {
        self.statusline
    }

    pub fn command_line(&self) -> bool {
        self.command_line
    }

    pub fn dirty_editor_rows(&self) -> Vec<usize> {
        self.editor_rows.iter().copied().collect()
    }

    pub fn mark_full(&mut self) {
        self.full = true;
        self.statusline = false;
        self.command_line = false;
        self.editor_rows.clear();
    }

    pub fn mark_statusline(&mut self) {
        if !self.full {
            self.statusline = true;
        }
    }

    pub fn mark_command_line(&mut self) {
        if !self.full {
            self.command_line = true;
        }
    }

    pub fn mark_editor_row(&mut self, row: usize) {
        if !self.full {
            self.editor_rows.insert(row);
        }
    }

    pub fn mark_editor_rows(&mut self, rows: Range<usize>) {
        if self.full {
            return;
        }
        self.editor_rows.extend(rows);
    }

    pub fn clear_after_full_render(&mut self) {
        *self = Self::clean();
    }
}

impl Default for RenderDamage {
    fn default() -> Self {
        Self::full()
    }
}

#[cfg(test)]
mod tests {
    use super::RenderDamage;

    #[test]
    fn damage_records_granular_regions_until_full_damage_takes_over() {
        let mut damage = RenderDamage::clean();

        assert!(damage.is_clean());

        damage.mark_statusline();
        damage.mark_command_line();
        damage.mark_editor_row(3);
        damage.mark_editor_rows(5..8);

        assert!(!damage.requires_full_render());
        assert!(damage.statusline());
        assert!(damage.command_line());
        assert_eq!(damage.dirty_editor_rows(), vec![3, 5, 6, 7]);

        damage.mark_full();

        assert!(damage.requires_full_render());
        assert!(!damage.statusline());
        assert!(!damage.command_line());
        assert!(damage.dirty_editor_rows().is_empty());

        damage.clear_after_full_render();

        assert!(damage.is_clean());
    }
}
