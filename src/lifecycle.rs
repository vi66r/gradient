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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discard {
    Close(u64),
    /// Close an app window (`apps.rs`), named by its tab.
    CloseApp(u64),
    /// Bulk close from a tab's menu. `keep` is the tab the menu belongs to; it
    /// becomes active if the active tab is among those closed.
    CloseTabs {
        keep: u64,
        ids: Vec<u64>,
    },
    Suspend(u64),
    Start(u64),
    /// Reload a tab in another Space (`space` names it).
    MoveTab {
        id: u64,
        space: [u8; 16],
    },
    /// Close every tab of a Space and erase it.
    DeleteSpace {
        space: [u8; 16],
        ids: Vec<u64>,
    },
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LivePage {
    pub id: u64,
    pub revision: u64,
}

impl Discard {
    pub fn includes(&self, id: u64) -> bool {
        match self {
            Self::Close(target)
            | Self::CloseApp(target)
            | Self::Suspend(target)
            | Self::Start(target) => *target == id,
            Self::MoveTab { id: target, .. } => *target == id,
            Self::CloseTabs { ids, .. } | Self::DeleteSpace { ids, .. } => ids.contains(&id),
            Self::Quit => true,
        }
    }

    pub fn labels(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::CloseTabs { .. } => (
                "Close these tabs?",
                "Close tabs",
                "Some of these tabs have edits you haven’t saved. Closing discards them; reopening restores addresses only, and private tabs can’t be reopened.",
            ),
            Self::Close(_) => (
                "Close this tab?",
                "Close tab",
                "This page has edits you haven’t saved. Closing discards them; reopening restores the address only, and private tabs can’t be reopened.",
            ),
            Self::CloseApp(_) => (
                "Close this window?",
                "Close window",
                "This page has edits you haven’t saved. Closing discards them; reopening restores the address only.",
            ),
            Self::Suspend(_) => (
                "Suspend this tab?",
                "Suspend",
                "This page has edits you haven’t saved. Suspending discards them; resuming reloads the address.",
            ),
            Self::Start(_) => (
                "Return to the start page?",
                "Leave page",
                "This page has edits you haven’t saved. Open a new tab instead to keep them.",
            ),
            Self::MoveTab { .. } => (
                "Move this tab?",
                "Move tab",
                "This page has edits you haven’t saved. Spaces don’t share website data, so the page reloads in the other Space and the edits are discarded.",
            ),
            Self::DeleteSpace { .. } => (
                "Delete this Space?",
                "Delete Space",
                "Some of its tabs have edits you haven’t saved. Deleting the Space discards them.",
            ),
            Self::Quit => (
                "Quit Browser?",
                "Quit Browser",
                "Some tabs have edits you haven’t saved. Quitting discards them. Tab addresses are restored next time, except private tabs; Request Lab drafts aren’t saved.",
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
        let bulk = Discard::CloseTabs {
            keep: 1,
            ids: vec![2, 3],
        };
        assert!(bulk.includes(2) && bulk.includes(3));
        assert!(!bulk.includes(1) && !bulk.includes(4));
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
