//! App windows: a page popped out of the browser window into a window of its
//! own, with no sidebar or toolbar. A slim strip in the page's color sits
//! above it; its handle opens Back, Forward and the way back into the
//! browser window. The page moves between windows live, never reloading.
//!
//! An app window's page stays the browser's tab in every other way: it uses
//! its Space's data store (so its sign-ins), permissions, history, agents,
//! downloads and unsaved-work checks. Pages it opens in new tabs go to the
//! browser window, in its Space. App windows reopen with the session.
use crate::assets::POP_IN;
use crate::menu_bar;
use crate::*;
use gpui::assets::IconName;
use gpui::component::Icon;

/// The strip above the page: tall enough for the traffic lights.
const STRIP: f32 = 28.;
/// How long an app window takes to glide out of the card or back onto it.
const GLIDE: f64 = 0.32;

/// The browser window's card on screen, with room above it for an app
/// window's strip: where an app window sits so its page lines up with the
/// card's.
fn card_frame(window: &Window) -> [f32; 4] {
    let main = window.bounds();
    let top = ui::CONTROL_ZONE - STRIP;
    [
        f32::from(main.origin.x) + ui::sidebar_width(),
        f32::from(main.origin.y) + top,
        f32::from(main.size.width) - ui::sidebar_width(),
        f32::from(main.size.height) - top,
    ]
}

fn bounds_of([x, y, width, height]: [f32; 4]) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

/// A page in its own window.
pub(crate) struct AppPage {
    pub tab: Tab,
    /// The Space it belongs to, whose data store it uses.
    pub space: [u8; 16],
    /// `None` for the moment between popping out and the window opening.
    pub window: Option<WindowHandle<AppWindow>>,
    /// Where the window sits on screen, for the session.
    pub frame: Option<[f32; 4]>,
    /// Gliding back onto the card, about to become a tab again.
    pub returning: bool,
    /// What its window was last sent.
    look: Option<AppLook>,
    /// Its place in the tab list before it popped out.
    slot: Option<Slot>,
}

/// A tab's place in its Space's list: below the tab that was above it, or
/// failing that at its old index.
#[derive(Clone, Copy)]
struct Slot {
    above: Option<u64>,
    index: usize,
}

/// The app window's view: the strip, and the page placed below it.
pub(crate) struct AppWindow {
    browser: WeakEntity<Browser>,
    main: AnyWindowHandle,
    id: u64,
    focus: FocusHandle,
    /// The strip's controls are showing.
    open: bool,
    /// What it shows, sent by the browser (`sync_apps`).
    look: Option<AppLook>,
}

/// What an app window draws, sent by the browser whenever it changes. The
/// window never reads the browser while drawing: GPUI ties an entity to the
/// last window that drew from it, and the browser's own tasks then ran in
/// the app window (a page returning from one went back into it and was lost
/// with it).
#[derive(Clone)]
pub(crate) struct AppLook {
    title: String,
    tint: Option<u32>,
    dark: bool,
    /// Placed only once it has moved into this window.
    page: Option<Rc<engine::Page>>,
    error: Option<String>,
}

impl PartialEq for AppLook {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title
            && self.tint == other.tint
            && self.dark == other.dark
            && self.error == other.error
            && match (&self.page, &other.page) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
    }
}

impl Browser {
    pub(crate) fn app(&self, id: u64) -> Option<&AppPage> {
        self.apps.iter().find(|app| app.tab.id == id)
    }

    /// Tab ▸ Open in App Window, or the tab's pop-out button: the live page
    /// moves into a window of its own.
    pub(crate) fn pop_out(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if tab.private {
            self.notice = Some("Private tabs stay in the browser window.".into());
            cx.notify();
            return;
        }
        if tab.saved.url == START {
            return;
        }
        if self.finding == Some(tab.id) {
            self.end_find();
        }
        // The window opens right over the card, its strip just above, so
        // the page doesn't move; then it glides out to a spot of its own,
        // a little smaller.
        let card = card_frame(window);
        let [x, y, width, height] = card;
        let (smaller_width, smaller_height) = ((width * 0.88).max(480.), (height * 0.88).max(360.));
        let away = [
            x + (width - smaller_width) / 2. + 24.,
            y + (height - smaller_height) / 2. + 24.,
            smaller_width,
            smaller_height,
        ];
        let (frame, glide) = if cx.reduce_motion() {
            (away, None)
        } else {
            (card, Some(away))
        };
        // Where it came from, so returning puts it back there.
        let slot = Slot {
            above: index.checked_sub(1).map(|above| self.tabs[above].id),
            index,
        };
        let tab = self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.new_tab(START.into(), false, window, cx);
        } else if index == self.active {
            self.activate(index.min(self.tabs.len() - 1), window, cx);
        } else if index < self.active {
            self.active -= 1;
        }
        let space = self.current_space().id;
        self.open_app_window(tab, space, Some(frame), glide, window, cx);
        if let Some(app) = self.apps.last_mut() {
            app.slot = Some(slot);
        }
        self.save();
        cx.notify();
    }

    /// Opens `tab` in an app window, moving its page there or starting it
    /// there. `window` is the browser window.
    fn open_app_window(
        &mut self,
        tab: Tab,
        space: [u8; 16],
        frame: Option<[f32; 4]>,
        glide: Option<[f32; 4]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = tab.id;
        let title = tab.saved.title.clone();
        self.apps.push(AppPage {
            tab,
            space,
            window: None,
            frame,
            returning: false,
            look: None,
            slot: None,
        });
        let browser = cx.entity();
        let main = window.window_handle();
        // GPUI draws a new window at once and its view reads the browser, so
        // the window opens after this update.
        cx.defer(move |cx| {
            let bounds = match frame {
                Some(frame) => bounds_of(frame),
                None => Bounds::centered(None, size(px(1000.), px(760.)), cx),
            };
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(360.), px(240.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(10.), px(8.))),
                }),
                app_owns_titlebar_drag: true,
                ..Default::default()
            };
            let opened = cx.open_window(options, |window, cx| {
                cx.new(|cx| AppWindow::new(&browser, main, id, window, cx))
            });
            let _ = main.update(cx, |_, window, cx| {
                browser.update(cx, |this, cx| match opened {
                    Ok(handle) => this.attach_app(id, handle, glide, cx),
                    Err(error) => {
                        this.notice = Some(format!("Couldn’t open an app window: {error}"));
                        // The page goes back to the browser window.
                        if let Some(index) = this.apps.iter().position(|app| app.tab.id == id) {
                            let app = this.apps.remove(index);
                            this.restore_tab(app.tab, app.space, None, window, cx);
                        }
                        cx.notify();
                    }
                })
            });
        });
    }

    /// The app window opened: its page moves in, or starts there.
    fn attach_app(
        &mut self,
        id: u64,
        handle: WindowHandle<AppWindow>,
        glide: Option<[f32; 4]>,
        cx: &mut Context<Self>,
    ) {
        let events = self.events.clone();
        let (context, ids, zooms, dark) = (&mut self.context, &self.ids, &self.zooms, self.dark);
        let Some(app) = self.apps.iter_mut().find(|app| app.tab.id == id) else {
            remove_window(handle, cx);
            return;
        };
        app.window = Some(handle);
        let (tab, space) = (&mut app.tab, app.space);
        let _ = handle.update(cx, |_, app_window, cx| {
            #[cfg(target_os = "macos")]
            engine::install_backdrop(app_window);
            engine::without_window_animations(app_window);
            if let Some(to) = glide {
                engine::animate_frame(app_window, bounds_of(to), GLIDE);
            }
            if let Some(page) = &tab.page {
                page.adopt(app_window, false);
                return;
            }
            // A suspended or restored tab starts in its window.
            match engine::Page::create(
                engine::PageOptions {
                    id: tab.id,
                    url: &tab.saved.url,
                    private: false,
                    profile: space,
                    ids,
                },
                context,
                events,
                app_window,
                cx,
            ) {
                Ok(page) => {
                    page.adopt(app_window, false);
                    page.set_backdrop(ui::page_color(tab.tint, dark));
                    tab.zoom =
                        zoom::site(&tab.saved.url).map_or(1., |site| zooms.get(&site, false));
                    if !zoom::is_actual_size(tab.zoom) {
                        page.set_zoom(tab.zoom);
                    }
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
        });
        cx.notify();
    }

    /// Puts a tab back in its Space's list in the browser window.
    fn restore_tab(
        &mut self,
        tab: Tab,
        space: [u8; 16],
        slot: Option<Slot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .spaces
            .iter()
            .position(|candidate| candidate.id == space)
        {
            self.switch_space(index, window, cx);
        }
        let position = slot.map_or(0, |slot| match slot.above {
            Some(above) => self
                .tabs
                .iter()
                .position(|tab| tab.id == above)
                .map_or(slot.index, |above| above + 1),
            None => 0,
        });
        let position = position.min(self.tabs.len());
        self.tabs.insert(position, tab);
        if let Some(page) = &self.tabs[position].page {
            page.adopt(window, true);
        }
        self.activate(position, window, cx);
    }

    /// The strip's return button, or Tab ▸ Move to Browser Window: the
    /// window glides onto the browser's card, then its page becomes the
    /// front tab there.
    pub(crate) fn return_app(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.apps.iter_mut().find(|app| app.tab.id == id) else {
            return;
        };
        let Some(handle) = app.window.filter(|_| !app.returning) else {
            return;
        };
        if cx.reduce_motion() {
            self.pop_in(id, window, cx);
            return;
        }
        app.returning = true;
        let to = bounds_of(card_frame(window));
        // The return button is in that window, which is busy with the click.
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, app_window, _| {
                engine::animate_frame(app_window, to, GLIDE)
            });
        });
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(GLIDE))
                .await;
            let _ = this.update_in(cx, |this, window, cx| this.pop_in(id, window, cx));
        })
        .detach();
    }

    /// Moves the page back into the browser window at once.
    pub(crate) fn pop_in(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.apps.iter().position(|app| app.tab.id == id) else {
            return;
        };
        let app = self.apps.remove(index);
        self.restore_tab(app.tab, app.space, app.slot, window, cx);
        remove_window(app.window, cx);
        window.activate_window();
        self.save();
        cx.notify();
    }

    /// Closes an app window after its page's work check (or with none, when
    /// the page closed itself).
    pub(crate) fn close_app_unchecked(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.apps.iter().position(|app| app.tab.id == id) else {
            return;
        };
        let app = self.apps.remove(index);
        let saved = app.tab.saved.clone();
        drop(app.tab);
        remove_window(app.window, cx);
        // ⇧⌘T brings it back, as a tab.
        if saved.url != START {
            self.closed.push(saved);
        }
        self.save();
        cx.notify();
    }

    /// Closes every app window of a Space being deleted.
    pub(crate) fn close_space_apps(&mut self, space: [u8; 16], cx: &mut Context<Self>) {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.apps)
            .into_iter()
            .partition(|app| app.space == space);
        self.apps = kept;
        for app in gone {
            drop(app.tab);
            remove_window(app.window, cx);
        }
    }

    /// Reopens the app windows saved with the session. `window` is the
    /// browser window.
    pub(crate) fn reopen_apps(
        &mut self,
        saved: Vec<([u8; 16], model::SavedApp)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (space, app) in saved {
            if !self
                .spaces
                .iter()
                .any(|candidate| candidate.id == space && !candidate.private)
            {
                continue;
            }
            let id = engine::next_id(&self.ids);
            let mut tab = Tab::new(id, app.tab.url, false);
            tab.saved.title = app.tab.title;
            self.open_app_window(tab, space, app.frame, None, window, cx);
        }
    }

    /// Brings an app window forward (a notification from its page).
    pub(crate) fn show_app(&self, id: u64, cx: &mut App) {
        if let Some(handle) = self.app(id).and_then(|app| app.window) {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
    }

    /// For a page an app window opens in a new tab: the browser window shows
    /// the app's Space and comes forward.
    pub(crate) fn show_app_space(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.app(id).map(|app| app.space) else {
            return;
        };
        if let Some(index) = self
            .spaces
            .iter()
            .position(|candidate| candidate.id == space)
        {
            self.switch_space(index, window, cx);
        }
        window.activate_window();
    }

    /// Asks on the window that shows tab `id`: its app window, or else the
    /// browser window (`window`). Resolves to the button chosen, if any.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prompt_in<T: Clone + Into<PromptButton>>(
        &self,
        id: Option<u64>,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[T],
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Option<usize>> {
        let app = id.and_then(|id| self.app(id)).and_then(|app| app.window);
        let answer = app
            .and_then(|handle| {
                handle
                    .update(cx, |_, window, cx| {
                        window.activate_window();
                        window.prompt(level, message, detail, answers, cx)
                    })
                    .ok()
            })
            .unwrap_or_else(|| window.prompt(level, message, detail, answers, cx));
        cx.foreground_executor()
            .spawn(async move { answer.await.ok() })
    }

    /// Called on every render: sends each app window what it shows, when
    /// that changed.
    pub(crate) fn sync_apps(&mut self, cx: &mut Context<Self>) {
        let dark = self.dark;
        for app in &mut self.apps {
            let Some(handle) = app.window else {
                continue;
            };
            let tab = &app.tab;
            let look = AppLook {
                title: if tab.saved.title.trim().is_empty() {
                    model::display_host(&tab.saved.url)
                } else {
                    tab.saved.title.clone()
                },
                tint: tab.tint,
                dark,
                page: tab.page.clone(),
                error: tab.error.clone(),
            };
            if app.look.as_ref() == Some(&look) {
                continue;
            }
            app.look = Some(look.clone());
            cx.defer(move |cx| {
                let _ = handle.update(cx, |view, _, cx| {
                    view.look = Some(look);
                    cx.notify();
                });
            });
        }
    }

    pub(crate) fn app_page(&self, id: u64) -> Option<Rc<engine::Page>> {
        self.app(id).and_then(|app| app.tab.page.clone())
    }

    fn app_frame(&mut self, id: u64, frame: [f32; 4]) {
        if let Some(app) = self.apps.iter_mut().find(|app| app.tab.id == id)
            && app.frame != Some(frame)
        {
            app.frame = Some(frame);
            self.dirty = true;
        }
    }

    /// Restarts a page that isn't running (a load that failed to start).
    fn app_reload(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(page) = self.app_page(id) {
            let _ = page.raw.reload();
        }
        cx.notify();
    }
}

/// Closes an app window once the current update is done: the window may be
/// the one handling the click that closes it.
fn remove_window(handle: impl Into<Option<WindowHandle<AppWindow>>>, cx: &mut App) {
    let Some(handle) = handle.into() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    });
}

/// Runs `f` on the browser in its own window, from an app window.
fn with_browser(
    browser: &WeakEntity<Browser>,
    main: AnyWindowHandle,
    cx: &mut App,
    f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>),
) {
    let Some(browser) = browser.upgrade() else {
        return;
    };
    let _ = main.update(cx, |_, window, cx| {
        browser.update(cx, |this, cx| f(this, window, cx))
    });
}

/// An action only the browser window handles (New Tab, Settings…): it goes
/// there, and the browser window comes forward.
fn forward(main: AnyWindowHandle, action: Box<dyn Action>, cx: &mut App) {
    let _ = main.update(cx, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(action, cx);
    });
}

impl AppWindow {
    fn new(
        browser: &Entity<Browser>,
        main: AnyWindowHandle,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = browser.downgrade();
        cx.observe_window_bounds(window, move |this, window, cx| {
            let bounds = window.bounds();
            let frame = [
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            ];
            let id = this.id;
            if let Some(browser) = weak.upgrade() {
                browser.update(cx, |browser, _| browser.app_frame(id, frame));
            }
        })
        .detach();
        // The close button asks the page first, as closing a tab does.
        let (weak, close_id) = (browser.downgrade(), id);
        window.on_window_should_close(cx, move |_, cx| {
            with_browser(&weak, main, cx, |this, window, cx| {
                this.request_discard(lifecycle::Discard::CloseApp(close_id), window, cx)
            });
            false
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            browser: browser.downgrade(),
            main,
            id,
            focus,
            open: false,
            look: None,
        }
    }

    fn browser(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>),
    ) {
        with_browser(&self.browser, self.main, cx, f);
    }

    fn page(&self, cx: &App) -> Option<Rc<engine::Page>> {
        self.browser.upgrade()?.read(cx).app_page(self.id)
    }
}

impl Render for AppWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let Some(look) = self.look.clone() else {
            return div().into_any_element();
        };
        let ground = ui::page_color(look.tint, look.dark);
        let light_ground = ui::luminance(ground) > 0.4;
        let ink = if light_ground { 0x1d1d1f } else { 0xf5f5f7 };
        let wash = if light_ground {
            0x0000_0014
        } else {
            0xffff_ff1f
        };
        window.set_window_title(&look.title);
        let (page, error) = (look.page, look.error);
        let button = |name: &'static str, icon: Icon| {
            div()
                .id(name)
                .role(Role::Button)
                .aria_label(name)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(rgb(ink))
                .opacity(0.75)
                .hover(move |s| s.bg(rgba(wash)).opacity(1.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon.size_3p5())
        };
        let controls = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                button("Back", Icon::new(IconName::ChevronLeft)).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(page) = this.page(cx) {
                            let _ = page.raw.go_back();
                        }
                    },
                )),
            )
            .child(
                button("Forward", Icon::new(IconName::ChevronRight)).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(page) = this.page(cx) {
                            let _ = page.raw.go_forward();
                        }
                    },
                )),
            )
            .child(
                button("Move to Browser Window", Icon::default().path(POP_IN)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.browser(cx, move |browser, window, cx| {
                            browser.return_app(id, window, cx)
                        })
                    }),
                ),
            );
        let handle = div()
            .id("strip-handle")
            .role(Role::Button)
            .aria_label("Show controls")
            .h(px(STRIP))
            .px_4()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = true;
                cx.notify();
            }))
            .child(
                div()
                    .w(px(32.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(ink))
                    .opacity(0.3),
            );
        let strip = div()
            .id("strip")
            .h(px(STRIP))
            .flex_shrink_0()
            .w_full()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(ground))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.open = *hovered;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .child(if self.open {
                controls.into_any_element()
            } else {
                handle.into_any_element()
            });
        let body = match page {
            Some(page) => canvas(
                move |bounds, window, _| page.place(bounds, window.scale_factor()),
                |_, _, _, _| {},
            )
            .size_full()
            .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(ink))
                .opacity(0.7)
                .child(error.unwrap_or_default())
                .into_any_element(),
        };
        let main = self.main;
        div()
            .id("app-window")
            .key_context("AppWindow")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(ground))
            .font_family(if cfg!(target_os = "macos") {
                ".AppleSystemUIFont"
            } else {
                "Inter"
            })
            .on_action(cx.listener(|this, _: &Back, _, cx| {
                if let Some(page) = this.page(cx) {
                    let _ = page.raw.go_back();
                }
            }))
            .on_action(cx.listener(|this, _: &Forward, _, cx| {
                if let Some(page) = this.page(cx) {
                    let _ = page.raw.go_forward();
                }
            }))
            .on_action(cx.listener(move |this, _: &Reload, _, cx| {
                this.browser(cx, move |browser, _, cx| browser.app_reload(id, cx))
            }))
            .on_action(cx.listener(move |this, _: &CloseTab, _, cx| {
                this.browser(cx, move |browser, window, cx| {
                    browser.request_discard(lifecycle::Discard::CloseApp(id), window, cx)
                })
            }))
            .on_action(cx.listener(move |this, _: &PopIn, _, cx| {
                this.browser(cx, move |browser, window, cx| {
                    browser.return_app(id, window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &CopyUrl, _, cx| {
                if let Some(url) = this
                    .page(cx)
                    .and_then(|page| page.url().ok())
                    .filter(|url| model::allowed_address(url))
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                }
            }))
            .on_action(cx.listener(move |this, _: &Print, _, cx| {
                let Some(browser) = this.browser.upgrade() else {
                    return;
                };
                if let Some(app) = browser.read(cx).app(id)
                    && let Some(page) = &app.tab.page
                {
                    page.print(&app.tab.saved.title);
                }
            }))
            .on_action(cx.listener(|this, _: &Inspect, _, cx| {
                if let Some(page) = this.page(cx) {
                    page.raw.open_devtools();
                }
            }))
            .on_action(|_: &menu_bar::EditUndo, _, _| {
                engine::page_undo(false);
            })
            .on_action(|_: &menu_bar::EditRedo, _, _| {
                engine::page_undo(true);
            })
            .on_action(|_: &menu_bar::Minimize, window, _| window.minimize_window())
            .on_action(|_: &menu_bar::ZoomWindow, window, _| window.zoom_window())
            .on_action(|_: &menu_bar::About, _, _| engine::show_about())
            .on_action(|_: &menu_bar::Hide, _, cx| cx.hide())
            .on_action(|_: &menu_bar::HideOthers, _, cx| cx.hide_other_apps())
            .on_action(|_: &menu_bar::ShowAll, _, cx| cx.unhide_other_apps())
            // The rest belong to the browser window.
            .on_action(move |action: &NewTab, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &NewPrivateTab, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &OpenFile, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &FocusAddress, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &ReopenTab, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &ShowHistory, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &ShowSettings, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &RequestLab, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &AddExtension, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &ClearHistory, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &NewSpace, _, cx| forward(main, action.boxed_clone(), cx))
            .on_action(move |action: &Quit, _, cx| forward(main, action.boxed_clone(), cx))
            .child(strip)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
