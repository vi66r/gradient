use crate::*;
use gpui::assets::IconName;
use gpui::base::motion::{self, Transition};
use gpui::component::{
    Icon,
    input::{Input, Textarea},
    tooltip::Tooltip,
};

/// Light and dark tokens. Colors stay in one place so both themes move together.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    /// Window and sidebar ground; the page card sits on it.
    chrome: u32,
    card: u32,
    field: u32,
    line: u32,
    ink: u32,
    muted: u32,
    hover: u32,
    selected: u32,
    accent: u32,
    on_accent: u32,
}

const LIGHT: Palette = Palette {
    chrome: 0xe7e9ed,
    card: 0xffffff,
    field: 0xf5f6f8,
    line: 0xd6dae1,
    ink: 0x1d232c,
    muted: 0x6b7280,
    hover: 0xdcdfe5,
    selected: 0xfbfbfc,
    accent: 0x3f63e0,
    on_accent: 0xffffff,
};

const DARK: Palette = Palette {
    // Lifted off black so the page panel's shadow has something to darken.
    chrome: 0x1b1c20,
    card: 0x212328,
    field: 0x25272c,
    line: 0x2e3137,
    ink: 0xe6e8ec,
    muted: 0x8b919c,
    hover: 0x222429,
    selected: 0x2b2e34,
    accent: 0x7e98ff,
    on_accent: 0x0d1322,
};

pub(crate) fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

const SIDEBAR: f32 = 248.;
/// Matches the native page host's corner radius in engine.rs.
/// The page panel runs flush to the window's top, right and bottom edges; only
/// its sidebar-facing corners are rounded, so it reads as lying above the
/// sidebar rather than framed inside the window.
pub(crate) const CARD_RADIUS: f32 = 16.;
/// The draggable strip across the top of the page panel: a calm "control
/// zone" in the titlebar band, tinted with the page's own color. It is where
/// a full-page app's chrome will live once pages can go UI-less.
/// Matches the sidebar's top row so its contents share the traffic lights'
/// centerline.
const CONTROL_ZONE: f32 = 44.;
/// Radius scale. Controls (address field, tab cells, rows) share one radius,
/// standalone buttons a step smaller, and anything nested inside a control
/// uses the control's radius minus its inset, so curves stay concentric.
const R_CONTROL: f32 = 10.;
const R_BUTTON: f32 = 8.;
/// Height of the media controls a playing tab's sidebar cell grows by.
const MEDIA_STRIP: f32 = 30.;
const FAST: Duration = Duration::from_millis(140);
const NORMAL: Duration = Duration::from_millis(200);

fn hsla(color: u32) -> Hsla {
    rgb(color).into()
}

impl Browser {
    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let grant = self.tabs[self.active].grant.is_some();
        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR))
            .flex_shrink_0()
            .h_full()
            // The traffic lights' row; the Spaces switcher will live here.
            .child(
                div()
                    .id("sidebar-titlebar")
                    .h(px(CONTROL_ZONE))
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, move_window),
            )
            .child(
                div()
                    .id("tab-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id("new-tab")
                            .role(Role::Button)
                            .aria_label("Open New Tab")
                            .h(px(32.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(R_CONTROL))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(rgb(p.muted))
                            .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
                            .child(Icon::new(IconName::Plus).size_4())
                            .child("Open New Tab")
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.new_tab(START.into(), false, w, cx)
                            })),
                    )
                    .children((0..self.tabs.len()).map(|index| self.tab_row(index, p, window, cx))),
            )
            // Tools: Request Lab as a labeled row (discoverable, shows when
            // it's open), and agent sharing as a state toggle beside it.
            .child(
                div()
                    .flex_shrink_0()
                    .mx_2()
                    .mb_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgba(if self.dark { 0xffffff14 } else { 0x0f172a14 }))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id("request-lab")
                            .role(Role::Button)
                            .aria_label("Request lab")
                            .aria_selected(self.lab)
                            .flex_1()
                            .h(px(32.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(R_CONTROL))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(rgb(if self.lab { p.ink } else { p.muted }))
                            .when(self.lab, |d| d.bg(rgb(p.selected)).shadow_xs())
                            .when(!self.lab, |d| {
                                d.hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
                            })
                            .child(Icon::new(IconName::Terminal).size_4())
                            .child("Request Lab")
                            .on_click(cx.listener(|this, _, w, cx| this.show_lab(w, cx))),
                    )
                    .child(
                        icon_button(
                            "agent-access",
                            if grant {
                                "Agents can see this tab. Click to revoke."
                            } else {
                                "Share this tab with agents"
                            },
                            IconName::Bot,
                            p,
                        )
                        .size(px(32.))
                        .rounded(px(R_CONTROL))
                        .when(grant, |d| {
                            d.bg(rgba((p.accent << 8) | 0x24)).text_color(rgb(p.accent))
                        })
                        .on_click(cx.listener(|this, _, w, cx| this.toggle_agent(w, cx))),
                    ),
            )
            .into_any_element()
    }

    fn tab_row(
        &self,
        index: usize,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tab = &self.tabs[index];
        let selected = index == self.active && !self.lab;
        let background = motion::transition(
            format!("tab-bg-{}", tab.id),
            hsla(if selected { p.selected } else { p.chrome }),
            Transition::new(NORMAL),
            window,
            cx,
        );
        let title: SharedString = if tab.saved.title.is_empty() {
            model::display_host(&tab.saved.url).into()
        } else {
            tab.saved.title.clone().into()
        };
        let site_icon = (!tab.private)
            .then(|| favicon::host_key(&tab.saved.url))
            .flatten()
            .and_then(|key| self.icons.get(&key))
            .map(|(image, _)| image.clone());
        let resting = tab.page.is_none() && tab.saved.url != START;
        let icon = if tab.loading {
            Icon::new(IconName::LoaderCircle)
                .size_4()
                .text_color(rgb(p.muted))
                .with_animation(
                    "spin",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
        } else if let Some(image) = site_icon {
            img(image)
                .size(px(16.))
                .flex_shrink_0()
                .rounded(px(3.))
                // Resting tabs keep their identity but read as asleep.
                .when(resting, |icon| icon.opacity(0.5))
                .into_any_element()
        } else {
            Icon::new(if tab.private {
                IconName::Lock
            } else if tab.page.is_none() && tab.saved.url != START {
                IconName::Moon
            } else if model::is_file_url(&tab.saved.url) {
                IconName::FileText
            } else {
                IconName::Globe
            })
            .size_4()
            .text_color(rgb(if selected { p.ink } else { p.muted }))
            .into_any_element()
        };
        let group: SharedString = format!("tab-{}", tab.id).into();
        let id = tab.id;
        let media = tab.media.as_ref().filter(|_| tab.page.is_some());
        let media_height = motion::transition(
            format!("media-height-{id}"),
            if media.is_some_and(|media| media.shown) {
                MEDIA_STRIP
            } else {
                0.
            },
            Transition::new(NORMAL),
            window,
            cx,
        );
        let strip = media.filter(|_| media_height > 0.5).map(|media| {
            let control = |id: &'static str, title: &str, icon: IconName| {
                icon_button(id, title, icon, p)
                    .size(px(24.))
                    .rounded(px(R_CONTROL - 4.))
            };
            div()
                .h(px(media_height))
                .overflow_hidden()
                .opacity((media_height / MEDIA_STRIP).min(1.))
                .flex()
                .items_center()
                .px_1()
                // Equal flexible sides keep playback centred in the cell.
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .when(media.skips, |group| {
                            group.child(
                                control("media-previous", "Previous track", IconName::SkipBack)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.skip_track(id, false);
                                    })),
                            )
                        })
                        .child(
                            control(
                                "media-play",
                                if media.playing { "Pause" } else { "Play" },
                                if media.playing {
                                    IconName::Pause
                                } else {
                                    IconName::Play
                                },
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_media_playback(id, cx);
                                },
                            )),
                        )
                        .when(media.skips, |group| {
                            group.child(
                                control("media-next", "Next track", IconName::SkipForward)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.skip_track(id, true);
                                    })),
                            )
                        }),
                )
                .child(
                    div().flex_1().flex().justify_end().child(
                        control(
                            "media-mute",
                            if media.muted {
                                "Unmute tab"
                            } else {
                                "Mute tab"
                            },
                            if media.muted {
                                IconName::VolumeX
                            } else {
                                IconName::Volume2
                            },
                        )
                        .when(media.muted, |d| d.text_color(rgb(p.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_media_muted(id, cx);
                        })),
                    ),
                )
        });
        let header = div()
            .h(px(34.))
            .flex_shrink_0()
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(if selected { p.ink } else { p.muted }))
                    .child(title.clone()),
            )
            .child(
                div()
                    .id(("close-tab", tab.id))
                    .role(Role::Button)
                    .aria_label(format!("Close {title}"))
                    .size(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(R_CONTROL - 4.))
                    .text_color(rgb(p.muted))
                    .when(!selected, |d| {
                        d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.))
                    })
                    .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
                    .child(Icon::new(IconName::X).size_3())
                    .on_click(cx.listener(move |this, _, w, cx| {
                        cx.stop_propagation();
                        this.close(index, w, cx);
                    })),
            );
        div()
            .id(("tab", tab.id))
            .group(group.clone())
            .role(Role::Tab)
            .aria_selected(selected)
            .aria_label(title.clone())
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded(px(R_CONTROL))
            .cursor_pointer()
            .bg(background)
            .when(selected, |d| d.shadow_xs())
            .when(!selected, |d| d.hover(|s| s.bg(rgb(p.hover))))
            .child(header)
            .children(strip)
            .on_click(cx.listener(move |this, _, w, cx| this.activate(index, w, cx)))
            // Acts on this tab without switching to it, as in Safari.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.tab_menu(id, event.position, window, cx);
                }),
            )
            // New tabs ease in once; restored tabs settle with them at launch.
            .with_animation(
                ("tab-enter", tab.id),
                Animation::new(NORMAL).with_easing(ease_out_quint()),
                |row, delta| row.opacity(delta).ml(px(6. * (1. - delta))),
            )
    }

    fn start_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let private = self.tabs[self.active].private;
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .pb(px(96.))
            .child(
                Icon::new(if private {
                    IconName::Lock
                } else {
                    IconName::Compass
                })
                .size(px(30.))
                .text_color(rgb(p.accent)),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(px(26.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if private {
                        "A little privacy."
                    } else {
                        "A little room to explore."
                    }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(p.muted))
                    .child(if private {
                        "This tab’s address and website data won’t be saved."
                    } else {
                        "Type an address or a search into the sidebar."
                    }),
            )
            .child(
                div()
                    .mt_5()
                    .flex()
                    .gap_2()
                    .child(
                        pill_button("start-webkit", "WebKit", IconName::Globe, p).on_click(
                            cx.listener(|this, _, w, cx| {
                                this.navigate("https://webkit.org/".into(), w, cx)
                            }),
                        ),
                    )
                    .child(
                        pill_button("start-3d", "WebGPU samples", IconName::Box, p).on_click(
                            cx.listener(|this, _, w, cx| {
                                this.navigate(
                                    "https://webgpu.github.io/webgpu-samples/".into(),
                                    w,
                                    cx,
                                )
                            }),
                        ),
                    )
                    .child(
                        pill_button("start-request", "Make a request", IconName::Terminal, p)
                            .on_click(cx.listener(|this, _, w, cx| this.show_lab(w, cx))),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(if cfg!(target_os = "macos") {
                        "⌘L go somewhere   ·   ⌘T new tab   ·   ⌘⇧C copy address"
                    } else {
                        "Ctrl L go somewhere   ·   Ctrl T new tab   ·   Ctrl Shift C copy address"
                    }),
            )
            .into_any_element()
    }

    fn resting_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let tab = &self.tabs[self.active];
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .pb(px(64.))
            .child(
                Icon::new(if tab.error.is_some() {
                    IconName::CloudOff
                } else {
                    IconName::Moon
                })
                .size(px(28.))
                .text_color(rgb(p.muted)),
            )
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if tab.error.is_some() {
                        "This page couldn’t open."
                    } else {
                        "This tab is resting."
                    }),
            )
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(rgb(p.muted))
                    .child(tab.error.clone().unwrap_or_else(|| {
                        "It’s using no memory. Resume it when you’re ready.".into()
                    })),
            )
            .child(
                div().mt_3().child(
                    pill_button(
                        "resume-tab",
                        if tab.error.is_some() {
                            "Try again"
                        } else {
                            "Resume tab"
                        },
                        IconName::RotateCw,
                        p,
                    )
                    .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
                ),
            )
            .into_any_element()
    }

    fn request_lab(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let copied = self.copy_feedback == Some(CopyTarget::Curl);
        div()
            .id("request-workbench")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Request lab"),
                            )
                            .child(div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                                "Requests stay in memory and never share the page’s cookies.",
                            )),
                    )
                    .child(
                        pill_button("back-to-page", "Back to page", IconName::ArrowLeft, p)
                            .on_click(
                                cx.listener(|this, _, w, cx| this.activate(this.active, w, cx)),
                            ),
                    ),
            )
            .child(
                div().flex().items_center().gap_2().child(
                    div()
                        .flex()
                        .p_0p5()
                        .gap_0p5()
                        .rounded(px(8.))
                        .bg(rgb(p.field))
                        .children(
                            ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
                                .into_iter()
                                .map(|method| {
                                    let active = self.request_method == method;
                                    div()
                                        .id(method)
                                        .role(Role::Button)
                                        .aria_label(format!("HTTP method {method}"))
                                        .aria_selected(active)
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .cursor_pointer()
                                        .text_color(rgb(if active { p.ink } else { p.muted }))
                                        .when(active, |d| d.bg(rgb(p.card)).shadow_xs())
                                        .when(!active, |d| d.hover(|s| s.text_color(rgb(p.ink))))
                                        .child(method)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.request_method = method.into();
                                            cx.notify();
                                        }))
                                }),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.request_url).aria_label("Request URL")),
                    )
                    .child(
                        pill_button(
                            "send-request",
                            if self.requesting {
                                "Sending…"
                            } else {
                                "Send"
                            },
                            IconName::ArrowUpRight,
                            p,
                        )
                        .bg(rgb(p.accent))
                        .text_color(rgb(p.on_accent))
                        .border_color(rgb(p.accent))
                        .hover(|s| s.bg(rgb(p.accent)).opacity(0.9))
                        .on_click(cx.listener(|this, _, w, cx| this.send_request(w, cx))),
                    )
                    .child(
                        pill_button(
                            "copy-curl",
                            if copied { "Copied" } else { "Copy cURL" },
                            if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            },
                            p,
                        )
                        .w(px(112.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(http_client::curl(
                                &this.request(cx),
                            )));
                            this.flash_copy(CopyTarget::Curl, cx);
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("Headers", p))
                            .child(
                                Textarea::new(&self.request_headers)
                                    .h(px(132.))
                                    .aria_label("Request headers"),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("Body", p))
                            .child(
                                Textarea::new(&self.request_body)
                                    .h(px(132.))
                                    .aria_label("Request body"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label("Response", p))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(self.response_summary.clone()),
                    ),
            )
            .child(
                Textarea::new(&self.response)
                    .h(px(320.))
                    .readonly(true)
                    .aria_label("Response"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Redirects are shown without following them."),
            )
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(self.dark);
        let tab = &self.tabs[self.active];
        let (content, key): (AnyElement, SharedString) = if self.lab {
            (self.request_lab(cx), "lab".into())
        } else if let Some(page) = &tab.page {
            let page = page.clone();
            (
                canvas(
                    move |bounds, window, _| page.place(bounds, window.scale_factor()),
                    |_, _, _, _| {},
                )
                .size_full()
                .into_any_element(),
                format!("page-{}", tab.id).into(),
            )
        } else if tab.saved.url == START {
            (self.start_page(cx), format!("start-{}", tab.id).into())
        } else {
            (self.resting_page(cx), format!("rest-{}", tab.id).into())
        };
        let body = div().flex_1().min_h_0().child(content);
        // Native pages draw above GPUI and can't fade; shell-drawn content can.
        let body = if !self.lab && tab.page.is_some() {
            body.into_any_element()
        } else {
            body.with_animation(
                key,
                Animation::new(FAST).with_easing(ease_out_quint()),
                |content, delta| content.opacity(delta),
            )
            .into_any_element()
        };
        div()
            .key_context("Browser")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(frosted(p.chrome, self.dark))
            .text_color(rgb(p.ink))
            .font_family(if cfg!(target_os = "macos") {
                ".AppleSystemUIFont"
            } else {
                "Inter"
            })
            .on_action(
                cx.listener(|this, _: &NewTab, w, cx| this.new_tab(START.into(), false, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewPrivateTab, w, cx| {
                    this.new_tab(START.into(), true, w, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenFile, w, cx| this.prompt_open_file(w, cx)))
            // Files dropped on the shell (sidebar, start or resting page). The
            // native page takes its own drops; see `engine::Page::create`.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, w, cx| {
                this.open_paths(paths.paths(), false, w, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTab, w, cx| this.close(this.active, w, cx)))
            .on_action(cx.listener(|this, _: &ReopenTab, w, cx| {
                if let Some(tab) = this.closed.pop() {
                    this.new_tab(tab.url, false, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusAddress, w, cx| this.focus_address(w, cx)))
            .on_action(
                cx.listener(|this, _: &gpui::component::input::Escape, w, cx| {
                    if this.address_focused(w, cx) {
                        this.release_address(w, cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &CopyUrl, _, cx| this.copy_url(cx)))
            .on_action(cx.listener(|this, action: &TabMenu, w, cx| this.tab_command(action, w, cx)))
            .on_action(cx.listener(|this, _: &Back, _, _| this.history(false)))
            .on_action(cx.listener(|this, _: &Forward, _, _| this.history(true)))
            .on_action(cx.listener(|this, _: &Reload, w, cx| this.reload(w, cx)))
            .on_action(cx.listener(|this, _: &NextTab, w, cx| {
                this.activate((this.active + 1) % this.tabs.len(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, w, cx| {
                this.activate((this.active + this.tabs.len() - 1) % this.tabs.len(), w, cx)
            }))
            .on_action(cx.listener(|this, _: &RequestLab, w, cx| this.show_lab(w, cx)))
            .on_action(cx.listener(|this, _: &Inspect, _, _| {
                if let Some(page) = &this.tabs[this.active].page {
                    page.raw.open_devtools();
                }
            }))
            .on_action(cx.listener(|this, _: &Suspend, w, cx| this.suspend(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleAgent, w, cx| this.toggle_agent(w, cx)))
            .on_action(cx.listener(|this, _: &Quit, w, cx| {
                this.request_discard(lifecycle::Discard::Quit, w, cx);
            }))
            .child(self.sidebar(window, cx))
            .child(
                div().flex_1().min_w_0().h_full().child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded_l(px(CARD_RADIUS))
                        .bg(rgb(p.card))
                        .border_l_1()
                        .border_color(panel_edge(self.dark))
                        .shadow(panel_shadow(self.dark))
                        .child(self.control_zone(p, window, cx))
                        .child(body)
                        .when_some(self.notice.clone(), |d, message| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .min_h(px(36.))
                                    .px_3()
                                    .py_1()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .border_t_1()
                                    .border_color(rgb(p.line))
                                    .bg(rgb(p.field))
                                    .text_size(px(12.))
                                    .child(
                                        Icon::new(IconName::Info)
                                            .size_3p5()
                                            .text_color(rgb(p.muted)),
                                    )
                                    .child(div().flex_1().min_w_0().child(message))
                                    .child(
                                        icon_button("dismiss-notice", "Dismiss", IconName::X, p)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.notice = None;
                                                cx.notify();
                                            })),
                                    )
                                    .with_animation(
                                        "notice-enter",
                                        Animation::new(NORMAL).with_easing(ease_out_quint()),
                                        |bar, delta| bar.opacity(delta),
                                    ),
                            )
                        }),
                ),
            )
    }
}

/// The page sits on a raised panel above the chrome: a tight contact shadow,
/// a soft mid shadow and a wide ambient one, like a macOS window. Dark mode
/// needs much denser shadows to read against near-black chrome.
fn panel_shadow(dark: bool) -> Vec<BoxShadow> {
    let layer = |color: u32, y: f32, blur: f32, spread: f32| BoxShadow {
        color: rgba(color).into(),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    };
    // Small and faint: wide, low-alpha blurs band on 8-bit surfaces.
    if dark {
        vec![
            layer(0x00000052, 0., 2., 0.),
            layer(0x00000047, 0., 10., 0.),
        ]
    } else {
        vec![
            layer(0x0f172a14, 0., 2., 0.),
            layer(0x0f172a0f, 0., 10., 0.),
        ]
    }
}

impl Browser {
    /// The page's own top bar: navigation, a borderless address and page
    /// actions on a strip tinted with the page's color. Empty space drags the
    /// window (double-click zooms); controls keep their clicks to themselves.
    fn control_zone(
        &self,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tab = &self.tabs[self.active];
        let tint = tab.tint.filter(|_| !self.lab && tab.page.is_some());
        let background = tint.unwrap_or(p.card);
        let ink = tint.map(readable_on).unwrap_or(p.ink);
        // Hover washes that work on any page color.
        let wash = if ink == 0xf2f3f5 || (tint.is_none() && self.dark) {
            0xffffff1f
        } else {
            0x0000000f
        };
        let live = tab.page.is_some() && !self.lab;
        let addressed = tab.saved.url != START;
        let copied = self.copy_feedback == Some(CopyTarget::Address);
        div()
            .id("control-zone")
            .h(px(CONTROL_ZONE))
            // GPUI clips children to rectangles, not the panel's rounded
            // shape, so the strip rounds its own corner to match.
            .rounded_tl(px(CARD_RADIUS))
            .flex_shrink_0()
            .relative()
            .px_2()
            .flex()
            .items_center()
            .gap_0p5()
            .bg(rgb(background))
            .text_color(rgb(ink))
            .child(
                strip_button("back", "Back", IconName::ChevronLeft, wash)
                    .on_click(cx.listener(|this, _, _, _| this.history(false))),
            )
            .child(
                strip_button("forward", "Forward", IconName::ChevronRight, wash)
                    .on_click(cx.listener(|this, _, _, _| this.history(true))),
            )
            .child(
                strip_button("reload", "Reload", IconName::RotateCw, wash)
                    .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
            )
            .child(self.address(p, wash, window, cx))
            .child(div().flex_1().h_full())
            .when(addressed, |strip| {
                strip.child(
                    strip_button(
                        "copy-url",
                        if copied {
                            "Address copied"
                        } else {
                            "Copy address"
                        },
                        if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        },
                        wash,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.copy_url(cx)))
                    .with_animation(
                        ("copy-feedback", self.copy_epoch),
                        Animation::new(NORMAL).with_easing(ease_out_quint()),
                        |button, delta| button.opacity(0.4 + 0.6 * delta),
                    ),
                )
            })
            .when(live, |strip| {
                strip.child(
                    strip_button("inspector", "Inspect page", IconName::Code, wash).on_click(
                        cx.listener(|this, _, _, _| {
                            if let Some(page) = &this.tabs[this.active].page {
                                page.raw.open_devtools();
                            }
                        }),
                    ),
                )
            })
            .when(tab.loading && !self.lab, |strip| {
                strip.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .h(px(2.))
                        .w(relative(0.3))
                        .rounded_full()
                        .bg(rgb(p.accent))
                        .with_animation(
                            "loading",
                            Animation::new(Duration::from_millis(1100))
                                .repeat()
                                .with_easing(ease_in_out),
                            |bar, delta| bar.left(relative(delta * 1.3 - 0.3)),
                        ),
                )
            })
            .on_mouse_down(MouseButton::Left, move_window)
    }

    /// Borderless at rest (lock or globe and the host, as in the page's own
    /// colors); a solid field while editing so typed text always reads.
    fn address(
        &self,
        p: Palette,
        wash: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = &self.tabs[self.active];
        let editing = self.address.read(cx).focus_handle(cx).is_focused(window);
        let field = div()
            .id("address")
            .min_w_0()
            .h(px(30.))
            .mx_1()
            .flex()
            .items_center()
            .rounded(px(R_CONTROL))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if editing {
            return field
                .flex_1()
                .px_1()
                .bg(rgb(p.card))
                .border_1()
                .border_color(rgb(p.accent))
                .text_color(rgb(p.ink))
                .child(
                    Input::new(&self.address)
                        .appearance(false)
                        .aria_label("Search or enter address")
                        .prefix(
                            Icon::new(IconName::Search)
                                .size_3p5()
                                .text_color(rgb(p.muted)),
                        ),
                )
                .into_any_element();
        }
        let url = &tab.saved.url;
        let (icon, text, placeholder) = if url == START {
            (
                IconName::Search,
                "Search or enter address".to_string(),
                true,
            )
        } else {
            (
                if tab.private {
                    IconName::Lock
                } else if url.starts_with("https://") {
                    IconName::LockKeyhole
                } else if model::is_file_url(url) {
                    IconName::FileText
                } else {
                    IconName::Globe
                },
                address_label(url),
                false,
            )
        };
        // At rest the address hugs its text; the rest of the strip stays free
        // for dragging the window.
        field
            .flex_shrink(1.)
            .max_w(relative(0.6))
            .px_2()
            .gap_2()
            .cursor_text()
            .hover(move |s| s.bg(rgba(wash)))
            .child(Icon::new(icon).size_3p5().opacity(0.6))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .when(placeholder, |d| d.opacity(0.55))
                    .child(text),
            )
            .on_click(cx.listener(|this, _, w, cx| this.focus_address(w, cx)))
            .into_any_element()
    }
}

/// Titlebar behaviour for shell-drawn titlebar areas: drag moves the window,
/// a double-click zooms it (or whatever the user set in System Settings).
fn move_window(event: &MouseDownEvent, window: &mut Window, _: &mut App) {
    if event.click_count == 2 {
        window.titlebar_double_click();
    } else {
        window.start_window_move();
    }
}

/// A control on the page-tinted strip: inherits the strip's ink, uses a
/// translucent wash on hover, and never starts a window drag.
fn strip_button(id: &'static str, title: &str, icon: IconName, wash: u32) -> Stateful<Div> {
    let tip: SharedString = title.to_string().into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.to_string())
        .size(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_BUTTON))
        .cursor_pointer()
        .opacity(0.72)
        .hover(move |s| s.bg(rgba(wash)).opacity(1.))
        .active(|s| s.opacity(0.55))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(Icon::new(icon).size_4())
}

/// Dark or light text, whichever reads on `background` (`0xRRGGBB`).
fn readable_on(background: u32) -> u32 {
    let channel = |shift: u32| {
        let value = ((background >> shift) & 0xff) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
    if luminance > 0.4 { 0x1d232c } else { 0xf2f3f5 }
}

/// A light veil of our palette over the system sidebar material, so the frost
/// keeps the shell's tone while the desktop still shows through.
fn frosted(chrome: u32, dark: bool) -> Hsla {
    let alpha = if dark { 0x59 } else { 0x40 };
    rgba((chrome << 8) | alpha).into()
}

/// A translucent hairline, so the edge reads as light rather than a stroke.
fn panel_edge(dark: bool) -> Hsla {
    rgba(if dark { 0xffffff12 } else { 0x0f172a12 }).into()
}

/// Host and port (or a local file's name) for the resting address field; the
/// full URL appears on focus.
fn address_label(url: &str) -> String {
    if let Some(name) = model::file_name(url) {
        return name;
    }
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            let host = parsed.host_str()?.trim_start_matches("www.").to_string();
            Some(match parsed.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| url.to_string())
}

fn icon_button(id: &'static str, title: &str, icon: IconName, p: Palette) -> Stateful<Div> {
    let tip: SharedString = title.to_string().into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.to_string())
        .size(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_BUTTON))
        .cursor_pointer()
        .text_color(rgb(p.muted))
        .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
        .active(|s| s.opacity(0.7))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(Icon::new(icon).size_4())
}

fn pill_button(id: &'static str, title: &str, icon: IconName, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.to_string())
        .h(px(32.))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(p.line))
        .bg(rgb(p.card))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .hover(|s| s.bg(rgb(p.field)))
        .active(|s| s.opacity(0.75))
        .child(Icon::new(icon).size_3p5())
        .child(title.to_string())
}

fn label(text: &'static str, p: Palette) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(p.muted))
        .child(text)
}
