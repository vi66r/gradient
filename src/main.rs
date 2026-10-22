extern crate gpui_kit as gpui;
mod agent;
mod assets;
mod engine;
mod favicon;
mod http_client;
mod lifecycle;
mod mcp;
mod model;
mod session;
mod ui;

use gpui::component::{
    Theme, ThemeMode,
    input::{self, InputEvent, InputState, TextareaState},
};
use gpui::{prelude::*, *};
use model::{START, SavedTab, Session};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

actions!(
    browser,
    [
        NewTab,
        NewPrivateTab,
        CloseTab,
        ReopenTab,
        FocusAddress,
        CopyUrl,
        Back,
        Forward,
        Reload,
        NextTab,
        PreviousTab,
        RequestLab,
        Inspect,
        Suspend,
        ToggleAgent,
        Quit
    ]
);

struct Tab {
    id: u64,
    saved: SavedTab,
    private: bool,
    loading: bool,
    revision: u64,
    page: Option<Rc<engine::Page>>,
    error: Option<String>,
    grant: Option<(String, u64)>,
}

/// Commands from a tab's context menu. They carry the tab's ID because the
/// menu acts on the tab that was right-clicked, not the active one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
enum TabCommand {
    Reload,
    CopyAddress,
    Duplicate,
    Suspend,
    Resume,
    ToggleAgent,
    Close,
    CloseOthers,
    CloseBelow,
}

#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
struct TabMenu {
    id: u64,
    command: TabCommand,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyTarget {
    Address,
    Curl,
}

struct Browser {
    // Native views must drop before the shared Linux WebContext.
    tabs: Vec<Tab>,
    context: wry::WebContext,
    profile: [u8; 16],
    active: usize,
    next_id: u64,
    closed: Vec<SavedTab>,
    address: Entity<InputState>,
    focus: FocusHandle,
    events: async_channel::Sender<engine::Event>,
    store: session::Store,
    dirty: bool,
    notice: Option<String>,
    copy_feedback: Option<CopyTarget>,
    copy_epoch: u64,
    discard_pending: bool,
    #[cfg(target_os = "macos")]
    termination: Option<engine::TerminationGuard>,
    lab: bool,
    request_method: String,
    request_url: Entity<InputState>,
    request_headers: Entity<TextareaState>,
    request_body: Entity<TextareaState>,
    response: Entity<TextareaState>,
    response_summary: String,
    requesting: bool,
    grant_epoch: u64,
    /// Follows the system appearance; see `ui::palette`.
    dark: bool,
    /// Site icons by host key, with whether the copy is fresh enough to keep.
    icons: HashMap<String, (Arc<Image>, bool)>,
    /// Hosts with a fetch in flight or no usable icon this session.
    icon_requests: HashSet<String>,
    icon_cache: favicon::DiskCache,
    _subscriptions: Vec<Subscription>,
}

impl Browser {
    fn new(
        store: session::Store,
        saved: Session,
        notice: Option<String>,
        profile: [u8; 16],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let address =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search or enter address"));
        let request_url = cx
            .new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1/items"));
        let request_headers = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Accept: application/json\nAuthorization: Bearer …")
                .rows(4)
        });
        let request_body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Request body")
                .rows(6)
        });
        let response = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Your response will appear here.")
                .rows(18)
        });
        let address_events = cx.subscribe_in(&address, window, |this, input, event, window, cx| {
            // The resting field shows the host; focus swaps in the full URL.
            if matches!(event, InputEvent::Focus) {
                this.sync_address(window, cx);
            }
            if matches!(event, InputEvent::Focus | InputEvent::Blur) {
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value().to_string();
                match model::destination(&value) {
                    Ok(url) => this.navigate(url, window, cx),
                    Err(error) => {
                        this.notice = Some(error.to_string());
                        cx.notify();
                    }
                }
            }
        });
        let (events, received) = async_channel::bounded(256);
        #[cfg(target_os = "macos")]
        engine::watch_native_clicks(events.clone());
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = received.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.event(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut tabs: Vec<Tab> = saved
            .tabs
            .into_iter()
            .enumerate()
            .map(|(index, saved)| Tab {
                id: index as u64 + 1,
                saved,
                private: false,
                loading: false,
                revision: 0,
                page: None,
                error: None,
                grant: None,
            })
            .collect();
        if tabs.is_empty() {
            tabs.push(Tab {
                id: 1,
                saved: SavedTab {
                    url: START.into(),
                    title: "New tab".into(),
                },
                private: false,
                loading: false,
                revision: 0,
                page: None,
                error: None,
                grant: None,
            });
        }
        let active = saved.active.min(tabs.len() - 1);
        let next_id = tabs.len() as u64 + 1;
        let icon_cache = favicon::DiskCache::new(store.dir());
        let mut this = Self {
            tabs,
            context: wry::WebContext::new(Some(session::data_dir().join("webkit"))),
            profile,
            active,
            next_id,
            closed: Vec::new(),
            address,
            focus: cx.focus_handle(),
            events,
            store,
            dirty: false,
            notice,
            copy_feedback: None,
            copy_epoch: 0,
            discard_pending: false,
            #[cfg(target_os = "macos")]
            termination: None,
            lab: false,
            request_method: "GET".into(),
            request_url,
            request_headers,
            request_body,
            response,
            response_summary: "Ready when you are".into(),
            requesting: false,
            grant_epoch: 0,
            dark: false,
            icons: HashMap::new(),
            icon_requests: HashSet::new(),
            icon_cache,
            _subscriptions: vec![
                address_events,
                cx.observe_window_appearance(window, |this, window, cx| {
                    this.follow_appearance(window, cx)
                }),
            ],
        };
        this.follow_appearance(window, cx);
        // Restored tabs show cached icons before (or without) loading.
        for tab in this.tabs.iter().filter(|tab| !tab.private) {
            if let Some(key) = favicon::host_key(&tab.saved.url)
                && !this.icons.contains_key(&key)
                && let Some((bytes, fresh)) = this.icon_cache.read(&key)
                && let Some(format) = favicon::sniff(&bytes)
            {
                this.icons
                    .insert(key, (Arc::new(Image::from_bytes(format, bytes)), fresh));
            }
        }
        this.activate(active, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if let Some(page) = &this.tabs[this.active].page
                            && !page.navigation_pending()
                            && let Ok(url) = page.url()
                            && model::allowed_navigation(&url)
                            && url != "about:blank"
                            && url != this.tabs[this.active].saved.url
                        {
                            this.tabs[this.active].saved.url = url;
                            this.revoke_changed_origins();
                            this.dirty = true;
                            if !this.address.read(cx).focus_handle(cx).is_focused(window) {
                                this.sync_address(window, cx);
                            }
                        }
                        for tab in &mut this.tabs {
                            if tab.loading && tab.page.as_ref().is_some_and(|page|page.initial_load_failed()) {
                                tab.page = None;
                                tab.loading = false;
                                tab.grant = None;
                                tab.error = Some("The page could not be reached. Check the address and your connection. For a localhost page, make sure its development server is running, then try again.".into());
                                cx.notify();
                            }
                        }
                        if this.dirty {
                            this.save();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn follow_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let mode = if self.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        Theme::change(mode, Some(window), cx);
        cx.notify();
    }

    fn sync_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        let url = &self.tabs[self.active].saved.url;
        let value = if url == START { "" } else { url };
        self.address.update(cx, |input, cx| {
            input.set_value(value.to_string(), window, cx)
        });
    }

    fn ensure_page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[index];
        if tab.saved.url == START || tab.page.is_some() {
            return;
        }
        match engine::Page::create(
            engine::PageOptions {
                id: tab.id,
                url: &tab.saved.url,
                private: tab.private,
                profile: self.profile,
            },
            &mut self.context,
            self.events.clone(),
            window,
            cx,
        ) {
            Ok(page) => {
                tab.revision = tab.revision.wrapping_add(1);
                tab.page = Some(page);
                tab.loading = true;
                tab.error = None;
            }
            Err(error) => {
                tab.error = Some(format!("Could not open this page: {error}"));
                tab.loading = false;
            }
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        for (tab_index, tab) in self.tabs.iter().enumerate() {
            if tab_index != index
                && let Some(page) = &tab.page
            {
                page.hide();
            }
        }
        self.active = index;
        self.lab = false;
        self.ensure_page(index, window, cx);
        self.sync_address(window, cx);
        window.focus(&self.focus, cx);
        self.dirty = true;
        cx.notify();
    }

    fn new_tab(&mut self, url: String, private: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab_at(0, url, private, window, cx);
    }

    fn new_tab_at(
        &mut self,
        position: usize,
        url: String,
        private: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() >= 1000 {
            self.notice = Some("Tab limit reached.".into());
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        // Newest first: the sidebar reads top-down from what you just opened.
        let position = position.min(self.tabs.len());
        self.tabs.insert(
            position,
            Tab {
                id,
                saved: SavedTab {
                    title: if url == START {
                        "New tab".into()
                    } else {
                        model::display_host(&url)
                    },
                    url,
                },
                private,
                loading: false,
                revision: 0,
                page: None,
                error: None,
                grant: None,
            },
        );
        self.activate(position, window, cx);
        if self.tabs[self.active].saved.url == START {
            self.focus_address(window, cx);
        }
    }

    fn navigate(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if url == START && self.tabs[self.active].page.is_some() {
            self.request_discard(
                lifecycle::Discard::Start(self.tabs[self.active].id),
                window,
                cx,
            );
            return;
        }
        self.navigate_unchecked(url, window, cx);
    }

    fn navigate_unchecked(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.lab = false;
        self.notice = None;
        let tab = &mut self.tabs[self.active];
        tab.error = None;
        if url != START
            && let Some(page) = &tab.page
        {
            if let Err(error) = page.raw.load_url(&url) {
                tab.error = Some(error.to_string());
            }
            // Keep the committed address, title, and grant until WebKit commits.
            // A canceled native navigation must not appear in session recovery.
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        tab.revision = tab.revision.wrapping_add(1);
        if tab
            .grant
            .as_ref()
            .is_some_and(|(origin, _)| mcp::origin(&url).as_ref() != Some(origin))
        {
            tab.grant = None;
        }
        tab.saved.url = url.clone();
        tab.saved.title = if url == START {
            "New tab".into()
        } else {
            model::display_host(&url)
        };
        if url == START {
            tab.page = None;
            tab.loading = false;
        } else if let Some(page) = &tab.page {
            if let Err(error) = page.raw.load_url(&url) {
                tab.error = Some(error.to_string());
            }
        } else {
            self.ensure_page(self.active, window, cx);
        }
        self.sync_address(window, cx);
        window.focus(&self.focus, cx);
        if let Some(page) = &self.tabs[self.active].page {
            let _ = page.raw.focus();
        }
        self.dirty = true;
        cx.notify();
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.request_discard(lifecycle::Discard::Close(self.tabs[index].id), window, cx);
    }

    fn remember_closed(&mut self, saved: SavedTab) {
        self.closed.push(saved);
        if self.closed.len() > 20 {
            self.closed.remove(0);
        }
    }

    /// Removes `ids` in one step so the survivors aren't activated (and their
    /// pages loaded) one by one. The menu's own tab takes over if needed.
    fn close_tabs_unchecked(
        &mut self,
        keep: u64,
        ids: &[u64],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.tabs[self.active].id;
        let active = if ids.contains(&active) { keep } else { active };
        let (closing, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tabs)
            .into_iter()
            .partition(|tab| ids.contains(&tab.id));
        self.tabs = kept;
        // Reopen restores the topmost (newest) of them first.
        for tab in closing.into_iter().rev().filter(|tab| !tab.private) {
            self.remember_closed(tab.saved);
        }
        match self.tabs.iter().position(|tab| tab.id == active) {
            Some(index) => self.activate(index, window, cx),
            None if self.tabs.is_empty() => self.new_tab(START.into(), false, window, cx),
            None => self.activate(0, window, cx),
        }
        self.save();
    }

    fn close_unchecked(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.tabs.remove(index);
        if !tab.private {
            self.remember_closed(tab.saved.clone());
        }
        drop(tab);
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

    fn address_focused(&self, window: &Window, cx: &App) -> bool {
        self.address.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Abandons an address edit and hands focus back to the page.
    fn release_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_address(window, cx);
        window.focus(&self.focus, cx);
        if !self.lab
            && let Some(page) = &self.tabs[self.active].page
        {
            let _ = page.raw.focus();
        }
        cx.notify();
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_address(window, cx);
        if let Some(page) = &self.tabs[self.active].page {
            let _ = page.raw.focus_parent();
        }
        self.address.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    fn tab_menu(&self, id: u64, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        use TabCommand::*;
        use gpui::component::native_menu::NativeMenu;
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = &self.tabs[index];
        let item = |command| Box::new(TabMenu { id, command }) as Box<dyn Action>;
        let live = tab.page.is_some();
        let addressed = tab.saved.url != START;
        let mut menu = NativeMenu::new();
        if live {
            menu = menu.menu("Reload", item(Reload));
        }
        menu = menu
            .menu_with_disabled("Copy Address", !addressed, item(CopyAddress))
            .menu("Duplicate Tab", item(Duplicate))
            .separator();
        menu = if live {
            menu.menu("Suspend Tab", item(Suspend))
        } else {
            menu.menu_with_disabled("Resume Tab", !addressed, item(Resume))
        };
        if !tab.private {
            menu = match tab.grant {
                Some(_) => menu.menu("Revoke Agent Access", item(ToggleAgent)),
                None => menu.menu_with_disabled("Share with Agent", !live, item(ToggleAgent)),
            };
        }
        menu.separator()
            .menu("Close Tab", item(Close))
            .menu_with_disabled("Close Other Tabs", self.tabs.len() < 2, item(CloseOthers))
            .menu_with_disabled(
                "Close Tabs Below",
                index + 1 >= self.tabs.len(),
                item(CloseBelow),
            )
            .show(position, window, cx);
    }

    fn tab_command(&mut self, action: &TabMenu, window: &mut Window, cx: &mut Context<Self>) {
        use TabCommand::*;
        // The menu was built for a tab that may have moved or closed since.
        let Some(index) = self.tabs.iter().position(|tab| tab.id == action.id) else {
            return;
        };
        let id = action.id;
        match action.command {
            Reload => {
                if let Some(page) = &self.tabs[index].page {
                    let _ = page.raw.reload();
                }
            }
            CopyAddress => {
                let url = self.tab_url(index);
                if url != START {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    if index == self.active && !self.lab {
                        self.flash_copy(CopyTarget::Address, cx);
                    }
                }
            }
            Duplicate => {
                let (url, private) = (self.tab_url(index), self.tabs[index].private);
                // Directly above the original, which is where "newer" lives.
                self.new_tab_at(index, url, private, window, cx);
            }
            Suspend => self.request_discard(lifecycle::Discard::Suspend(id), window, cx),
            Resume => self.ensure_page(index, window, cx),
            ToggleAgent => self.toggle_agent_for(index, window, cx),
            Close => self.close(index, window, cx),
            CloseOthers | CloseBelow => {
                let ids: Vec<u64> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| match action.command {
                        CloseBelow => *i > index,
                        _ => *i != index,
                    })
                    .map(|(_, tab)| tab.id)
                    .collect();
                if !ids.is_empty() {
                    self.request_discard(
                        lifecycle::Discard::CloseTabs { keep: id, ids },
                        window,
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    /// The live address when the page has one, else the saved address.
    fn tab_url(&self, index: usize) -> String {
        let tab = &self.tabs[index];
        tab.page
            .as_ref()
            .and_then(|page| page.url().ok())
            .filter(|url| model::allowed_navigation(url))
            .unwrap_or_else(|| tab.saved.url.clone())
    }

    fn copy_url(&mut self, cx: &mut Context<Self>) {
        let url = self.tab_url(self.active);
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.flash_copy(CopyTarget::Address, cx);
    }

    fn flash_copy(&mut self, target: CopyTarget, cx: &mut Context<Self>) {
        self.copy_feedback = Some(target);
        self.copy_epoch = self.copy_epoch.wrapping_add(1);
        let epoch = self.copy_epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                // Repeated copies get a full feedback interval of their own.
                if this.copy_epoch == epoch {
                    this.copy_feedback = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn save(&mut self) -> bool {
        let active_id = self.tabs[self.active].id;
        let normal: Vec<_> = self.tabs.iter().filter(|tab| !tab.private).collect();
        let active = normal
            .iter()
            .position(|tab| tab.id == active_id)
            .unwrap_or(0);
        let snapshot = Session {
            version: 1,
            tabs: normal.iter().map(|tab| tab.saved.clone()).collect(),
            active,
        };
        if let Err(error) = self.store.save(&snapshot) {
            self.notice = Some(format!("Session not saved: {error}"));
            return false;
        }
        self.dirty = false;
        true
    }

    fn event(&mut self, event: engine::Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            engine::Event::QuitRequested => {
                self.request_discard(lifecycle::Discard::Quit, window, cx)
            }
            engine::Event::NavigationDecision { id, reply } => {
                if self.discard_pending
                    || !self
                        .tabs
                        .iter()
                        .any(|tab| tab.id == id && tab.page.is_some())
                {
                    let _ = reply.try_send(false);
                    return;
                }
                self.discard_pending = true;
                let host = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id == id)
                    .map(|tab| model::display_host(&tab.saved.url))
                    .unwrap_or_default();
                let detail = format!(
                    "{host} may have unsaved edits, active media, or content Browser could not check. Leaving may discard that work."
                );
                let answer = window.prompt(
                    PromptLevel::Warning,
                    "Leave this page?",
                    Some(&detail),
                    &["Cancel", "Leave page"],
                    cx,
                );
                cx.spawn_in(window, async move |this, cx| {
                    let confirmed = answer.await == Ok(1);
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.discard_pending = false;
                        this.sync_address(window, cx);
                        let _ = reply.try_send(confirmed);
                        cx.notify();
                    });
                })
                .detach();
            }
            engine::Event::Title(id, title) => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.saved.title = title;
                    self.dirty = true;
                }
            }
            engine::Event::Loading(id, loading, url) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    if loading {
                        self.tabs[index].revision = self.tabs[index].revision.wrapping_add(1);
                    }
                    self.tabs[index].loading = loading;
                    if self.tabs[index]
                        .grant
                        .as_ref()
                        .is_some_and(|(origin, _)| mcp::origin(&url).as_ref() != Some(origin))
                    {
                        self.tabs[index].grant = None;
                    }
                    if model::allowed_navigation(&url) && url != "about:blank" {
                        self.tabs[index].saved.url = url;
                        self.dirty = true;
                    }
                    if !loading {
                        self.request_icon(index);
                    }
                    if index == self.active
                        && !self.address.read(cx).focus_handle(cx).is_focused(window)
                    {
                        self.sync_address(window, cx);
                    }
                }
            }
            engine::Event::NewTab(id, url) => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) {
                    self.new_tab(url, tab.private, window, cx);
                }
            }
            engine::Event::Notice(message) => self.notice = Some(message),
            engine::Event::Icon(key, bytes) => {
                match bytes.and_then(|bytes| Some((favicon::sniff(&bytes)?, bytes))) {
                    Some((format, bytes)) => {
                        self.icon_requests.remove(&key);
                        self.icons
                            .insert(key, (Arc::new(Image::from_bytes(format, bytes)), true));
                    }
                    // Leave the host marked so a failing site isn't asked again.
                    None => {
                        self.icon_requests.insert(key);
                    }
                }
            }
            engine::Event::NativeClick(point) => {
                let on_page = !self.lab
                    && self.tabs[self.active]
                        .page
                        .as_ref()
                        .is_some_and(|page| page.contains(point));
                if on_page && self.address_focused(window, cx) {
                    self.release_address(window, cx);
                }
            }
            engine::Event::AgentResult {
                id,
                epoch,
                origin,
                reply,
                value,
            } => self.finish_agent(id, epoch, origin, reply, value),
        }
        cx.notify();
    }

    /// Uses the disk copy when fresh; otherwise asks the page which icon it
    /// declares and fetches that off the UI thread. Never for private tabs.
    fn request_icon(&mut self, index: usize) {
        let tab = &self.tabs[index];
        let Some(key) = (!tab.private)
            .then(|| favicon::host_key(&tab.saved.url))
            .flatten()
        else {
            return;
        };
        if self.icons.get(&key).is_some_and(|(_, fresh)| *fresh)
            || self.icon_requests.contains(&key)
        {
            return;
        }
        if !self.icons.contains_key(&key)
            && let Some((bytes, fresh)) = self.icon_cache.read(&key)
            && let Some(format) = favicon::sniff(&bytes)
        {
            self.icons.insert(
                key.clone(),
                (Arc::new(Image::from_bytes(format, bytes)), fresh),
            );
            if fresh {
                return;
            }
        }
        let Some(page) = &tab.page else {
            return;
        };
        self.icon_requests.insert(key.clone());
        let page_url = tab.saved.url.clone();
        let events = self.events.clone();
        let cache = self.icon_cache.clone();
        let _ = page
            .raw
            .evaluate_script_with_callback(favicon::DISCOVER, move |json| {
                let found: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
                let text = |field: &str| found.get(field).and_then(|value| value.as_str());
                // The tab may have moved on before the script ran.
                let same_host = text("page").and_then(favicon::host_key).as_ref() == Some(&key);
                let icon = same_host
                    .then(|| favicon::icon_url(&page_url, text("href")))
                    .flatten();
                let agent = text("agent").unwrap_or_default().to_string();
                let (key, cache, events) = (key.clone(), cache.clone(), events.clone());
                std::thread::spawn(move || {
                    let bytes = icon.and_then(|icon| favicon::fetch(&icon, &agent).ok());
                    if let Some(bytes) = &bytes {
                        let _ = cache.write(&key, bytes);
                    }
                    let _ = events.try_send(engine::Event::Icon(key, bytes));
                });
            });
    }

    fn history(&self, forward: bool) {
        if let Some(page) = &self.tabs[self.active].page {
            let _ = if forward {
                page.raw.go_forward()
            } else {
                page.raw.go_back()
            };
        }
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = &self.tabs[self.active].page {
            let _ = page.raw.reload();
        } else {
            self.ensure_page(self.active, window, cx);
        }
        cx.notify();
    }
    fn show_lab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if let Some(page) = &tab.page {
                page.hide();
                let _ = page.raw.focus_parent();
            }
        }
        self.lab = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn suspend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_discard(
            lifecycle::Discard::Suspend(self.tabs[self.active].id),
            window,
            cx,
        );
    }

    fn live_pages(&self, action: &lifecycle::Discard) -> Vec<lifecycle::LivePage> {
        self.tabs
            .iter()
            .filter(|tab| action.includes(tab.id) && tab.page.is_some())
            .map(|tab| lifecycle::LivePage {
                id: tab.id,
                revision: tab.revision,
            })
            .collect()
    }

    async fn check_work(
        pages: &[(u64, Rc<engine::Page>)],
        background: BackgroundExecutor,
    ) -> Vec<(u64, lifecycle::WorkState)> {
        futures_lite::future::race(
            async {
                let mut states = Vec::new();
                for (id, page) in pages {
                    states.push((*id, page.work_state(background.clone()).await));
                }
                states
            },
            async {
                background.timer(Duration::from_secs(2)).await;
                pages
                    .iter()
                    .map(|(id, _)| (*id, lifecycle::WorkState::unknown()))
                    .collect()
            },
        )
        .await
    }

    fn request_discard(
        &mut self,
        action: lifecycle::Discard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keyboard repeat, window close, and Dock Quit all share one decision.
        if self.discard_pending {
            return;
        }
        let expected = self.live_pages(&action);
        let pages: Vec<_> = self
            .tabs
            .iter()
            .filter(|tab| action.includes(tab.id))
            .filter_map(|tab| tab.page.as_ref().map(|page| (tab.id, page.clone())))
            .collect();
        self.discard_pending = true;
        let background = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let states = Self::check_work(&pages, background.clone()).await;
            let answer = this.update_in(cx, |this, window, cx| {
                if !lifecycle::consent_matches(&expected, &this.live_pages(&action)) {
                    this.discard_pending = false;
                    this.notice = Some("The open pages changed during the check. Nothing was discarded; try again.".into());
                    cx.notify();
                    return None;
                }
                let lab_draft = action == lifecycle::Discard::Quit
                    && (this.requesting || !this.request_url.read(cx).value().is_empty()
                        || !this.request_headers.read(cx).value().is_empty()
                        || !this.request_body.read(cx).value().is_empty());
                if states.iter().all(|(_, state)| state.clear()) && !lab_draft {
                    this.discard_pending = false;
                    this.discard(action.clone(), window, cx);
                    return None;
                }
                let (title, confirm, detail) = action.labels();
                Some(window.prompt(PromptLevel::Warning, title, Some(detail), &["Cancel", confirm], cx))
            }).ok().flatten();
            let Some(answer) = answer else { return; };
            let confirmed = answer.await == Ok(1);
            let covered = if confirmed {
                let current = Self::check_work(&pages, background).await;
                states.iter().zip(&current).all(|((id, before), (current_id, after))| id == current_id && before.still_covered(after))
            } else { true };
            let _ = this.update_in(cx, |this, window, cx| {
                this.discard_pending = false;
                if confirmed {
                    if covered && lifecycle::consent_matches(&expected, &this.live_pages(&action)) {
                        this.discard(action, window, cx);
                    } else {
                        this.notice = Some("The open pages changed while the dialog was open. Nothing was discarded; try again.".into());
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn discard(&mut self, action: lifecycle::Discard, window: &mut Window, cx: &mut Context<Self>) {
        use lifecycle::Discard;
        if action == Discard::Quit {
            if !self.save() {
                let expected = self.live_pages(&action);
                self.discard_pending = true;
                let detail = format!(
                    "{}\n\nQuit anyway? The current tab addresses will not be saved, and live page state and Request Lab drafts will be lost.",
                    self.notice
                        .as_deref()
                        .unwrap_or("Session could not be saved.")
                );
                let answer = window.prompt(
                    PromptLevel::Warning,
                    "Quit without saving the session?",
                    Some(&detail),
                    &["Cancel", "Quit without saving"],
                    cx,
                );
                cx.spawn_in(window, async move |this, cx| {
                    let confirmed = answer.await == Ok(1);
                    let _ = this.update_in(cx, |this, _, cx| {
                        this.discard_pending = false;
                        if confirmed
                            && lifecycle::consent_matches(&expected, &this.live_pages(&action))
                        {
                            this.finish_quit(cx);
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
                return;
            }
            self.finish_quit(cx);
            return;
        }
        if let Discard::CloseTabs { keep, ids } = &action {
            self.close_tabs_unchecked(*keep, ids, window, cx);
            return;
        }
        let Some(index) = self.tabs.iter().position(|tab| action.includes(tab.id)) else {
            return;
        };
        match action {
            Discard::Close(_) => self.close_unchecked(index, window, cx),
            Discard::Suspend(_) => {
                let tab = &mut self.tabs[index];
                tab.page = None;
                tab.grant = None;
                tab.loading = false;
                tab.revision = tab.revision.wrapping_add(1);
                cx.notify();
            }
            Discard::Start(_) => {
                self.activate(index, window, cx);
                self.navigate_unchecked(START.into(), window, cx);
            }
            Discard::Quit | Discard::CloseTabs { .. } => unreachable!(),
        }
    }

    fn finish_quit(&self, cx: &App) {
        #[cfg(target_os = "macos")]
        if let Some(guard) = &self.termination {
            guard.allow();
        }
        cx.quit();
    }
    fn request(&self, cx: &App) -> http_client::Request {
        http_client::Request {
            method: self.request_method.clone(),
            url: self.request_url.read(cx).value().to_string(),
            headers: self.request_headers.read(cx).value().to_string(),
            body: self.request_body.read(cx).value().to_string(),
        }
    }
    fn send_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.requesting {
            return;
        }
        let request = self.request(cx);
        self.requesting = true;
        self.response_summary = "Sending…".into();
        let (send, receive) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = send
                .send_blocking(http_client::execute(request).map_err(|error| error.to_string()));
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(result) = receive.recv().await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.requesting = false;
                    let text = match result {
                        Ok(response) => {
                            this.response_summary = format!(
                                "{}  ·  {} ms  ·  {} bytes",
                                response.status, response.elapsed_ms, response.bytes
                            );
                            response.text
                        }
                        Err(error) => {
                            this.response_summary = "Request failed".into();
                            error
                        }
                    };
                    this.response
                        .update(cx, |response, cx| response.set_value(text, window, cx));
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
}

fn profile_id(dir: &std::path::Path) -> anyhow::Result<[u8; 16]> {
    let path = dir.join("profile-id");
    if path.exists() {
        return Ok(*uuid::Uuid::parse_str(std::fs::read_to_string(path)?.trim())?.as_bytes());
    }
    let uuid = uuid::Uuid::new_v4();
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    write!(file, "{uuid}")?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(*uuid.as_bytes())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let dir = session::data_dir();
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return mcp::bridge(&dir);
    }
    let mut store = session::Store::open(dir.clone())?;
    let (agent_send, agent_receive) = async_channel::bounded(32);
    mcp::start(&dir, agent_send)?;
    let (saved, notice) = match store.load() {
        Ok(saved) => (saved, None),
        Err(error) => (
            Session::default(),
            Some(format!(
                "Recovery unavailable; original session preserved: {error}"
            )),
        ),
    };
    let profile = profile_id(&dir)?;
    let initial = std::env::args()
        .nth(1)
        .map(|url| model::destination(&url))
        .transpose()?;
    #[cfg(target_os = "linux")]
    let app = gpui::platform::linux(WindowingModes::X11);
    #[cfg(not(target_os = "linux"))]
    let app = gpui::application();
    let (urls_send, urls_receive) = async_channel::bounded(32);
    app.on_open_urls(move |urls| {
        let _ = urls_send.try_send(urls);
    });
    app.with_assets(assets::Assets).run(move |cx| {
        gpui::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.bind_keys([
            KeyBinding::new(&format!("{modifier}-t"), NewTab, None),
            KeyBinding::new(&format!("{modifier}-shift-n"), NewPrivateTab, None),
            KeyBinding::new(&format!("{modifier}-w"), CloseTab, None),
            KeyBinding::new(&format!("{modifier}-shift-t"), ReopenTab, None),
            KeyBinding::new(&format!("{modifier}-l"), FocusAddress, None),
            KeyBinding::new(&format!("{modifier}-shift-c"), CopyUrl, None),
            KeyBinding::new(&format!("{modifier}-["), Back, None),
            KeyBinding::new(&format!("{modifier}-]"), Forward, None),
            KeyBinding::new(&format!("{modifier}-r"), Reload, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
            KeyBinding::new(&format!("{modifier}-shift-j"), RequestLab, None),
            KeyBinding::new(&format!("{modifier}-alt-i"), Inspect, None),
            KeyBinding::new(&format!("{modifier}-q"), Quit, None),
        ]);
        cx.set_menus([
            Menu::new("Browser").items([MenuItem::action("Quit Browser", Quit)]),
            Menu::new("File").items([
                MenuItem::action("New Tab", NewTab),
                MenuItem::action("New Private Tab", NewPrivateTab),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::action("Reopen Closed Tab", ReopenTab),
            ]),
            Menu::new("Edit").items([
                MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            ]),
            Menu::new("Navigate").items([
                MenuItem::action("Open Location", FocusAddress),
                MenuItem::action("Copy Address", CopyUrl),
                MenuItem::action("Back", Back),
                MenuItem::action("Forward", Forward),
                MenuItem::action("Reload", Reload),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
            ]),
            Menu::new("Tools").items([
                MenuItem::action("Request Lab", RequestLab),
                MenuItem::action("Inspect Page", Inspect),
                MenuItem::action("Suspend Current Tab…", Suspend),
                MenuItem::action("Share / Revoke Agent Access…", ToggleAgent),
            ]),
        ]);
        let (handle, view) = gpui::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1240.), px(820.)),
                    cx,
                ))),
                window_min_size: Some(size(px(880.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Browser".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(16.))),
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut browser = Browser::new(store, saved, notice, profile, window, cx);
                    if let Some(url) = initial {
                        browser.navigate(url, window, cx);
                    }
                    browser
                })
            },
        )
        .expect("Could not open Browser window");
        let agent_view = view.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(command) = agent_receive.recv().await {
                if handle
                    .update(cx, |_, _, cx| {
                        agent_view.update(cx, |this, cx| this.agent_command(command, cx))
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let urls_view = view.downgrade();
        cx.spawn(async move |cx| {
            while let Ok(urls) = urls_receive.recv().await {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = urls_view.update(cx, |this, cx| {
                        for url in urls {
                            if mcp::origin(&url).is_some() {
                                this.new_tab(url, false, window, cx);
                            }
                        }
                    });
                });
            }
        })
        .detach();
        let weak = view.downgrade();
        let _ = handle.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.request_discard(lifecycle::Discard::Quit, window, cx)
                });
                false
            });
        });
        #[cfg(target_os = "macos")]
        view.update(cx, |this, _| {
            this.termination = Some(engine::TerminationGuard::install(this.events.clone()));
        });
        cx.activate(true);
    });
    Ok(())
}
