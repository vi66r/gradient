//! Spaces: sandboxed sets of tabs. Each Space has its own WebKit data store
//! (cookies, site storage, sign-ins), tabs, recently closed tabs, history
//! and color, and one window shows one Space at a time. The Private Space is built in:
//! its pages share one throwaway store, it is never saved, and it exists only
//! while it has tabs.
//!
//! The Space on screen keeps its tabs in `Browser::tabs` (with `active` and
//! `closed`), so tab code needs no notion of Spaces. Switching parks those
//! fields under the outgoing Space and swaps in the incoming Space's. Parked
//! pages keep running; `tab_mut` finds their tabs for engine events.
use crate::model::{
    MAX_SPACES, SPACE_COLORS, SPACE_NAME_MAX, START, SavedSpace, SavedTab, Session,
};
use crate::{Browser, Tab, engine, lifecycle};
use gpui::{Action, App, Context, Pixels, Point, PromptLevel, Window};
use std::mem::take;

pub(crate) struct Space {
    /// Names the Space's WebKit data store; also keys its site permissions
    /// and history.
    pub id: [u8; 16],
    pub name: String,
    pub color: u8,
    pub private: bool,
}

/// A Space that isn't on screen. Its pages keep running.
#[derive(Default)]
pub(crate) struct Parked {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub closed: Vec<SavedTab>,
}

/// Commands from a Space's context menu, which acts on the Space that was
/// right-clicked rather than the one on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub(crate) enum SpaceCommand {
    Rename,
    Color(u8),
    MoveLeft,
    MoveRight,
    Delete,
}

#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct SpaceMenu {
    pub space: [u8; 16],
    pub command: SpaceCommand,
}

/// Brings the Nth Space (from zero) on screen; bound to ⌃1…⌃9.
#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct SwitchSpace {
    pub index: usize,
}

fn new_id() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

/// Tabs for a saved Space, numbered from `next` so IDs stay unique across
/// every Space.
fn restore(saved: Vec<SavedTab>, next: &mut u64) -> Vec<Tab> {
    let mut tabs: Vec<Tab> = saved
        .into_iter()
        .map(|saved| {
            let mut tab = Tab::new(*next, saved.url, false);
            tab.saved.title = saved.title;
            *next += 1;
            tab
        })
        .collect();
    if tabs.is_empty() {
        tabs.push(Tab::new(*next, START.into(), false));
        *next += 1;
    }
    tabs
}

/// The restored Spaces, which one is on screen, and what the others park.
pub(crate) struct Restored {
    pub spaces: Vec<Space>,
    pub current: usize,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub parked: std::collections::HashMap<[u8; 16], Parked>,
    pub next_id: u64,
}

pub(crate) fn restore_session(session: Session) -> Restored {
    let mut next = 1;
    let current = session.current.min(session.spaces.len() - 1);
    let mut restored = Restored {
        spaces: Vec::new(),
        current,
        tabs: Vec::new(),
        active: 0,
        parked: Default::default(),
        next_id: 0,
    };
    for (index, saved) in session.spaces.into_iter().enumerate() {
        // `Session::validate` already checked every ID.
        let id = saved.profile().unwrap_or_else(new_id);
        let tabs = restore(saved.tabs, &mut next);
        let active = saved.active.min(tabs.len() - 1);
        restored.spaces.push(Space {
            id,
            name: saved.name,
            color: saved.color,
            private: false,
        });
        if index == current {
            restored.tabs = tabs;
            restored.active = active;
        } else {
            restored.parked.insert(
                id,
                Parked {
                    tabs,
                    active,
                    closed: Vec::new(),
                },
            );
        }
    }
    restored.next_id = next;
    restored
}

impl Browser {
    pub(crate) fn current_space(&self) -> &Space {
        &self.spaces[self.space]
    }

    /// Every tab in every Space, the one on screen first.
    pub(crate) fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs
            .iter()
            .chain(self.parked.values().flat_map(|parked| parked.tabs.iter()))
    }

    /// A tab by ID in any Space; engine events arrive for parked pages too.
    pub(crate) fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs
            .iter_mut()
            .chain(
                self.parked
                    .values_mut()
                    .flat_map(|parked| parked.tabs.iter_mut()),
            )
            .find(|tab| tab.id == id)
    }

    /// Which Space holds a tab, by index.
    pub(crate) fn space_of(&self, id: u64) -> Option<usize> {
        if self.tabs.iter().any(|tab| tab.id == id) {
            return Some(self.space);
        }
        let (space, _) = self
            .parked
            .iter()
            .find(|(_, parked)| parked.tabs.iter().any(|tab| tab.id == id))?;
        self.spaces
            .iter()
            .position(|candidate| candidate.id == *space)
    }

    /// Tells the engine which sites each saved Space allows to notify.
    pub(crate) fn sync_notification_sites(&self) {
        for space in self.spaces.iter().filter(|space| !space.private) {
            engine::set_notification_sites(
                space.id,
                self.permissions
                    .sites(&space.id, crate::permissions::Kind::Notifications),
            );
        }
    }

    pub(crate) fn switch_space(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index == self.space || index >= self.spaces.len() {
            return;
        }
        let previous = self.space;
        self.end_find();
        self.renaming = false;
        for tab in &self.tabs {
            if let Some(page) = &tab.page {
                page.hide();
            }
        }
        let outgoing = Parked {
            tabs: take(&mut self.tabs),
            active: self.active,
            closed: take(&mut self.closed),
        };
        self.parked.insert(self.spaces[self.space].id, outgoing);
        let incoming = self
            .parked
            .remove(&self.spaces[index].id)
            .unwrap_or_default();
        self.space = index;
        self.profile = self.spaces[index].id;
        self.tabs = incoming.tabs;
        self.closed = incoming.closed;
        if self.tabs.is_empty() {
            let id = engine::next_id(&self.ids);
            self.tabs
                .push(Tab::new(id, START.into(), self.spaces[index].private));
        }
        self.active = incoming.active.min(self.tabs.len() - 1);
        self.space_slide = if index > previous { 1. } else { -1. };
        self.space_epoch = self.space_epoch.wrapping_add(1);
        self.space_scroll.scroll_to_item(index);
        self.activate(self.active, window, cx);
        // Parked pages may have started or stopped playing while away.
        let live: Vec<u64> = self
            .tabs
            .iter()
            .filter(|tab| tab.page.is_some())
            .map(|tab| tab.id)
            .collect();
        for id in live {
            self.media_changed(id, cx);
        }
        for index in 0..self.tabs.len() {
            self.request_icon(index);
        }
        self.refresh_extension_actions();
        self.save();
    }

    pub(crate) fn switch_space_by(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.spaces.len() as isize;
        let index = (self.space as isize + step).rem_euclid(count) as usize;
        self.switch_space(index, window, cx);
    }

    /// A two-finger sideways swipe over the tabs moves one Space, like
    /// turning a page: fingers left for the next, right for the previous.
    /// It stops at the ends instead of wrapping around.
    pub(crate) fn space_swipe(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui::{ScrollDelta, TouchPhase};
        const DISTANCE: f32 = 90.;
        match event.touch_phase {
            TouchPhase::Started => self.swipe = Some(0.),
            TouchPhase::Moved => {
                let ScrollDelta::Pixels(delta) = event.delta else {
                    return;
                };
                let Some(travel) = self.swipe.as_mut() else {
                    return;
                };
                let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                // Mostly sideways only; scrolling the tab list stays vertical.
                if dx.abs() > dy.abs() * 1.5 {
                    *travel += dx;
                }
                if travel.abs() >= DISTANCE {
                    let step: isize = if *travel < 0. { 1 } else { -1 };
                    self.swipe = None;
                    let target = self.space as isize + step;
                    if (0..self.spaces.len() as isize).contains(&target) {
                        self.switch_space(target as usize, window, cx);
                    }
                }
            }
            _ => self.swipe = None,
        }
    }

    /// Brings the Private Space on screen, opening it if needed.
    pub(crate) fn enter_private(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_space().private {
            return;
        }
        self.before_private = Some(self.current_space().id);
        let index = match self.spaces.iter().position(|space| space.private) {
            Some(index) => index,
            None => {
                self.spaces.push(Space {
                    id: new_id(),
                    name: "Private".into(),
                    color: 0,
                    private: true,
                });
                // Before its first page joins the controller.
                self.apply_private_extensions(cx);
                self.spaces.len() - 1
            }
        };
        self.switch_space(index, window, cx);
    }

    /// The Private Space's last tab closed: it goes away with its website
    /// data, and the Space before it comes back.
    pub(crate) fn leave_private(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.spaces.iter().position(|space| space.private) else {
            return;
        };
        let id = self.spaces[index].id;
        if self.space == index {
            let back = self
                .before_private
                .and_then(|id| self.spaces.iter().position(|space| space.id == id))
                .or_else(|| self.spaces.iter().position(|space| !space.private))
                .unwrap_or(0);
            self.switch_space(back, window, cx);
        }
        self.parked.remove(&id);
        self.spaces.remove(index);
        if index < self.space {
            self.space -= 1;
        }
        self.before_private = None;
        engine::forget_extensions(id, false);
        engine::forget_private_store(id);
        self.prune_downloads();
        cx.notify();
    }

    pub(crate) fn new_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self.spaces.iter().filter(|space| !space.private).count();
        if saved >= MAX_SPACES {
            self.notice = Some(format!("Browser keeps at most {MAX_SPACES} Spaces."));
            cx.notify();
            return;
        }
        let used: Vec<u8> = self.spaces.iter().map(|space| space.color).collect();
        let color = (1..SPACE_COLORS)
            .find(|color| !used.contains(color))
            .unwrap_or(saved as u8 % SPACE_COLORS);
        let name = (saved + 1..)
            .map(|number| format!("Space {number}"))
            .find(|name| !self.spaces.iter().any(|space| &space.name == name))
            .unwrap_or_default();
        // The Private Space, when open, stays last.
        let position = self
            .spaces
            .iter()
            .position(|space| space.private)
            .unwrap_or(self.spaces.len());
        self.spaces.insert(
            position,
            Space {
                id: new_id(),
                name,
                color,
                private: false,
            },
        );
        if position <= self.space {
            self.space += 1;
        }
        self.switch_space(position, window, cx);
        self.begin_rename(window, cx);
    }

    /// Edits the current Space's name in place, in the switcher.
    pub(crate) fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_space().private {
            return;
        }
        let name = self.current_space().name.clone();
        self.space_name.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.renaming = true;
        cx.notify();
    }

    pub(crate) fn finish_rename(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.renaming {
            return;
        }
        self.renaming = false;
        if commit {
            let name: String = self
                .space_name
                .read(cx)
                .value()
                .trim()
                .chars()
                .take(SPACE_NAME_MAX)
                .collect();
            if !name.is_empty() {
                self.spaces[self.space].name = name;
                self.save();
            }
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn space_menu(
        &self,
        id: [u8; 16],
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        use gpui::component::native_menu::NativeMenu;
        let Some(index) = self.spaces.iter().position(|space| space.id == id) else {
            return;
        };
        let space = &self.spaces[index];
        let item = |command| Box::new(SpaceMenu { space: id, command }) as Box<dyn Action>;
        if space.private {
            return;
        }
        let mut colors = NativeMenu::new();
        for color in 0..SPACE_COLORS {
            colors = colors.menu_with_check(
                crate::ui::space_color_name(color),
                space.color == color,
                item(SpaceCommand::Color(color)),
            );
        }
        let saved = self.spaces.iter().filter(|space| !space.private).count();
        NativeMenu::new()
            .menu("Rename…", item(SpaceCommand::Rename))
            .submenu("Color", colors)
            .separator()
            .menu_with_disabled("Move Left", index == 0, item(SpaceCommand::MoveLeft))
            .menu_with_disabled(
                "Move Right",
                index + 1 >= saved,
                item(SpaceCommand::MoveRight),
            )
            .separator()
            .menu_with_disabled("Delete Space…", saved < 2, item(SpaceCommand::Delete))
            .show(position, window, cx);
    }

    pub(crate) fn space_command(
        &mut self,
        action: &SpaceMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .spaces
            .iter()
            .position(|space| space.id == action.space)
        else {
            return;
        };
        match action.command {
            SpaceCommand::Rename => {
                self.switch_space(index, window, cx);
                self.begin_rename(window, cx);
            }
            SpaceCommand::Color(color) if color < SPACE_COLORS => {
                self.spaces[index].color = color;
                self.save();
            }
            SpaceCommand::Color(_) => {}
            SpaceCommand::MoveLeft if index > 0 => self.swap_spaces(index - 1, index),
            SpaceCommand::MoveRight
                if index + 1 < self.spaces.len() && !self.spaces[index + 1].private =>
            {
                self.swap_spaces(index, index + 1)
            }
            SpaceCommand::MoveLeft | SpaceCommand::MoveRight => {}
            SpaceCommand::Delete => self.delete_space(action.space, window, cx),
        }
        cx.notify();
    }

    fn swap_spaces(&mut self, left: usize, right: usize) {
        self.spaces.swap(left, right);
        if self.space == left {
            self.space = right;
        } else if self.space == right {
            self.space = left;
        }
        self.save();
    }

    /// Asks first: deleting erases the Space's website data. Unsaved page
    /// edits then get the usual work-protection check.
    pub(crate) fn delete_space(
        &mut self,
        id: [u8; 16],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.spaces.iter().position(|space| space.id == id) else {
            return;
        };
        if self.spaces[index].private
            || self.spaces.iter().filter(|space| !space.private).count() < 2
        {
            return;
        }
        let ids: Vec<u64> = if index == self.space {
            self.tabs.iter().map(|tab| tab.id).collect()
        } else {
            self.parked
                .get(&id)
                .map(|parked| parked.tabs.iter().map(|tab| tab.id).collect())
                .unwrap_or_default()
        };
        let tabs = match ids.len() {
            0 | 1 => "Its tab closes".to_string(),
            count => format!("Its {count} tabs close"),
        };
        let detail = format!(
            "{tabs}, and the website data it keeps (cookies, sign-ins and site storage) and its history are erased. This can’t be undone."
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete “{}”?", self.spaces[index].name),
            Some(&detail),
            &["Cancel", "Delete Space"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.request_discard(
                        lifecycle::Discard::DeleteSpace { space: id, ids },
                        window,
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    pub(crate) fn delete_space_unchecked(
        &mut self,
        id: [u8; 16],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.spaces.iter().position(|space| space.id == id) else {
            return;
        };
        if index == self.space {
            let neighbor = (0..self.spaces.len())
                .filter(|&other| other != index && !self.spaces[other].private)
                .min_by_key(|&other| other.abs_diff(index));
            let Some(neighbor) = neighbor else {
                return;
            };
            self.switch_space(neighbor, window, cx);
        }
        let index = self
            .spaces
            .iter()
            .position(|space| space.id == id)
            .unwrap_or(index);
        self.spaces.remove(index);
        if index < self.space {
            self.space -= 1;
        }
        // Closing its pages releases the data store; WebKit then erases it.
        drop(self.parked.remove(&id));
        self.prune_downloads();
        if let Err(error) = self.permissions.forget_profile(&id) {
            self.notice = Some(error.to_string());
        }
        if let Err(error) = self.history.forget_space(id) {
            self.notice = Some(format!("The Space’s history couldn’t be erased: {error}"));
        }
        self.forget_space_extensions(id, cx);
        engine::set_notification_sites(id, Vec::new());
        let events = self.events.clone();
        cx.spawn(async move |_, cx| {
            // Give the pages' web processes a moment to let go of the store.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            cx.update(|_| engine::remove_data_store(id, events));
        })
        .detach();
        self.save();
        cx.notify();
    }

    /// Reloads a tab in another Space: Spaces don't share website data, so
    /// the page can't carry over. Unsaved edits are checked first.
    pub(crate) fn move_tab_unchecked(
        &mut self,
        id: u64,
        target: [u8; 16],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if target == self.current_space().id
            || !self
                .spaces
                .iter()
                .any(|space| space.id == target && !space.private)
        {
            return;
        }
        let url = self.tab_url(index);
        let title = self.tabs[index].saved.title.clone();
        drop(self.tabs.remove(index));
        let mut moved = Tab::new(engine::next_id(&self.ids), url, false);
        moved.saved.title = title;
        let parked = self.parked.entry(target).or_default();
        // It lands on top; the Space's own current tab stays current.
        if !parked.tabs.is_empty() {
            parked.active += 1;
        }
        parked.tabs.insert(0, moved);
        self.prune_downloads();
        if self.tabs.is_empty() {
            self.new_tab(START.into(), false, window, cx);
        } else {
            let active = if index < self.active {
                self.active - 1
            } else {
                self.active.min(self.tabs.len() - 1)
            };
            self.activate(active, window, cx);
        }
        self.save();
    }

    /// The session to save: every Space but the Private Space.
    pub(crate) fn session_snapshot(&self) -> Session {
        let anchor = if self.current_space().private {
            self.before_private
        } else {
            Some(self.current_space().id)
        };
        let mut session = Session {
            version: 2,
            spaces: Vec::new(),
            current: 0,
        };
        for space in self.spaces.iter().filter(|space| !space.private) {
            let (tabs, active) = if space.id == self.current_space().id {
                (&self.tabs[..], self.active)
            } else {
                self.parked
                    .get(&space.id)
                    .map_or((&[][..], 0), |parked| (&parked.tabs[..], parked.active))
            };
            let mut saved: Vec<SavedTab> = tabs.iter().map(|tab| tab.saved.clone()).collect();
            if saved.is_empty() {
                saved.push(SavedTab {
                    url: START.into(),
                    title: "New Tab".into(),
                });
            }
            if anchor == Some(space.id) {
                session.current = session.spaces.len();
            }
            session.spaces.push(SavedSpace {
                id: uuid::Uuid::from_bytes(space.id).to_string(),
                name: space.name.clone(),
                color: space.color,
                active: active.min(saved.len() - 1),
                tabs: saved,
            });
        }
        session
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_tab_ids_are_unique_across_spaces() {
        let mut session = Session::fresh([7; 16]);
        let mut work = session.spaces[0].clone();
        work.id = uuid::Uuid::from_bytes([9; 16]).to_string();
        work.tabs.push(SavedTab {
            url: "https://example.com/".into(),
            title: "Example".into(),
        });
        work.active = 5;
        session.spaces.push(work);
        session.current = 1;
        let restored = restore_session(session);
        assert_eq!(restored.spaces.len(), 2);
        assert_eq!(restored.current, 1);
        assert_eq!(restored.tabs.len(), 2);
        // An out-of-range active tab clamps instead of panicking.
        assert_eq!(restored.active, 1);
        let parked = &restored.parked[&[7; 16]];
        let mut ids: Vec<u64> = restored
            .tabs
            .iter()
            .chain(&parked.tabs)
            .map(|tab| tab.id)
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        assert!(restored.next_id > *ids.last().unwrap());
    }
}
