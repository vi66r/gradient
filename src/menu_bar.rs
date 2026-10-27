//! The menu bar, laid out the way Mac browsers are (Safari's order, with a
//! Tab menu and Spaces). Some items follow the browser: the Spaces list,
//! and the Tab menu's labels for the active tab. The bar is rebuilt only
//! when those change; see `sync_menus`.
//!
//! GPUI enables an item whenever its action has a handler, so items that
//! wouldn't apply are reworded or left out rather than disabled.
use crate::agent::Access;
use crate::{
    ActualSize, AddExtension, Back, Browser, ClearHistory, CloseTab, CopyUrl, DeleteSpace, Find,
    FindNext, FindPrevious, FocusAddress, Forward, Inspect, NewPrivateTab, NewSpace, NewTab,
    NextSpace, NextTab, OpenFile, PopIn, PopOut, PreviousSpace, PreviousTab, Print, Quit, Reload,
    RenameSpace, ReopenTab, RequestLab, ShowHistory, ShowSettings, SitePermissions, TabCommand,
    TabMenu, ToggleAgent, ToggleSidebar, ZoomIn, ZoomOut, model, spaces,
};
use gpui::component::input;
use gpui::{Action, Context, Menu, MenuItem, OsAction, SystemMenuType, Window, actions};

actions!(
    browser,
    [
        About, Hide, HideOthers, ShowAll, Minimize, ZoomWindow, EditUndo, EditRedo
    ]
);

/// ⌘1…⌘8 pick a tab by position; ⌘9 is always the last tab.
#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct SelectTab {
    pub index: usize,
}

/// The browser state the bar shows.
#[derive(Clone, PartialEq)]
pub(crate) struct MenuState {
    tab: u64,
    live: bool,
    agent: Option<Agent>,
    /// Other regular Spaces the tab can move to.
    elsewhere: Vec<([u8; 16], String)>,
    spaces: Vec<String>,
    space: usize,
    compact: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Agent {
    Share,
    StopSharing,
    StopTrusting,
}

impl Agent {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Share => "Share with Agents…",
            Self::StopSharing => "Stop Sharing with Agents",
            Self::StopTrusting => "Stop Trusting Agents on This Site…",
        }
    }
}

impl Browser {
    /// What the tab's agent item does: `None` where agents never reach
    /// (private tabs and local files).
    pub(crate) fn agent_item(&self, tab: &crate::Tab) -> Option<Agent> {
        if tab.private || model::is_file_url(&tab.saved.url) {
            return None;
        }
        Some(match self.agent_access(tab) {
            Some(Access::Trusted { .. }) => Agent::StopTrusting,
            Some(Access::Shared { .. }) => Agent::StopSharing,
            None => Agent::Share,
        })
    }

    fn menu_state(&self) -> MenuState {
        let tab = &self.tabs[self.active];
        let agent = self.agent_item(tab);
        let elsewhere = if tab.private {
            Vec::new()
        } else {
            self.spaces
                .iter()
                .filter(|space| !space.private && space.id != self.profile)
                .map(|space| (space.id, space.name.clone()))
                .collect()
        };
        MenuState {
            tab: tab.id,
            live: tab.page.is_some(),
            agent,
            elsewhere,
            spaces: self.spaces.iter().map(|space| space.name.clone()).collect(),
            space: self.space,
            compact: self.settings.compact_sidebar,
        }
    }

    /// Called on every render: rebuilds the bar only when what it shows
    /// has changed.
    pub(crate) fn sync_menus(&mut self, cx: &mut Context<Self>) {
        let state = self.menu_state();
        if self.menus.as_ref() == Some(&state) {
            return;
        }
        self.menus = Some(state.clone());
        cx.defer(move |cx| {
            cx.set_menus(menus(&state));
            crate::engine::native_window_menu();
        });
    }

    pub(crate) fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.tabs.len() - 1;
        let index = if index == 8 { last } else { index.min(last) };
        self.activate(index, window, cx);
    }

    /// Edit ▸ Undo and Redo: the page's own history when it has the
    /// keyboard, otherwise the focused field's.
    pub(crate) fn edit_undo(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if crate::engine::page_undo(redo) {
            return;
        }
        if redo {
            window.dispatch_action(Box::new(input::Redo), cx);
        } else {
            window.dispatch_action(Box::new(input::Undo), cx);
        }
    }
}

pub(crate) fn menus(state: &MenuState) -> Vec<Menu> {
    let tab = |command| TabMenu {
        id: state.tab,
        command,
    };
    let mut tab_menu = vec![
        MenuItem::action("Next Tab", NextTab),
        MenuItem::action("Previous Tab", PreviousTab),
        MenuItem::separator(),
        MenuItem::action("Copy Address", CopyUrl),
        MenuItem::action("Duplicate Tab", tab(TabCommand::Duplicate)),
        MenuItem::action("Open in App Window", PopOut),
        MenuItem::action("Move to Browser Window", PopIn),
        if state.live {
            MenuItem::action("Suspend Tab", tab(TabCommand::Suspend))
        } else {
            MenuItem::action("Resume Tab", tab(TabCommand::Resume))
        },
    ];
    if !state.elsewhere.is_empty() {
        tab_menu.push(MenuItem::submenu(
            Menu::new("Move to Space").items(
                state
                    .elsewhere
                    .iter()
                    .map(|(id, name)| MenuItem::action(name.clone(), tab(TabCommand::MoveTo(*id)))),
            ),
        ));
    }
    tab_menu.push(MenuItem::separator());
    if let Some(agent) = state.agent {
        tab_menu.push(MenuItem::action(agent.label(), ToggleAgent));
    }
    tab_menu.extend([
        MenuItem::action("Site Permissions…", SitePermissions),
        MenuItem::separator(),
        MenuItem::action("Close Other Tabs", tab(TabCommand::CloseOthers)),
        MenuItem::action("Close Tabs Below", tab(TabCommand::CloseBelow)),
    ]);

    let mut spaces_menu = vec![
        MenuItem::action("New Space", NewSpace),
        MenuItem::action("Rename Space…", RenameSpace),
        MenuItem::action("Delete Space…", DeleteSpace),
        MenuItem::separator(),
        MenuItem::action("Next Space", NextSpace),
        MenuItem::action("Previous Space", PreviousSpace),
        MenuItem::separator(),
    ];
    spaces_menu.extend(state.spaces.iter().enumerate().map(|(index, name)| {
        MenuItem::action(name.clone(), spaces::SwitchSpace { index }).checked(index == state.space)
    }));

    vec![
        Menu::new("Browser").items([
            MenuItem::action("About Browser", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", ShowSettings),
            MenuItem::action("Add Extension…", AddExtension),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Browser", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Browser", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("New Private Tab", NewPrivateTab),
            MenuItem::action("Open Location…", FocusAddress),
            MenuItem::action("Open File…", OpenFile),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::separator(),
            MenuItem::action("Print…", Print),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", EditUndo),
            MenuItem::action("Redo", EditRedo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Find").items([
                MenuItem::action("Find…", Find),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
            ])),
        ]),
        Menu::new("View").items([
            MenuItem::action(
                if state.compact {
                    "Expand Sidebar"
                } else {
                    "Collapse Sidebar"
                },
                ToggleSidebar,
            ),
            MenuItem::separator(),
            MenuItem::action("Reload Page", Reload),
            MenuItem::separator(),
            MenuItem::action("Actual Size", ActualSize),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            // AppKit adds Enter Full Screen below, in a section of its own.
            MenuItem::separator(),
        ]),
        Menu::new("History").items([
            MenuItem::action("Back", Back),
            MenuItem::action("Forward", Forward),
            MenuItem::separator(),
            MenuItem::action("Show All History", ShowHistory),
            MenuItem::action("Reopen Closed Tab", ReopenTab),
            MenuItem::separator(),
            MenuItem::action("Clear History…", ClearHistory),
        ]),
        Menu::new("Spaces").items(spaces_menu),
        Menu::new("Tab").items(tab_menu),
        Menu::new("Develop").items([
            MenuItem::action("Request Lab", RequestLab),
            MenuItem::action("Inspect Page", Inspect),
        ]),
        // Named "Window", AppKit adds window tiling and the window list.
        // Minimize and Zoom go to AppKit; see `engine::native_window_menu`.
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", ZoomWindow),
        ]),
        // AppKit fills it with the search field that finds menu items.
        Menu::new("Help"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(agent: Option<Agent>, live: bool) -> MenuState {
        MenuState {
            tab: 1,
            live,
            agent,
            elsewhere: Vec::new(),
            spaces: vec!["Personal".into(), "Work".into()],
            space: 1,
            compact: false,
        }
    }

    fn names(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                MenuItem::Submenu(menu) => Some(menu.name.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_bar_follows_the_mac_order() {
        let bar = menus(&state(Some(Agent::Share), true));
        let titles: Vec<&str> = bar.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(
            titles,
            [
                "Browser", "File", "Edit", "View", "History", "Spaces", "Tab", "Develop", "Window",
                "Help"
            ]
        );
    }

    #[test]
    fn the_tab_menu_names_what_its_items_will_do() {
        let tab = |agent, live| names(&menus(&state(agent, live))[6]);
        assert!(tab(Some(Agent::Share), true).contains(&"Share with Agents…".into()));
        assert!(
            tab(Some(Agent::StopTrusting), true)
                .contains(&"Stop Trusting Agents on This Site…".into())
        );
        assert!(!tab(None, true).iter().any(|name| name.contains("Agents")));
        assert!(tab(None, false).contains(&"Resume Tab".into()));
    }

    #[test]
    fn the_spaces_menu_checks_the_space_on_screen() {
        let spaces = &menus(&state(None, true))[5];
        let checked: Vec<String> = spaces
            .items
            .iter()
            .filter(|item| item.is_checked())
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(checked, ["Work"]);
    }
}
