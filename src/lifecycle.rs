//! Work protection is a conservative heuristic, not the beforeunload contract.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct WorkState {
    pub risk: u8,
    pub stamp: String,
}

impl WorkState {
    pub fn unknown() -> Self {
        Self {
            risk: 3,
            stamp: String::new(),
        }
    }
    pub fn clear(&self) -> bool {
        self.risk == 0
    }
    pub fn parse(json: &str) -> Self {
        if json.len() > 16384 {
            return Self::unknown();
        }
        serde_json::from_str::<Self>(json)
            .ok()
            .filter(|state| state.risk <= 3 && !state.stamp.is_empty())
            .unwrap_or_else(Self::unknown)
    }
    pub fn still_covered(&self, current: &Self) -> bool {
        self == current
            || (current.clear() && self.stamp.split(':').next() == current.stamp.split(':').next())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
    Close(u64),
    Suspend(u64),
    Start(u64),
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LivePage {
    pub id: u64,
    pub revision: u64,
}

impl Discard {
    pub fn includes(self, id: u64) -> bool {
        match self {
            Self::Close(target) | Self::Suspend(target) | Self::Start(target) => target == id,
            Self::Quit => true,
        }
    }

    pub fn labels(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Close(_) => (
                "Close this tab?",
                "Close tab",
                "Unsaved forms, calls, and other live page state will be lost. Reopening a tab restores its address only. Private tabs cannot be reopened.",
            ),
            Self::Suspend(_) => (
                "Suspend this tab?",
                "Suspend",
                "Unsaved forms, calls, and other live page state will be lost. Resuming reloads the address only.",
            ),
            Self::Start(_) => (
                "Return to the start page?",
                "Leave page",
                "Unsaved forms, calls, and other live page state will be lost. Open a new tab instead to keep this page.",
            ),
            Self::Quit => (
                "Quit Browser?",
                "Quit Browser",
                "Unsaved forms, calls, and other live page state will be lost in all tabs. Only normal tab addresses are restored next time. Private tabs and Request Lab drafts are not saved.",
            ),
        }
    }
}

/// Consent is for the exact set of pages shown when the prompt opened. A page
/// can navigate through an agent or script while a native sheet is displayed.
pub fn consent_matches(expected: &[LivePage], current: &[LivePage]) -> bool {
    expected.len() == current.len() && expected.iter().all(|page| current.contains(page))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(id: u64, revision: u64) -> LivePage {
        LivePage { id, revision }
    }

    #[test]
    fn consent_survives_tab_reordering_but_not_navigation_or_replacement() {
        let expected = [page(1, 2), page(5, 3)];
        assert!(consent_matches(&expected, &[page(5, 3), page(1, 2)]));
        assert!(!consent_matches(&expected, &[page(1, 3), page(5, 3)]));
        assert!(!consent_matches(&expected, &[page(1, 2), page(6, 3)]));
    }

    #[test]
    fn quit_consent_cannot_cover_newly_loaded_or_removed_pages() {
        assert!(!consent_matches(&[page(1, 2)], &[page(1, 2), page(2, 1)]));
        assert!(!consent_matches(&[page(1, 2)], &[]));
        assert!(consent_matches(&[], &[]));
    }

    #[test]
    fn single_tab_actions_never_target_a_new_active_tab() {
        for action in [Discard::Close(7), Discard::Suspend(7), Discard::Start(7)] {
            assert!(action.includes(7));
            assert!(!action.includes(8));
        }
        assert!(Discard::Quit.includes(8));
    }

    #[test]
    fn invalid_or_missing_probe_results_are_never_clear() {
        for json in [
            "null",
            "{}",
            "",
            r#"{"risk":0,"stamp":""}"#,
            r#"{"risk":4,"stamp":"doc:0"}"#,
        ] {
            assert!(!WorkState::parse(json).clear());
        }
        assert!(!WorkState::parse(&"x".repeat(16385)).clear());
    }

    #[test]
    fn consent_allows_saved_work_but_not_a_new_document_or_new_edits() {
        let dirty = WorkState::parse(r#"{"risk":1,"stamp":"doc:1"}"#);
        assert!(dirty.still_covered(&dirty));
        assert!(dirty.still_covered(&WorkState::parse(r#"{"risk":0,"stamp":"doc:2"}"#)));
        assert!(!dirty.still_covered(&WorkState::parse(r#"{"risk":1,"stamp":"doc:2"}"#)));
        assert!(!dirty.still_covered(&WorkState::parse(r#"{"risk":0,"stamp":"other:1"}"#)));
        assert!(!dirty.still_covered(&WorkState::unknown()));
    }
}
