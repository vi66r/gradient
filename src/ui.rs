use crate::*;
use gpui::assets::IconName;
use gpui::component::{
    Icon,
    input::{Input, Textarea},
};

const BG: u32 = 0xf7f8fa;
const PANEL: u32 = 0xedeff3;
const LINE: u32 = 0xdde1e8;
const INK: u32 = 0x252c38;
const MUTED: u32 = 0x77808f;
const ACCENT: u32 = 0x456ae6;

impl Browser {
    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let live = self.tabs.iter().filter(|tab| tab.page.is_some()).count();
        div()
            .flex()
            .flex_col()
            .w(px(224.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .h(px(64.))
                    .flex_shrink_0()
                    .flex()
                    .items_end()
                    .pb_3()
                    .px_5()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Browser"),
            )
            .child(
                div().px_3().pb_3().child(
                    control("new-tab", "New tab", IconName::Plus)
                        .w_full()
                        .justify_start()
                        .on_click(
                            cx.listener(|this, _, w, cx| this.new_tab(START.into(), false, w, cx)),
                        ),
                ),
            )
            .child(
                div()
                    .px_5()
                    .pb_2()
                    .text_size(px(10.))
                    .text_color(rgb(MUTED))
                    .child("TABS"),
            )
            .child(
                div()
                    .id("tab-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                        let selected = index == self.active && !self.lab;
                        let icon = if tab.private {
                            IconName::Lock
                        } else if tab.page.is_none() && tab.saved.url != START {
                            IconName::Moon
                        } else {
                            IconName::Globe
                        };
                        div()
                            .id(("tab", tab.id))
                            .role(Role::Tab)
                            .aria_selected(selected)
                            .aria_label(tab.saved.title.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .p_2()
                            .mb_1()
                            .rounded_lg()
                            .cursor_pointer()
                            .bg(rgb(if selected { 0xffffff } else { PANEL }))
                            .hover(|s| s.bg(rgb(if selected { 0xffffff } else { 0xe2e6ee })))
                            .child(Icon::new(icon).size_4().text_color(rgb(if selected {
                                ACCENT
                            } else {
                                MUTED
                            })))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_size(px(12.)).truncate().child(
                                        if tab.saved.title.is_empty() {
                                            model::display_host(&tab.saved.url)
                                        } else {
                                            tab.saved.title.clone()
                                        },
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(MUTED))
                                            .truncate()
                                            .child(if tab.loading {
                                                "Loading…".into()
                                            } else if tab.private {
                                                "Private tab".into()
                                            } else if tab.saved.url == START {
                                                "Start somewhere".into()
                                            } else {
                                                model::display_host(&tab.saved.url)
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id(("close-tab", tab.id))
                                    .role(Role::Button)
                                    .aria_label(format!("Close {}", tab.saved.title))
                                    .size(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .hover(|s| s.bg(rgb(0xdce1eb)))
                                    .child(Icon::new(IconName::X).size_3())
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        cx.stop_propagation();
                                        this.close(index, w, cx);
                                    })),
                            )
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.activate(index, w, cx)),
                            )
                    })),
            )
            .child(
                div()
                    .px_3()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        control("request-lab", "Request lab", IconName::Terminal)
                            .w_full()
                            .justify_start()
                            .when(self.lab, |d| d.bg(rgb(0xffffff)))
                            .on_click(cx.listener(|this, _, w, cx| this.show_lab(w, cx))),
                    )
                    .child(
                        control(
                            "agent-access",
                            if self.tabs[self.active].grant.is_some() {
                                "Revoke agent access"
                            } else {
                                "Share tab with agent"
                            },
                            IconName::Code,
                        )
                        .w_full()
                        .justify_start()
                        .when(self.tabs[self.active].grant.is_some(), |d| {
                            d.bg(rgb(0xdce6ff))
                        })
                        .on_click(cx.listener(|this, _, w, cx| this.toggle_agent(w, cx))),
                    )
                    .child(
                        div()
                            .px_2()
                            .text_size(px(10.))
                            .text_color(rgb(MUTED))
                            .child(format!("{live} live · {} tabs", self.tabs.len())),
                    ),
            )
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .h(px(64.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                control("back", "Back", IconName::ChevronLeft)
                    .w(px(32.))
                    .on_click(cx.listener(|this, _, _, _| this.history(false))),
            )
            .child(
                control("forward", "Forward", IconName::ChevronRight)
                    .w(px(32.))
                    .on_click(cx.listener(|this, _, _, _| this.history(true))),
            )
            .child(
                control("reload", "Reload", IconName::RotateCw)
                    .w(px(32.))
                    .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
            )
            .child(
                div().flex_1().min_w_0().mx_2().child(
                    Input::new(&self.address)
                        .aria_label("Search or enter address")
                        .prefix(Icon::new(IconName::Globe).size_3().text_color(rgb(MUTED))),
                ),
            )
            .child(
                control("copy-url", "Copy address", IconName::Copy)
                    .w(px(32.))
                    .on_click(cx.listener(|this, _, _, cx| this.copy_url(cx))),
            )
            .child(
                control("inspector", "Inspect page", IconName::Code)
                    .w(px(32.))
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(page) = &this.tabs[this.active].page {
                            page.raw.open_devtools();
                        }
                    })),
            )
            .into_any_element()
    }

    fn start_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let private = self.tabs[self.active].private;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .pb(px(100.))
            .child(
                Icon::new(if private {
                    IconName::Lock
                } else {
                    IconName::Compass
                })
                .size(px(38.))
                .text_color(rgb(ACCENT)),
            )
            .child(
                div()
                    .text_size(px(32.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if private {
                        "A little privacy."
                    } else {
                        "A little room to explore."
                    }),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(rgb(MUTED))
                    .child(if private {
                        "This tab’s address and website data won’t be saved."
                    } else {
                        "Search above. Follow your curiosity."
                    }),
            )
            .child(
                div()
                    .mt_6()
                    .flex()
                    .gap_3()
                    .child(control("start-webkit", "WebKit", IconName::Globe).on_click(
                        cx.listener(|this, _, w, cx| {
                            this.navigate("https://webkit.org/".into(), w, cx)
                        }),
                    ))
                    .child(
                        control("start-3d", "Explore WebGPU", IconName::Box).on_click(cx.listener(
                            |this, _, w, cx| {
                                this.navigate(
                                    "https://webgpu.github.io/webgpu-samples/".into(),
                                    w,
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        control("start-request", "Make a request", IconName::Terminal)
                            .on_click(cx.listener(|this, _, w, cx| this.show_lab(w, cx))),
                    ),
            )
            .child(
                div()
                    .mt_5()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(if cfg!(target_os = "macos") {
                        "⌘ L to go somewhere   ·   ⌘ ⇧ C to copy its address"
                    } else {
                        "Ctrl L to go somewhere   ·   Ctrl Shift C to copy its address"
                    }),
            )
            .into_any_element()
    }

    fn request_lab(&self, cx: &mut Context<Self>) -> AnyElement {
        div().id("request-workbench").flex_1().min_h_0().overflow_y_scroll().p_6().flex().flex_col().gap_4()
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap_2().child(div().text_size(px(24.)).font_weight(FontWeight::MEDIUM).child("Request lab"))
                    .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child("A request, a response, and room to understand it.")))
                .child(control("back-to-page","Back to page",IconName::ArrowLeft).on_click(cx.listener(|this,_,w,cx|this.activate(this.active,w,cx)))))
            .child(div().flex().gap_1().children(["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS"].into_iter().map(|method| {
                div().id(method).role(Role::Button).aria_label(format!("HTTP method {method}")).px_3().py_2().rounded_md().text_size(px(11.)).cursor_pointer().bg(rgb(if self.request_method==method {ACCENT} else {PANEL})).text_color(rgb(if self.request_method==method {0xffffff} else {MUTED})).child(method)
                    .on_click(cx.listener(move |this,_,_,cx| { this.request_method=method.into();cx.notify(); }))
            })))
            .child(div().flex().items_center().gap_2().child(div().flex_1().child(Input::new(&self.request_url).aria_label("Request URL")))
                .child(control("send-request",if self.requesting {"Sending…"} else {"Send request"},IconName::ArrowUpRight).bg(rgb(ACCENT)).text_color(rgb(0xffffff)).on_click(cx.listener(|this,_,w,cx|this.send_request(w,cx))))
                .child(control("copy-curl","Copy cURL",IconName::Copy).on_click(cx.listener(|this,_,_,cx| {cx.write_to_clipboard(ClipboardItem::new_string(http_client::curl(&this.request(cx))));this.notice=Some("cURL copied.".into());cx.notify();}))))
            .child(div().flex().gap_4()
                .child(div().flex_1().min_w_0().flex().flex_col().gap_2().child(label("HEADERS")).child(Textarea::new(&self.request_headers).h(px(136.)).aria_label("Request headers")))
                .child(div().flex_1().min_w_0().flex().flex_col().gap_2().child(label("BODY")).child(Textarea::new(&self.request_body).h(px(136.)).aria_label("Request body"))))
            .child(div().flex().items_center().justify_between().child(label("RESPONSE")).child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(self.response_summary.clone())))
            .child(Textarea::new(&self.response).h(px(320.)).readonly(true).aria_label("Response"))
            .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("Requests stay in memory. Website cookies are separate. Redirects are shown without following them."))
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = &self.tabs[self.active];
        let content = if self.lab {
            self.request_lab(cx)
        } else if let Some(page) = &tab.page {
            let page = page.clone();
            div()
                .flex_1()
                .min_h_0()
                .child(
                    canvas(
                        move |bounds, window, _| page.place(bounds, window.scale_factor()),
                        |_, _, _, _| {},
                    )
                    .size_full(),
                )
                .into_any_element()
        } else if tab.saved.url == START {
            self.start_page(cx)
        } else {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_4()
                .child(
                    Icon::new(IconName::Moon)
                        .size(px(32.))
                        .text_color(rgb(MUTED)),
                )
                .child(div().text_size(px(21.)).child(if tab.error.is_some() {
                    "This page couldn’t open."
                } else {
                    "This tab is resting."
                }))
                .child(
                    div().text_size(px(12.)).max_w(px(540.)).child(
                        tab.error
                            .clone()
                            .unwrap_or_else(|| "Resume it when you’re ready.".into()),
                    ),
                )
                .child(
                    control("resume-tab", "Resume tab", IconName::RotateCw)
                        .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
                )
                .into_any_element()
        };
        div()
            .key_context("Browser")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(rgb(BG))
            .text_color(rgb(INK))
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
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.toolbar(cx))
                    .child(content)
                    .when_some(self.notice.clone(), |d, message| {
                        d.child(
                            div()
                                .h(px(32.))
                                .flex_shrink_0()
                                .px_4()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_t_1()
                                .border_color(rgb(LINE))
                                .bg(rgb(0xf0f2f7))
                                .text_size(px(11.))
                                .child(div().flex_1().truncate().child(message))
                                .child(
                                    control("dismiss-notice", "Dismiss", IconName::X)
                                        .w(px(28.))
                                        .h(px(26.))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.notice = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            )
    }
}

fn control(id: &'static str, title: &str, icon: IconName) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.to_string())
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .h(px(34.))
        .px_2()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.))
        .hover(|s| s.bg(rgb(0xe1e6f0)))
        .child(Icon::new(icon).size_4())
        .when(
            ![
                "back",
                "forward",
                "reload",
                "copy-url",
                "inspector",
                "dismiss-notice",
            ]
            .contains(&id),
            |d| d.child(title.to_string()),
        )
}
fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(text)
}
