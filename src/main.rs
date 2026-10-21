extern crate gpui_kit as gpui;
mod agent;
mod assets;
mod engine;
mod http_client;
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
use std::{rc::Rc, time::Duration};

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
    page: Option<Rc<engine::Page>>,
    error: Option<String>,
    grant: Option<(String, u64)>,
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
                page: None,
                error: None,
                grant: None,
            });
        }
        let active = saved.active.min(tabs.len() - 1);
        let next_id = tabs.len() as u64 + 1;
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
            _subscriptions: vec![
                address_events,
                cx.observe_window_appearance(window, |this, window, cx| {
                    this.follow_appearance(window, cx)
                }),
            ],
        };
        this.follow_appearance(window, cx);
        this.activate(active, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if let Some(page) = &this.tabs[this.active].page
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
        if self.tabs.len() >= 1000 {
            self.notice = Some("Tab limit reached.".into());
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
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
            page: None,
            error: None,
            grant: None,
        });
        self.activate(self.tabs.len() - 1, window, cx);
        if self.tabs[self.active].saved.url == START {
            self.focus_address(window, cx);
        }
    }

    fn navigate(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.lab = false;
        self.notice = None;
        let tab = &mut self.tabs[self.active];
        tab.error = None;
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
        let tab = self.tabs.remove(index);
        if !tab.private {
            self.closed.push(tab.saved.clone());
            if self.closed.len() > 20 {
                self.closed.remove(0);
            }
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
        if let Some(page) = &self.tabs[self.active].page {
            let _ = page.raw.focus_parent();
        }
        self.address.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    fn copy_url(&mut self, cx: &mut Context<Self>) {
        let tab = &self.tabs[self.active];
        let url = tab
            .page
            .as_ref()
            .and_then(|page| page.url().ok())
            .filter(|url| model::allowed_navigation(url))
            .unwrap_or_else(|| tab.saved.url.clone());
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

    fn save(&mut self) {
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
        }
        self.dirty = false;
    }

    fn event(&mut self, event: engine::Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            engine::Event::Title(id, title) => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.saved.title = title;
                    self.dirty = true;
                }
            }
            engine::Event::Loading(id, loading, url) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
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

    fn history(&self, forward: bool) {
        if let Some(page) = &self.tabs[self.active].page {
            let _ = page.raw.evaluate_script(if forward {
                "history.forward()"
            } else {
                "history.back()"
            });
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
        if self.tabs[self.active].page.is_none() {
            return;
        }
        let id = self.tabs[self.active].id;
        let answer=window.prompt(PromptLevel::Warning,"Suspend this tab?",Some("The page will reload when you resume it. Unsaved forms, calls, and other live page state will be lost."),&["Cancel","Suspend"],cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |this, _, cx| {
                    if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == id) {
                        tab.page = None;
                        tab.grant = None;
                        tab.loading = false;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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
            window.on_window_should_close(cx, move |_, cx| {
                let _ = weak.update(cx, |this, _| this.save());
                cx.quit();
                true
            });
        });
        cx.activate(true);
    });
    Ok(())
}
