use crate::*;
use gpui::assets::IconName;
use gpui::base::motion::{self, Transition};
use gpui::component::{
    ActiveTheme, Icon, Theme,
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
pub(crate) const CARD_RADIUS: f32 = 20.;
/// The draggable strip across the top of the page panel: a calm "control
/// zone" in the titlebar band, tinted with the page's own color. It is where
/// a full-page app's chrome will live once pages can go UI-less.
/// Matches the sidebar's top row so its contents share the traffic lights'
/// centerline.
const CONTROL_ZONE: f32 = 44.;
/// Radius scale. Controls (address field, tab cells, rows) share one radius,
/// standalone buttons a step smaller, and anything nested inside a control
/// uses the control's radius minus its inset, so curves stay concentric.
const R_CONTROL: f32 = 12.;
const R_BUTTON: f32 = 10.;
/// Height of the media controls a playing tab's sidebar cell grows by.
const MEDIA_STRIP: f32 = 30.;
/// The find bar's width when open; it eases open from nothing.
const FIND_BAR: f32 = 264.;
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
            // The footer: downloads, when there are any, then the tools.
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
                    .flex_col()
                    .gap_1()
                    .children(
                        self.downloads
                            .iter()
                            .map(|download| self.download_row(download, p, window, cx)),
                    )
                    .child(
                        div()
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
                    ),
            )
            .into_any_element()
    }

    /// A download, shaped like the Request Lab row beneath it: the file's name
    /// with a quiet note (percent, size, or "Failed"), and a hairline of
    /// progress along the bottom while it runs. Hovering swaps the note for
    /// Cancel, or Show in Finder and Remove once it's done; clicking a
    /// finished row also shows it in Finder.
    fn download_row(
        &self,
        download: &downloads::Download,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = download.id;
        let group: SharedString = format!("download-{id}").into();
        let done = download.state == downloads::State::Done;
        let failed = download.state == downloads::State::Failed;
        let fraction = download.fraction().map(|fraction| {
            motion::transition(
                format!("download-progress-{id}"),
                fraction,
                Transition::new(NORMAL),
                window,
                cx,
            )
        });
        let name: SharedString = download.name.clone().into();
        // The tab cell's close button, with a tooltip.
        let action = |id: &'static str, title: &str, icon: IconName| {
            let tip: SharedString = title.to_string().into();
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(title.to_string())
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_CONTROL - 4.))
                .cursor_pointer()
                .text_color(rgb(p.muted))
                .hover(|s| s.bg(rgb(p.selected)).text_color(rgb(p.ink)))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(Icon::new(icon).size_3())
        };
        let actions = div()
            .absolute()
            .right(px(5.))
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .gap_0p5()
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .when(done, |actions| {
                actions.child(
                    action("download-reveal", "Show in Finder", IconName::Search).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.reveal_download(id);
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(if download.active() {
                action("download-cancel", "Cancel download", IconName::X).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.cancel_download(id);
                        cx.notify();
                    },
                ))
            } else {
                action("download-remove", "Remove from list", IconName::X).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.dismiss_download(id);
                        cx.notify();
                    },
                ))
            });
        let bar = download.active().then(|| {
            let track = div()
                .absolute()
                .left(px(10.))
                .right(px(10.))
                .bottom(px(3.))
                .h(px(2.))
                .rounded_full()
                .overflow_hidden()
                .bg(rgba((p.muted << 8) | 0x2e));
            match fraction {
                Some(fraction) => track.child(
                    div()
                        .h_full()
                        .w(relative(fraction))
                        .rounded_full()
                        .bg(rgb(p.accent)),
                ),
                // No length from the server: the address field's sliding bar.
                None => track.child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(relative(0.3))
                        .rounded_full()
                        .bg(rgb(p.accent))
                        .with_animation(
                            ("download-indeterminate", id),
                            Animation::new(Duration::from_millis(1100))
                                .repeat()
                                .with_easing(ease_in_out),
                            |bar, delta| bar.left(relative(delta * 1.3 - 0.3)),
                        ),
                ),
            }
        });
        div()
            .id(("download", id))
            .group(group.clone())
            .role(Role::Button)
            .aria_label(format!(
                "{name}, {}",
                if done {
                    "downloaded".to_string()
                } else if failed {
                    "failed".into()
                } else {
                    download.status()
                }
            ))
            .relative()
            .h(px(32.))
            .flex_shrink_0()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(R_CONTROL))
            .text_size(px(13.))
            .text_color(rgb(p.muted))
            .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
            .when(done, |row| {
                row.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reveal_download(id);
                        cx.notify();
                    }))
            })
            .child(
                Icon::new(if done {
                    IconName::File
                } else if failed {
                    IconName::CircleAlert
                } else {
                    IconName::Download
                })
                .size_4()
                .flex_shrink_0(),
            )
            .child(div().flex_1().min_w_0().truncate().child(name))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .group_hover(group, |s| s.opacity(0.))
                    .child(download.status()),
            )
            .child(actions)
            .children(bar)
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
                .rounded(px(4.))
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
                        .rounded(px(R_BUTTON))
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
                                        .rounded(px(R_BUTTON - 2.))
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
                    if this.find.read(cx).focus_handle(cx).is_focused(w) {
                        this.close_find(w, cx);
                    } else if this.address_focused(w, cx) {
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
            .on_action(cx.listener(|this, _: &Find, w, cx| this.open_find(w, cx)))
            .on_action(cx.listener(|this, _: &FindNext, w, cx| this.find_step(true, w, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, w, cx| this.find_step(false, w, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(Some(true), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(Some(false), cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| this.zoom_by(None, cx)))
            .on_action(cx.listener(|this, _: &Print, _, _| this.print()))
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
        let over_dark = ink == 0xf2f3f5 || (tint.is_none() && self.dark);
        // The theme's caret is a fixed primary that can vanish on a page's
        // color; the strip's ink always reads. Request Lab's fields sit on
        // the palette, where the untinted ink is the palette's own.
        let caret: Hsla = rgb(ink).into();
        if cx.theme().caret != caret {
            Theme::global_mut(cx).caret = caret;
        }
        // Hover washes that work on any page color.
        let wash = if over_dark { 0xffffff1f } else { 0x0000000f };
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
            // Eased with the page's fade-in, so a new site arrives as one.
            .bg(Hsla::from(motion::transition(
                "strip-background",
                Rgb::from(background),
                Transition::new(NORMAL),
                window,
                cx,
            )))
            .text_color(motion::transition(
                "strip-ink",
                Rgb::from(ink),
                Transition::new(NORMAL),
                window,
                cx,
            ))
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
            .child(self.address(Glass { over_dark }, window, cx))
            .when(live && !zoom::is_actual_size(tab.zoom), |strip| {
                strip.child(
                    div()
                        .id("zoom-level")
                        .role(Role::Button)
                        .aria_label("Reset zoom to actual size")
                        .h(px(28.))
                        .px_2()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .rounded(px(R_BUTTON))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .opacity(0.72)
                        .hover(move |s| s.bg(rgba(wash)).opacity(1.))
                        .active(|s| s.opacity(0.55))
                        .tooltip(|window, cx| {
                            Tooltip::new("Actual size")
                                .action(&ActualSize, None)
                                .build(window, cx)
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.zoom_by(None, cx)))
                        .child(format!("{}%", (tab.zoom * 100.).round()))
                        .with_animation(
                            "zoom-level-enter",
                            Animation::new(NORMAL).with_easing(ease_out_quint()),
                            |badge, delta| badge.opacity(0.72 * delta),
                        ),
                )
            })
            .children(self.find_bar(Glass { over_dark }, wash, window, cx))
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

    /// Find in page: the same lifted glass pill the address becomes while
    /// editing, beside it on the strip. It eases open and closed in width,
    /// the address giving way, with its contents laid out at full width so
    /// they never reflow mid-motion. Enter and ⇧Enter step through matches.
    fn find_bar(
        &self,
        glass: Glass,
        wash: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.finding == Some(self.tabs[self.active].id) && !self.lab;
        let reveal = motion::transition(
            "find-open",
            if open { 1. } else { 0. },
            Transition::new(NORMAL),
            window,
            cx,
        );
        if reveal < 0.01 {
            return None;
        }
        let query = self.find.read(cx).value();
        let label = (!query.is_empty())
            .then(|| self.find_matches.map(find_label))
            .flatten();
        let control = |id: &'static str, title: &str, icon: IconName| {
            strip_control(id, title, icon, wash, 22.).rounded_full()
        };
        Some(
            div()
                .id("find")
                .flex_shrink_0()
                .w(px(FIND_BAR * reveal))
                .h(px(30.))
                .mx_1()
                .overflow_hidden()
                .rounded_full()
                .bg(glass.fill(reveal))
                .border_1()
                .border_color(glass.edge(reveal))
                .shadow(glass.shadow(reveal))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .w(px(FIND_BAR - 2.))
                        .h_full()
                        .flex()
                        .items_center()
                        .pr(px(3.))
                        .opacity(reveal)
                        .child(
                            div().flex_1().min_w_0().h_full().child(
                                Input::new(&self.find)
                                    .appearance(false)
                                    .aria_label("Find on page")
                                    .h_full()
                                    .py_0()
                                    .px_3()
                                    .text_size(px(13.))
                                    .prefix(Icon::new(IconName::Search).size_3p5().opacity(0.6)),
                            ),
                        )
                        .children(label.map(|label| {
                            div()
                                .flex_shrink_0()
                                .pr_1()
                                .text_size(px(11.))
                                .opacity(0.6)
                                .whitespace_nowrap()
                                .child(label)
                        }))
                        .child(
                            control("find-previous", "Previous match", IconName::ChevronUp)
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.find_step(false, w, cx)),
                                ),
                        )
                        .child(
                            control("find-next", "Next match", IconName::ChevronDown).on_click(
                                cx.listener(|this, _, w, cx| this.find_step(true, w, cx)),
                            ),
                        )
                        .child(
                            control("find-close", "Done", IconName::X)
                                .on_click(cx.listener(|this, _, w, cx| this.close_find(w, cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The address, filling the strip between the buttons. At rest it is
    /// just the site's icon and host on the strip itself, left-aligned, with
    /// no field around it. Editing lifts a glass pill out of the strip (fill,
    /// hairline and shadow ease in together) while the content cross-fades,
    /// and both states share one layout so nothing jumps. Its text and caret
    /// take the strip's ink, so they read on any page color. Dragging the
    /// resting address moves the window, since it covers most of the strip's
    /// drag area.
    fn address(&self, glass: Glass, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[self.active];
        let editing = self.address.read(cx).focus_handle(cx).is_focused(window);
        let lift = motion::transition(
            "address-lift",
            if editing { 1. } else { 0. },
            Transition::new(NORMAL),
            window,
            cx,
        );
        // Whichever content is showing fades up from partway as the pill
        // moves, so the swap reads as one change rather than a cut.
        let presence = if editing { lift } else { 1. - lift };
        let field = div()
            .id("address")
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .mx_1()
            .flex()
            .items_center()
            .rounded_full()
            .bg(glass.fill(lift))
            .border_1()
            .border_color(glass.edge(lift))
            .shadow(glass.shadow(lift));
        let content = div()
            .size_full()
            .min_w_0()
            .flex()
            .items_center()
            .opacity(0.3 + 0.7 * presence);
        if editing {
            return field
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    content.child(
                        Input::new(&self.address)
                            .appearance(false)
                            .aria_label("Search or enter address")
                            // Line the text up with the resting address.
                            .h_full()
                            .py_0()
                            .px_3()
                            .text_size(px(13.))
                            .prefix(Icon::new(IconName::Search).size_3p5().opacity(0.6)),
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
        field
            .cursor_text()
            .child(
                content
                    .px_3()
                    .gap_1p5()
                    .child(Icon::new(icon).size_3p5().opacity(0.6))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .when(placeholder, |d| d.opacity(0.55))
                            .child(text),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        this.address_press = Some(event.position);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                let Some(start) = this.address_press else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.address_press = None;
                } else if (event.position - start).magnitude() > 4. {
                    this.address_press = None;
                    window.start_window_move();
                }
            }))
            .on_click(cx.listener(|this, _, w, cx| {
                // A press that turned into a window drag isn't a click.
                if this.address_press.take().is_some() {
                    this.focus_address(w, cx);
                }
            }))
            .into_any_element()
    }
}

/// Translucent layers for the address pill, tuned to the strip beneath it:
/// white glass over dark strips, frosted white over light ones. `lift` runs
/// 0 (resting: nothing visible) to 1 (editing).
#[derive(Clone, Copy)]
struct Glass {
    over_dark: bool,
}

impl Glass {
    fn fill(self, lift: f32) -> Hsla {
        white(lift * if self.over_dark { 0.12 } else { 0.72 })
    }

    fn edge(self, lift: f32) -> Hsla {
        if self.over_dark {
            white(0.14 * lift)
        } else {
            black(0.08 * lift)
        }
    }

    fn shadow(self, lift: f32) -> Vec<BoxShadow> {
        let depth = if self.over_dark { 1. } else { 0.45 };
        let layer = |alpha: f32, y: f32, blur: f32| BoxShadow {
            color: black(alpha * depth * lift),
            offset: point(px(0.), px(y * lift)),
            blur_radius: px(blur * lift),
            spread_radius: px(0.),
            inset: false,
        };
        vec![layer(0.22, 1., 2.), layer(0.26, 4., 12.)]
    }
}

fn white(alpha: f32) -> Hsla {
    gpui::Rgba {
        r: 1.,
        g: 1.,
        b: 1.,
        a: alpha,
    }
    .into()
}

fn black(alpha: f32) -> Hsla {
    gpui::Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: alpha,
    }
    .into()
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
    strip_control(id, title, icon, wash, 28.)
}

/// `strip_button` at another size; smaller ones nest inside pills.
fn strip_control(
    id: &'static str,
    title: &str,
    icon: IconName,
    wash: u32,
    size: f32,
) -> Stateful<Div> {
    let tip: SharedString = title.to_string().into();
    let icon = Icon::new(icon);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.to_string())
        .size(px(size))
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
        .child(if size < 28. {
            icon.size_3p5()
        } else {
            icon.size_4()
        })
}

/// "3 of 12", "No matches", or a floor when WebKit stopped counting.
fn find_label(matches: engine::Matches) -> String {
    match matches {
        engine::Matches { count: Some(0), .. } => "No matches".into(),
        engine::Matches {
            count: Some(count),
            index: Some(index),
        } if index < count => format!("{} of {count}", index + 1),
        engine::Matches { count: Some(1), .. } => "1 match".into(),
        engine::Matches {
            count: Some(count), ..
        } => format!("{count} matches"),
        engine::Matches { count: None, .. } => format!("{}+ matches", engine::MAX_MATCHES),
    }
}

/// A color that eases in RGB. GPUI's `Hsla` interpolates hue, so white to
/// navy swept through teal; a straight RGB blend reads as one color change.
#[derive(Clone, Copy, PartialEq)]
struct Rgb([f32; 3]);

impl From<u32> for Rgb {
    fn from(color: u32) -> Self {
        let channel = |shift: u32| ((color >> shift) & 0xff) as f32 / 255.;
        Rgb([channel(16), channel(8), channel(0)])
    }
}

impl motion::Interpolate for Rgb {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        let mix = |i: usize| self.0[i] + (target.0[i] - self.0[i]) * progress;
        Rgb([mix(0), mix(1), mix(2)])
    }
}

impl From<Rgb> for Hsla {
    fn from(Rgb([r, g, b]): Rgb) -> Self {
        gpui::Rgba { r, g, b, a: 1. }.into()
    }
}

/// The control strip's color for a page: its own tint, else the panel color.
pub(crate) fn strip_color(tint: Option<u32>, dark: bool) -> u32 {
    tint.unwrap_or(palette(dark).card)
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
        .rounded(px(R_BUTTON))
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::base::motion::Interpolate;

    // `crate::*` brings in GPUI's own `test` macro; use the standard one.
    #[core::prelude::v1::test]
    fn find_counts_read_naturally() {
        let label = |count, index| find_label(engine::Matches { count, index });
        assert_eq!(label(Some(12), Some(2)), "3 of 12");
        assert_eq!(label(Some(0), None), "No matches");
        assert_eq!(label(Some(4), None), "4 matches");
        assert_eq!(label(Some(1), None), "1 match");
        assert_eq!(label(None, None), "1000+ matches");
    }

    #[core::prelude::v1::test]
    fn strip_colors_blend_in_rgb_not_through_other_hues() {
        // White to navy used to pass through teal (hue interpolation).
        let mid = Rgb::from(0xffffff).interpolate(&Rgb::from(0x1f2a44), 0.5);
        let expected = [
            (1. + 31. / 255.) / 2.,
            (1. + 42. / 255.) / 2.,
            (1. + 68. / 255.) / 2.,
        ];
        for (got, want) in mid.0.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{:?}", mid.0);
        }
        // Blue stays the strongest channel all the way, as navy's is.
        assert!(mid.0[2] > mid.0[1] && mid.0[1] > mid.0[0]);
    }
}
