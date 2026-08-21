//! Overlay pickers: the model selector (Ctrl+P / /models) and the session
//! selector (/sessions). Each owns a filtered list plus the current cursor.
pub(crate) struct ModelPicker {
    pub(crate) query: Vec<char>,
    pub(crate) models: Vec<String>,
    pub(crate) selected: usize,
}

impl ModelPicker {
    pub(crate) fn new(models: Vec<String>) -> Self {
        Self {
            query: Vec::new(),
            models,
            selected: 0,
        }
    }

    pub(crate) fn filtered(&self) -> Vec<&str> {
        let query: String = self.query.iter().collect();
        let query = query.to_lowercase();
        if query.is_empty() {
            return self.models.iter().map(|m| m.as_str()).collect();
        }
        self.models
            .iter()
            .filter(|m| m.to_lowercase().contains(&query))
            .map(|m| m.as_str())
            .collect()
    }

    pub(crate) fn clamp(&mut self) {
        let count = self.filtered().len();
        self.selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
    }

    pub(crate) fn selected_model(&self) -> Option<String> {
        self.filtered().get(self.selected).map(|m| m.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Selection {
    pub(crate) anchor_row: usize,
    pub(crate) anchor_col: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
}

impl Selection {
    pub(crate) fn normalized(self) -> ((usize, usize), (usize, usize)) {
        if (self.anchor_row, self.anchor_col) <= (self.row, self.col) {
            ((self.anchor_row, self.anchor_col), (self.row, self.col))
        } else {
            ((self.row, self.col), (self.anchor_row, self.anchor_col))
        }
    }
}

pub(crate) struct SessionPicker {
    pub(crate) query: Vec<char>,
    pub(crate) sessions: Vec<firment_core::SessionSummary>,
    pub(crate) selected: usize,
}

impl SessionPicker {
    pub(crate) fn new(sessions: Vec<firment_core::SessionSummary>) -> Self {
        Self {
            query: Vec::new(),
            sessions,
            selected: 0,
        }
    }

    pub(crate) fn filtered(&self) -> Vec<&firment_core::SessionSummary> {
        let query: String = self.query.iter().collect();
        let query = query.to_lowercase();
        if query.is_empty() {
            return self.sessions.iter().collect();
        }
        self.sessions
            .iter()
            .filter(|s| {
                s.id.to_lowercase().contains(&query)
                    || s.model.to_lowercase().contains(&query)
                    || s.preview.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub(crate) fn clamp(&mut self) {
        let count = self.filtered().len();
        self.selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
    }
}
