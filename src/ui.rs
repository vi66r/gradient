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
    chrome: 0x16171a,
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
pub(crate) const CARD_RADIUS: f32 = 10.;
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
            .child(
                // Traffic lights sit at the left of this row.
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_0p5()
                    .px_2()
                    .child(
                        icon_button("back", "Back", IconName::ChevronLeft, p)
                            .on_click(cx.listener(|this, _, _, _| this.history(false))),
                    )
                    .child(
                        icon_button("forward", "Forward", IconName::ChevronRight, p)
                            .on_click(cx.listener(|this, _, _, _| this.history(true))),
                    )
                    .child(
                        icon_button("reload", "Reload", IconName::RotateCw, p)
                            .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
                    ),
            )
            .child(div().px_2().pb_3().child(self.address_field(window, cx)))
            .child(
                div()
                    .id("tab-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
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
                            .rounded(px(8.))
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
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        icon_button("request-lab", "Request lab", IconName::Terminal, p)
                            .when(self.lab, |d| d.bg(rgb(p.selected)).text_color(rgb(p.ink)))
                            .on_click(cx.listener(|this, _, w, cx| this.show_lab(w, cx))),
                    )
                    .child(
                        icon_button(
                            "agent-access",
                            if grant {
                                "Revoke agent access"
                            } else {
                                "Share tab with agent"
                            },
                            IconName::Bot,
                            p,
                        )
                        .when(grant, |d| d.text_color(rgb(p.accent)))
                        .on_click(cx.listener(|this, _, w, cx| this.toggle_agent(w, cx))),
                    )
                    .child(
                        icon_button("inspector", "Inspect page", IconName::Code, p).on_click(
                            cx.listener(|this, _, _, _| {
                                if let Some(page) = &this.tabs[this.active].page {
                                    page.raw.open_devtools();
                                }
                            }),
                        ),
                    )
                    .when(grant, |d| {
                        d.child(
                            div()
                                .ml_1()
                                .text_size(px(11.))
                                .text_color(rgb(p.accent))
                                .child("Agent can see this tab"),
                        )
                    }),
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
        } else {
            Icon::new(if tab.private {
                IconName::Lock
            } else if tab.page.is_none() && tab.saved.url != START {
                IconName::Moon
            } else {
                IconName::Globe
            })
            .size_4()
            .text_color(rgb(if selected { p.ink } else { p.muted }))
            .into_any_element()
        };
        let group: SharedString = format!("tab-{}", tab.id).into();
        div()
            .id(("tab", tab.id))
            .group(group.clone())
            .role(Role::Tab)
            .aria_selected(selected)
            .aria_label(title.clone())
            .h(px(34.))
            .flex_shrink_0()
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .cursor_pointer()
            .bg(background)
            .when(selected, |d| d.shadow_xs())
            .when(!selected, |d| d.hover(|s| s.bg(rgb(p.hover))))
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
                    .rounded(px(6.))
                    .text_color(rgb(p.muted))
                    .when(!selected, |d| {
                        d.opacity(0.).group_hover(group, |s| s.opacity(1.))
                    })
                    .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
                    .child(Icon::new(IconName::X).size_3())
                    .on_click(cx.listener(move |this, _, w, cx| {
                        cx.stop_propagation();
                        this.close(index, w, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, w, cx| this.activate(index, w, cx)))
            // New tabs ease in once; restored tabs settle with them at launch.
            .with_animation(
                ("tab-enter", tab.id),
                Animation::new(NORMAL).with_easing(ease_out_quint()),
                |row, delta| row.opacity(delta).ml(px(6. * (1. - delta))),
            )
    }

    fn address_field(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let tab = &self.tabs[self.active];
        let editing = self.address.read(cx).focus_handle(cx).is_focused(window);
        let field = div()
            .id("address")
            .h(px(36.))
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .rounded(px(9.))
            .bg(rgb(if editing { p.card } else { p.field }))
            .border_1()
            .border_color(rgb(if editing { p.accent } else { p.line }));
        let field = if editing {
            field.px_1().child(
                Input::new(&self.address)
                    .appearance(false)
                    .aria_label("Search or enter address")
                    .prefix(
                        Icon::new(IconName::Search)
                            .size_3p5()
                            .text_color(rgb(p.muted)),
                    ),
            )
        } else {
            let url = &tab.saved.url;
            let secure = url.starts_with("https://");
            let (icon, text, ink) = if url == START {
                (
                    IconName::Search,
                    "Search or enter address".to_string(),
                    p.muted,
                )
            } else {
                (
                    if tab.private {
                        IconName::Lock
                    } else if secure {
                        IconName::LockKeyhole
                    } else {
                        IconName::Globe
                    },
                    address_label(url),
                    p.ink,
                )
            };
            let copied = self.copy_feedback == Some(CopyTarget::Address);
            field
                .pl_2p5()
                .pr_1()
                .gap_2()
                .cursor_text()
                .hover(|s| s.bg(rgb(p.card)))
                .child(Icon::new(icon).size_3p5().text_color(rgb(p.muted)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(rgb(ink))
                        .child(text),
                )
                .when(url != START, |d| {
                    d.child(
                        icon_button(
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
                            p,
                        )
                        .size(px(26.))
                        .when(copied, |d| d.text_color(rgb(p.accent)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.copy_url(cx);
                        }))
                        .with_animation(
                            ("copy-feedback", self.copy_epoch),
                            Animation::new(NORMAL).with_easing(ease_out_quint()),
                            |button, delta| button.opacity(0.4 + 0.6 * delta),
                        ),
                    )
                })
                .on_click(cx.listener(|this, _, w, cx| this.focus_address(w, cx)))
        };
        field
            .when(tab.loading && !self.lab, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .h(px(2.))
                        .w(relative(0.4))
                        .rounded_full()
                        .bg(rgb(p.accent))
                        .with_animation(
                            "loading",
                            Animation::new(Duration::from_millis(1100))
                                .repeat()
                                .with_easing(ease_in_out),
                            |bar, delta| bar.left(relative(delta * 1.4 - 0.4)),
                        ),
                )
            })
            .into_any_element()
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
            .bg(rgb(p.chrome))
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
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                this.save();
                cx.quit();
            }))
            .child(self.sidebar(window, cx))
            .child(
                div().flex_1().min_w_0().h_full().py_2().pr_2().child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded(px(CARD_RADIUS))
                        .bg(rgb(p.card))
                        .border_1()
                        .border_color(rgb(p.line))
                        .shadow_sm()
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

/// Host and port for the resting address field; the full URL appears on focus.
fn address_label(url: &str) -> String {
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
        .rounded(px(7.))
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
