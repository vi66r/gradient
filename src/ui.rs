use crate::*;

mod tools;

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

pub(crate) const SIDEBAR: f32 = 248.;
/// The compact sidebar: square cells, room for the traffic lights.
const COMPACT: f32 = 76.;
/// A tab's square cell in the compact sidebar.
const SQUARE: f32 = 44.;

thread_local! {
    /// The sidebar's width as last drawn, while it glides between sizes too.
    static SIDEBAR_WIDTH: std::cell::Cell<f32> = const { std::cell::Cell::new(SIDEBAR) };
}

/// The sidebar's width on screen now.
pub(crate) fn sidebar_width() -> f32 {
    SIDEBAR_WIDTH.with(|width| width.get())
}
/// Matches the native page host's corner radius in engine.rs.
/// The page card sits below the toolbar and runs flush to the window's right
/// and bottom edges. Its top corners and its sidebar-facing bottom corner are
/// rounded; the window's own corner rounds the last one.
pub(crate) const CARD_RADIUS: f32 = 20.;
/// The titlebar row: the traffic lights over the sidebar, the toolbar over
/// the page card. Window chrome, in the palette's colors whatever the page.
pub(crate) const CONTROL_ZONE: f32 = 44.;
/// Radius scale. Controls (address field, tab cells, rows) share one radius,
/// standalone buttons a step smaller, and anything nested inside a control
/// uses the control's radius minus its inset, so curves stay concentric.
const R_CONTROL: f32 = 12.;
const R_BUTTON: f32 = 10.;
/// Height of the media controls a playing tab's sidebar cell grows by.
const MEDIA_STRIP: f32 = 30.;
/// The find bar's width when open; it eases open from nothing.
const FIND_BAR: f32 = 264.;
/// The suggestions list's radius: the pill's curve, a step tighter.
const R_LIST: f32 = 14.;
const FAST: Duration = Duration::from_millis(140);
const NORMAL: Duration = Duration::from_millis(200);

fn hsla(color: u32) -> Hsla {
    rgb(color).into()
}

impl Browser {
    /// The sidebar: full, or compact (square icons). It glides between the
    /// two widths.
    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let compact = self.settings.compact_sidebar;
        let width = motion::transition(
            "sidebar-width",
            if compact { COMPACT } else { SIDEBAR },
            Transition::new(NORMAL),
            window,
            cx,
        );
        SIDEBAR_WIDTH.with(|now| now.set(width));
        div()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .child(if compact {
                self.compact_sidebar(window, cx)
            } else {
                self.full_sidebar(window, cx)
            })
            .into_any_element()
    }

    fn full_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let access = self.agent_access(&self.tabs[self.active]);
        let grant = access.is_some();
        let agent_ink = agent_color(p, self.dark, access.as_ref().is_some_and(|a| a.trusted()));
        let lab = self.tool == Some(Tool::Lab);
        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR))
            .flex_shrink_0()
            .h_full()
            // The traffic lights' row, with the toggle at the tab cells'
            // right edge.
            .child(
                div()
                    .id("sidebar-titlebar")
                    .h(px(CONTROL_ZONE))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_end()
                    .on_mouse_down(MouseButton::Left, move_window)
                    .child(
                        icon_button(
                            "sidebar-toggle",
                            "Collapse Sidebar",
                            IconName::PanelLeftClose,
                            p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
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
                    .children((0..self.tabs.len()).map(|index| self.tab_row(index, p, window, cx)))
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                        this.space_swipe(event, window, cx)
                    }))
                    // A Space's tabs arrive together, like a page does, sliding
                    // in from the side the Space sits on.
                    .relative()
                    .with_animation(
                        ("space-tabs", self.space_epoch),
                        Animation::new(NORMAL).with_easing(ease_out_quint()),
                        {
                            let slide = self.space_slide;
                            move |list, delta| {
                                list.opacity(0.35 + 0.65 * delta)
                                    .left(px(16. * slide * (1. - delta)))
                            }
                        },
                    ),
            )
            // Spaces, below the tabs, in a section of their own.
            .child(self.space_switcher(p, cx))
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
                                    .aria_selected(lab)
                                    .flex_1()
                                    .h(px(32.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(px(R_CONTROL))
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .text_color(rgb(if lab { p.ink } else { p.muted }))
                                    .when(lab, |d| d.bg(rgb(p.selected)).shadow_xs())
                                    .when(!lab, |d| {
                                        d.hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
                                    })
                                    .child(Icon::new(IconName::Terminal).size_4())
                                    .child("Request Lab")
                                    .on_click(cx.listener(|this, _, w, cx| this.show_tool(Tool::Lab, w, cx))),
                            )
                            .child(self.tool_button(Tool::History, p, cx))
                            .child(self.tool_button(Tool::Settings, p, cx))
                            .child(
                                icon_button(
                                    "agent-access",
                                    match &access {
                                        Some(access) if access.trusted() => {
                                            "Agents have full access to this site. Click to stop trusting it."
                                        }
                                        Some(_) => "Agents can see this tab. Click to stop sharing.",
                                        None => "Share this tab with agents",
                                    },
                                    IconName::Bot,
                                    p,
                                )
                                .size(px(32.))
                                .rounded(px(R_CONTROL))
                                .when(grant, |d| {
                                    d.bg(rgba((agent_ink << 8) | 0x24)).text_color(rgb(agent_ink))
                                })
                                .on_click(cx.listener(|this, _, w, cx| this.toggle_agent(w, cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// History or Settings in the footer, lit while it's in the card.
    fn tool_button(&self, tool: Tool, p: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let (id, title, icon) = match tool {
            Tool::History => ("show-history", "History", IconName::Clock),
            _ => ("show-settings", "Settings", IconName::Settings),
        };
        let open = self.tool == Some(tool);
        icon_button(id, title, icon, p)
            .size(px(32.))
            .rounded(px(R_CONTROL))
            .aria_selected(open)
            .when(open, |d| {
                d.bg(rgb(p.selected)).text_color(rgb(p.ink)).shadow_xs()
            })
            .on_click(cx.listener(move |this, _, w, cx| this.show_tool(tool, w, cx)))
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
                .tooltip(placed(move || Tooltip::new(tip.clone())))
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

    /// Spaces as a row of chips between the tabs and the tools: each with its color and name,
    /// the current one filled, and a button for a new one at the end. When
    /// the names don't all fit, the other Spaces shrink to their color dots
    /// (named in a tooltip); past that the row scrolls sideways. Right-click
    /// a Space for its menu; double-click the current one to rename it.
    fn space_switcher(&self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let wash = if self.dark { 0xffffff14 } else { 0x0000000d };
        let saved = self.spaces.iter().filter(|space| !space.private).count();
        // Roughly: chip padding, dot and gap, plus the name at this size.
        let named = |space: &spaces::Space| 37. + space.name.chars().count().min(16) as f32 * 7.;
        let room = SIDEBAR - 16. - 32.;
        let compact = self.spaces.iter().map(named).sum::<f32>()
            + 4. * self.spaces.len().saturating_sub(1) as f32
            > room;
        let chips = self.spaces.iter().enumerate().map(|(index, space)| {
            let current = index == self.space;
            let dot_only = compact && !current;
            let color = space_color(space, self.dark);
            let id = space.id;
            let shortcut = (index < 9 && !space.private).then(|| format!("⌃{}", index + 1));
            // Named in the tooltip only when the chip doesn't show the name.
            let tip = match (dot_only, shortcut) {
                (true, Some(shortcut)) => Some(format!("{}  {shortcut}", space.name)),
                (true, None) => Some(space.name.clone()),
                (false, shortcut) => shortcut,
            };
            let mark = if space.private {
                Icon::new(IconName::Lock)
                    .size_3()
                    .text_color(rgb(color))
                    .into_any_element()
            } else {
                div()
                    .size(px(8.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(rgb(color))
                    .into_any_element()
            };
            let label = if current && self.renaming {
                div()
                    .w(px(96.))
                    .min_w_0()
                    .child(
                        Input::new(&self.space_name)
                            .appearance(false)
                            .aria_label("Space name")
                            .h(px(24.))
                            .py_0()
                            .px_0()
                            .text_size(px(12.5)),
                    )
                    .into_any_element()
            } else {
                div()
                    .min_w_0()
                    .max_w(px(112.))
                    .truncate()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(space.name.clone())
                    .into_any_element()
            };
            div()
                .id(("space", index))
                .role(Role::Tab)
                .aria_label(space.name.clone())
                .aria_selected(current)
                .h(px(28.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .when(dot_only, |chip| chip.w(px(28.)).justify_center())
                .when(!dot_only, |chip| chip.gap(px(7.)).pl(px(10.)).pr(px(12.)))
                .rounded_full()
                .cursor_pointer()
                .text_color(rgb(if current { p.ink } else { p.muted }))
                .when(current, |chip| chip.bg(rgba(wash)))
                .when(!current, |chip| {
                    chip.hover(move |s| s.bg(rgba(wash)).text_color(rgb(p.ink)))
                })
                .when_some(tip, |chip, tip| {
                    let tip: SharedString = tip.into();
                    chip.tooltip(placed(move || Tooltip::new(tip.clone())))
                })
                .child(mark)
                .when(!dot_only, |chip| chip.child(label))
                // A tab dropped here moves to this Space (and reloads there).
                .when(!current && !space.private, |chip| {
                    chip.drag_over::<DraggedTab>(move |style, _, _, _| {
                        style.bg(rgba(wash)).text_color(rgb(p.ink))
                    })
                    .on_drop(cx.listener(
                        move |this, dragged: &DraggedTab, window, cx| {
                            this.dragging = None;
                            this.request_discard(
                                lifecycle::Discard::MoveTab {
                                    id: dragged.id,
                                    space: id,
                                },
                                window,
                                cx,
                            );
                        },
                    ))
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.space_menu(id, event.position, window, cx);
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if index != this.space {
                        this.switch_space(index, window, cx);
                    } else if event.click_count() == 2 {
                        this.begin_rename(window, cx);
                    }
                }))
                // A click anywhere else keeps the name, as in Finder; empty
                // areas don't take focus, so blur alone can miss it.
                .when(current && self.renaming, |chip| {
                    chip.on_mouse_down_out(
                        cx.listener(|this, _, window, cx| this.finish_rename(true, window, cx)),
                    )
                })
                .into_any_element()
        });
        div()
            .id("spaces")
            .flex_shrink_0()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("space-row")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.space_scroll)
                    .children(chips),
            )
            // Outside the scrolling row, so it's always within reach.
            .when(saved < model::MAX_SPACES, |section| {
                section.child(
                    div()
                        .id("new-space")
                        .role(Role::Button)
                        .aria_label("New Space")
                        .size(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .text_color(rgb(p.muted))
                        .hover(move |s| s.bg(rgba(wash)).text_color(rgb(p.ink)))
                        .tooltip(placed(|| Tooltip::new("New Space")))
                        .child(Icon::new(IconName::Plus).size_3p5())
                        .on_click(cx.listener(|this, _, window, cx| this.new_space(window, cx))),
                )
            })
    }

    /// The window ground: the shell's chrome veil, tinted toward the current
    /// Space's color so each Space is recognizable at a glance.
    fn space_veil(&self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Hsla {
        let space = self.current_space();
        let amount = match (space.private, space.color, self.dark) {
            (true, _, true) => 0.22,
            (true, _, false) => 0.16,
            (false, 0, _) => 0.,
            (false, _, true) => 0.14,
            (false, _, false) => 0.10,
        };
        let target = motion::Interpolate::interpolate(
            &Rgb::from(p.chrome),
            &Rgb::from(space_color(space, self.dark)),
            amount,
        );
        let Rgb([r, g, b]) =
            motion::transition("space-veil", target, Transition::new(NORMAL), window, cx);
        let alpha = if self.dark { 0x59 } else { 0x40 };
        gpui::Rgba {
            r,
            g,
            b,
            a: alpha as f32 / 255.,
        }
        .into()
    }

    fn tab_row(
        &self,
        index: usize,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tab = &self.tabs[index];
        let selected = index == self.active && self.tool.is_none();
        let background = motion::transition(
            format!("tab-bg-{}", tab.id),
            hsla(if selected { p.selected } else { p.chrome }),
            Transition::new(NORMAL),
            window,
            cx,
        );
        let title = tab_title(tab);
        let icon = self.tab_icon(tab, selected, p);
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
            // Shared with agents: says so in the tab itself; a click stops it.
            // Open to agents: says so in the tab itself; a click stops it.
            .when_some(self.agent_access(tab), |header, access| {
                let trusted = access.trusted();
                header.child(
                    div()
                        .id(("tab-shared", tab.id))
                        .role(Role::Button)
                        .aria_label(if trusted {
                            "Stop trusting agents on this site"
                        } else {
                            "Stop sharing with agents"
                        })
                        .size(px(22.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(R_CONTROL - 4.))
                        .text_color(rgb(agent_color(p, self.dark, trusted)))
                        .hover(|s| s.bg(rgb(p.hover)))
                        .tooltip(placed(move || {
                            Tooltip::new(if trusted {
                                "Trusted for agents: full access. Click to stop trusting."
                            } else {
                                "Shared with agents. Click to stop sharing."
                            })
                        }))
                        .child(Icon::new(IconName::Bot).size_3p5())
                        .on_click(cx.listener(move |this, _, w, cx| {
                            cx.stop_propagation();
                            this.toggle_agent_for(index, w, cx);
                        })),
                )
            })
            // Pops the page out into its own window; ✕'s neighbor, shown
            // as ✕ is.
            .when(!tab.private && tab.saved.url != START, |header| {
                header.child(
                    div()
                        .id(("pop-out-tab", tab.id))
                        .role(Role::Button)
                        .aria_label(format!("Open {title} in an app window"))
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
                        .tooltip(placed(|| Tooltip::new("Open in App Window")))
                        .child(Icon::default().path(crate::assets::POP_OUT).size_3())
                        .on_click(cx.listener(move |this, _, w, cx| {
                            cx.stop_propagation();
                            this.pop_out(index, w, cx);
                        })),
                )
            })
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
        let row = div()
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
            .on_aux_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                if event.is_middle_click() {
                    this.close(index, w, cx);
                }
            }));
        self.reorderable(row, tab.id, title.clone(), p, cx)
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

    fn start_page(&self, _: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let private = self.tabs[self.active].private;
        let shortcuts = if cfg!(target_os = "macos") {
            "⌘L  Address      ⌘T  New Tab      ⌘Y  History"
        } else {
            "Ctrl L  Address      Ctrl T  New Tab      Ctrl Y  History"
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .pb(px(96.))
            .when(private, |page| {
                page.child(
                    Icon::new(IconName::Lock)
                        .size(px(24.))
                        .text_color(rgb(p.muted)),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Private Tab"),
                )
            })
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(p.muted))
                    .child(if private {
                        "Its history and website data aren’t saved."
                    } else {
                        "Search or enter an address above."
                    }),
            )
            .child(
                div()
                    .mt_3()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .opacity(0.8)
                    .child(shortcuts),
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
                        "This tab is suspended."
                    }),
            )
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(rgb(p.muted))
                    .child(tab.error.clone().unwrap_or_else(|| {
                        "It isn’t using memory. Resuming reloads the page.".into()
                    })),
            )
            .child(
                div().mt_3().child(
                    pill_button(
                        "resume-tab",
                        if tab.error.is_some() {
                            "Try Again"
                        } else {
                            "Resume Tab"
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
        // Every change to the tabs ends in a render, so extensions hear of
        // them here; unchanged tabs cost a comparison.
        self.sync_extension_tabs();
        self.sync_menus(cx);
        self.sync_apps(cx);
        let p = palette(self.dark);
        // A card open to agents glows: accent when shared, amber when its
        // site is trusted (see `engine::Page::set_shared`). Trusted pages
        // also keep the console and network logs agents can read.
        let pulse = !cx.reduce_motion();
        for tab in self.all_tabs() {
            if let Some(page) = &tab.page {
                let access = self.agent_access(tab);
                let trusted = access.as_ref().is_some_and(|access| access.trusted());
                page.set_shared(access.map(|_| (agent_color(p, self.dark, trusted), pulse)));
                page.set_agent_trust(trusted);
            }
        }
        let reveal = motion::transition(
            "suggestions",
            if self.suggest.shown() { 1. } else { 0. },
            Transition::new(FAST),
            window,
            cx,
        );
        // Once the list has faded out, the live page takes its picture's place.
        if !self.suggest.open && reveal < 0.01 {
            self.suggest.cover = suggest::Cover::None;
        }
        let tool_view = self.tool.map(|tool| -> (AnyElement, SharedString) {
            match tool {
                Tool::Lab => (self.request_lab(cx), "lab".into()),
                Tool::History => (self.history_view(window, cx), "history".into()),
                Tool::Settings => (self.settings_view(window, cx), "settings".into()),
            }
        });
        let tab = &self.tabs[self.active];
        let covered = match &self.suggest.cover {
            suggest::Cover::Ready(id, image) if *id == tab.id && self.tool.is_none() => {
                Some(image.clone())
            }
            _ => None,
        };
        let (content, key): (AnyElement, SharedString) = if let Some(view) = tool_view {
            view
        } else if let Some(image) = covered {
            (
                self.page_picture(image, reveal, cx),
                format!("page-{}", tab.id).into(),
            )
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
        let body = if self.tool.is_none() && tab.page.is_some() {
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
            .bg(self.space_veil(p, window, cx))
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
            .on_action(cx.listener(|this, _: &NewSpace, w, cx| this.new_space(w, cx)))
            .on_action(cx.listener(|this, _: &RenameSpace, w, cx| this.begin_rename(w, cx)))
            .on_action(cx.listener(|this, _: &DeleteSpace, w, cx| {
                let id = this.current_space().id;
                this.delete_space(id, w, cx)
            }))
            .on_action(cx.listener(|this, _: &NextSpace, w, cx| this.switch_space_by(1, w, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousSpace, w, cx| this.switch_space_by(-1, w, cx)),
            )
            .on_action(cx.listener(|this, action: &spaces::SwitchSpace, w, cx| {
                this.switch_space(action.index, w, cx)
            }))
            .on_action(cx.listener(|this, action: &spaces::SpaceMenu, w, cx| {
                this.space_command(action, w, cx)
            }))
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
                    if this.renaming {
                        this.finish_rename(false, w, cx);
                        return;
                    }
                    if this.find.read(cx).focus_handle(cx).is_focused(w) {
                        this.close_find(w, cx);
                    } else if this.address_focused(w, cx) && this.suggest.open {
                        // First Esc closes the list; the next leaves the field.
                        this.drop_completion(w, cx);
                        this.close_suggestions();
                        cx.notify();
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
            .on_action(cx.listener(|this, _: &RequestLab, w, cx| this.show_tool(Tool::Lab, w, cx)))
            .on_action(
                cx.listener(|this, _: &ShowHistory, w, cx| this.show_tool(Tool::History, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowSettings, w, cx| this.show_tool(Tool::Settings, w, cx)),
            )
            .on_action(
                cx.listener(|this, action: &history_view::ClearRange, w, cx| {
                    this.clear_history_range(action.range, w, cx)
                }),
            )
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
            .on_action(cx.listener(|this, _: &SitePermissions, w, cx| this.site_permissions(w, cx)))
            .on_action(cx.listener(|this, _: &AddExtension, w, cx| this.add_extension(w, cx)))
            .on_action(
                cx.listener(|this, action: &extensions::ExtensionMenu, w, cx| {
                    this.extension_command(action, w, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleAgent, w, cx| this.toggle_agent(w, cx)))
            .on_action(cx.listener(|this, _: &Quit, w, cx| {
                this.request_discard(lifecycle::Discard::Quit, w, cx);
            }))
            .on_action(cx.listener(|this, _: &ClearHistory, w, cx| this.clear_history(w, cx)))
            .on_action(cx.listener(|this, _: &PopOut, w, cx| this.pop_out(this.active, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|_, _: &menu_bar::About, _, _| crate::engine::show_about()))
            .on_action(cx.listener(|_, _: &menu_bar::Hide, _, cx| cx.hide()))
            .on_action(cx.listener(|_, _: &menu_bar::HideOthers, _, cx| cx.hide_other_apps()))
            .on_action(cx.listener(|_, _: &menu_bar::ShowAll, _, cx| cx.unhide_other_apps()))
            .on_action(cx.listener(|_, _: &menu_bar::Minimize, w, _| w.minimize_window()))
            .on_action(cx.listener(|_, _: &menu_bar::ZoomWindow, w, _| w.zoom_window()))
            .on_action(
                cx.listener(|this, _: &menu_bar::EditUndo, w, cx| this.edit_undo(false, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &menu_bar::EditRedo, w, cx| this.edit_undo(true, w, cx)),
            )
            .on_action(cx.listener(|this, action: &menu_bar::SelectTab, w, cx| {
                this.select_tab(action.index, w, cx)
            }))
            .child(self.sidebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.control_zone(p, window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .rounded_tl(px(CARD_RADIUS))
                            .rounded_tr(px(CARD_RADIUS))
                            .rounded_bl(px(CARD_RADIUS))
                            .bg(rgb(p.card))
                            .border_l_1()
                            .border_t_1()
                            .border_color(panel_edge(self.dark))
                            .shadow(panel_shadow(self.dark))
                            .child(body)
                            .children(self.suggestion_list(p, reveal, cx))
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
                                            icon_button(
                                                "dismiss-notice",
                                                "Dismiss",
                                                IconName::X,
                                                p,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.notice = None;
                                                    cx.notify();
                                                }),
                                            ),
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
    /// The toolbar's solid surface (for the suggestions list), its ink, and
    /// whether that ink is light. The toolbar is chrome over the Space's
    /// frosted veil, so it never takes the page's color.
    fn toolbar_look(&self, p: Palette) -> (u32, u32, bool) {
        (p.card, p.ink, self.dark)
    }

    /// The toolbar in the titlebar row, above the page card: navigation, a
    /// borderless address and page actions. Empty space drags the window
    /// (double-click zooms); controls keep their clicks to themselves.
    fn control_zone(
        &self,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tab = &self.tabs[self.active];
        let (background, ink, over_dark) = self.toolbar_look(p);
        // The theme's caret is a fixed primary; the toolbar's ink reads on
        // the veil in every Space color.
        let caret: Hsla = rgb(ink).into();
        if cx.theme().caret != caret {
            Theme::global_mut(cx).caret = caret;
        }
        // Hover washes that work on any Space color.
        let wash = if over_dark { 0xffffff1f } else { 0x0000000f };
        let live = tab.page.is_some() && self.tool.is_none();
        let addressed = tab.saved.url != START;
        let copied = self.copy_feedback == Some(CopyTarget::Address);
        div()
            .id("control-zone")
            .h(px(CONTROL_ZONE))
            .flex_shrink_0()
            .relative()
            .px_2()
            .flex()
            .items_center()
            .gap_0p5()
            .text_color(rgb(ink))
            // The compact sidebar's traffic lights leave it no room for its
            // toggle, so it leads the toolbar.
            .when(self.settings.compact_sidebar, |bar| {
                bar.child(
                    toolbar_button(
                        "sidebar-expand",
                        "Expand Sidebar",
                        IconName::PanelLeftOpen,
                        wash,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
            })
            .child(
                toolbar_button("back", "Back", IconName::ChevronLeft, wash)
                    .on_click(cx.listener(|this, _, _, _| this.history(false))),
            )
            .child(
                toolbar_button("forward", "Forward", IconName::ChevronRight, wash)
                    .on_click(cx.listener(|this, _, _, _| this.history(true))),
            )
            .child(
                toolbar_button("reload", "Reload", IconName::RotateCw, wash)
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
                        .tooltip(placed(|| {
                            Tooltip::new("Actual size").action(&ActualSize, None)
                        }))
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
                    toolbar_button(
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
                    toolbar_button("inspector", "Inspect page", IconName::Code, wash).on_click(
                        cx.listener(|this, _, _, _| {
                            if let Some(page) = &this.tabs[this.active].page {
                                page.raw.open_devtools();
                            }
                        }),
                    ),
                )
            })
            .when(self.tool.is_none(), |strip| {
                strip.children(self.extension_buttons(wash, ink, background, cx))
            })
            .when(self.tool.is_none() && self.has_extensions(), |strip| {
                strip.child(
                    toolbar_button("extensions", "Extensions", IconName::Puzzle, wash).on_click(
                        cx.listener(|this, event: &ClickEvent, window, cx| {
                            this.extensions_menu(event.position(), window, cx)
                        }),
                    ),
                )
            })
            .when(tab.loading && self.tool.is_none(), |strip| {
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

    /// Extensions' toolbar buttons: their own icons at full color, a
    /// disabled one faded, and a badge as a small pill in the toolbar's ink.
    /// A click runs the action (or opens its popup beneath the button); a
    /// right-click opens the extension's menu.
    fn extension_buttons(
        &self,
        wash: u32,
        ink: u32,
        background: u32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.extension_actions
            .iter()
            .map(|action| {
                let tip: SharedString = action.label.clone().into();
                let (clicked, menu, anchor) = (
                    action.install.clone(),
                    action.install.clone(),
                    action.install.clone(),
                );
                let anchors = self.extension_anchors.clone();
                let icon = match &action.icon {
                    Some(icon) => img(icon.clone()).size(px(16.)).into_any_element(),
                    None => Icon::new(IconName::Puzzle).size_4().into_any_element(),
                };
                let badge: String = action.badge.chars().take(4).collect();
                div()
                    .id(SharedString::from(format!("extension-{}", action.install)))
                    .role(Role::Button)
                    .aria_label(action.label.clone())
                    .relative()
                    .size(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(R_BUTTON))
                    .tooltip(placed(move || Tooltip::new(tip.clone())))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.extension_menu(&menu, event.position, window, cx)
                        }),
                    )
                    .when(action.enabled, |button| {
                        button
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgba(wash)))
                            .active(|s| s.opacity(0.7))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.perform_extension_action(&clicked);
                                cx.notify();
                            }))
                    })
                    .when(!action.enabled, |button| button.opacity(0.4))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                anchors.borrow_mut().insert(anchor.clone(), bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .child(icon)
                    .when(!badge.is_empty(), |button| {
                        button.child(
                            div()
                                .absolute()
                                .top(px(2.))
                                .right(px(0.))
                                .h(px(12.))
                                .min_w(px(12.))
                                .px(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rgb(ink))
                                .text_color(rgb(background))
                                .text_size(px(8.5))
                                .line_height(px(12.))
                                .font_weight(FontWeight::BOLD)
                                .child(badge),
                        )
                    })
                    .into_any_element()
            })
            .collect()
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
        let open = self.finding == Some(self.tabs[self.active].id) && self.tool.is_none();
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
            toolbar_control(id, title, icon, wash, 22.).rounded_full()
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

    /// The address, filling the toolbar between the buttons. At rest it is
    /// just the site's icon and host on the toolbar itself, left-aligned,
    /// with no field around it. Editing lifts a glass pill out of the toolbar
    /// (fill, hairline and shadow ease in together) while the content
    /// cross-fades, and both states share one layout so nothing jumps.
    /// Dragging the resting address moves the window, since it covers most
    /// of the toolbar's drag area.
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
            let bounds = self.address_bounds.clone();
            return field
                .relative()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // ↑ and ↓ move through the suggestions instead of the text.
                .capture_action(
                    cx.listener(|this, _: &gpui::component::input::MoveUp, w, cx| {
                        if this.suggest.open {
                            this.move_suggestion(false, w, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
                .capture_action(
                    cx.listener(|this, _: &gpui::component::input::MoveDown, w, cx| {
                        if this.suggest.open {
                            this.move_suggestion(true, w, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
                .child(
                    canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
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

impl Browser {
    /// The page's picture while the suggestions cover it (`suggest::Cover`),
    /// with the card's rounded corners, as the live page has. A faint veil sets the list above it. Clicking
    /// it puts the list away and hands focus back to the page.
    fn page_picture(
        &self,
        image: Option<Arc<RenderImage>>,
        reveal: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let radius = px(CARD_RADIUS - 1.);
        let veil = if self.dark { 0.14 } else { 0.035 };
        div()
            .id("page-picture")
            .size_full()
            .relative()
            .overflow_hidden()
            .rounded_tl(radius)
            .rounded_tr(radius)
            .rounded_bl(radius)
            .children(image.map(|image| {
                img(image)
                    .size_full()
                    .rounded_tl(radius)
                    .rounded_tr(radius)
                    .rounded_bl(radius)
            }))
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded_tl(radius)
                    .rounded_tr(radius)
                    .rounded_bl(radius)
                    .bg(black(veil * reveal)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.release_address(window, cx);
                }),
            )
            .into_any_element()
    }

    /// The suggestions, hung just below the address pill and as wide: the
    /// pill's glass made solid, with a soft lift. It
    /// eases in and out with a small drop, over the page's picture.
    fn suggestion_list(
        &self,
        p: Palette,
        reveal: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if reveal < 0.01 || self.suggest.rows.is_empty() {
            return None;
        }
        let anchor = self.address_bounds.get();
        let (background, ink, over_dark) = self.toolbar_look(p);
        let glass = Glass { over_dark };
        let terms = suggest::terms(&self.suggest.typed);
        let (wash, hover) = if over_dark {
            (white(0.11), white(0.06))
        } else {
            (black(0.06), black(0.035))
        };
        let rows = self
            .suggest
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.suggestion_row(index, row, &terms, wash, hover, cx));
        let list = div()
            .id("suggestions")
            .occlude()
            .w(anchor.size.width)
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(R_LIST))
            .bg(glass.solid(background))
            .border_1()
            .border_color(glass.edge(1.))
            .shadow(glass.drop_shadow(reveal))
            .text_color(rgb(ink))
            .text_size(px(13.))
            .opacity(reveal)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(rows);
        let at = point(anchor.left(), anchor.bottom() + px(6. - 4. * (1. - reveal)));
        Some(deferred(anchored().position(at).child(list)).into_any_element())
    }

    fn suggestion_row(
        &self,
        index: usize,
        row: &suggest::Row,
        terms: &[String],
        wash: Hsla,
        hover: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        use suggest::Look;
        let selected = index == self.suggest.selected;
        // Private pages never show site icons.
        let site_icon = row
            .url
            .as_deref()
            .filter(|_| !self.current_space().private)
            .and_then(favicon::host_key)
            .and_then(|key| self.icons.get(&key))
            .map(|(image, _)| image.clone());
        let icon = match site_icon {
            Some(image) => img(image)
                .size(px(16.))
                .flex_shrink_0()
                .rounded(px(4.))
                .into_any_element(),
            None => Icon::new(match row.look {
                Look::Search => IconName::Search,
                Look::Tab => IconName::AppWindow,
                Look::Page | Look::Address => IconName::Globe,
            })
            .size_4()
            .flex_shrink_0()
            .opacity(0.6)
            .into_any_element(),
        };
        // Matches stand out in pages and tabs; a search or address row is
        // the typed text itself.
        let matched = matches!(row.look, Look::Page | Look::Tab);
        let text = |text: &str| emphasized(text, if matched { terms } else { &[] });
        div()
            .id(("suggestion", index))
            .h(px(34.))
            .px_2p5()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded(px(R_LIST - 4.))
            .cursor_pointer()
            .when(selected, |row| row.bg(wash))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(relative(0.7))
                            .truncate()
                            .child(text(&row.title)),
                    )
                    .when(!row.detail.is_empty(), |line| {
                        line.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .opacity(0.55)
                                .child(text(&row.detail)),
                        )
                    }),
            )
            .when(row.look == Look::Tab, |line| {
                line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.5))
                        .opacity(if selected { 0.7 } else { 0.5 })
                        .child("Switch to Tab"),
                )
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.choose_suggestion(index, window, cx)),
            )
    }
}

/// `text` with each of `terms` (lowercase) set a weight heavier where it
/// matched (see `suggest::find_term`).
fn emphasized(text: &str, terms: &[String]) -> StyledText {
    let lower = text.to_ascii_lowercase();
    let mut ranges: Vec<std::ops::Range<usize>> = terms
        .iter()
        .filter_map(|term| {
            let start = suggest::find_term(&lower, term)?;
            Some(start..start + term.len())
        })
        .filter(|range| text.is_char_boundary(range.start) && text.is_char_boundary(range.end))
        .collect();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    let bold = HighlightStyle {
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    StyledText::new(text.to_string()).with_highlights(merged.into_iter().map(|range| (range, bold)))
}

/// Translucent layers for the address pill, tuned to the chrome beneath it:
/// white glass over dark chrome, frosted white over light. `lift` runs
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

    /// The glass as one opaque color over `under`, for surfaces that leave
    /// the toolbar (the suggestions list).
    fn solid(self, under: u32) -> Hsla {
        let alpha = if self.over_dark { 0.12 } else { 0.72 };
        motion::Interpolate::interpolate(&Rgb::from(under), &Rgb::from(0xffffff), alpha).into()
    }

    /// A softer, deeper lift than the pill's, for a surface over the page.
    fn drop_shadow(self, lift: f32) -> Vec<BoxShadow> {
        let depth = if self.over_dark { 1. } else { 0.5 };
        let layer = |alpha: f32, y: f32, blur: f32| BoxShadow {
            color: black(alpha * depth * lift),
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(0.),
            inset: false,
        };
        vec![layer(0.12, 1., 3.), layer(0.2, 12., 32.)]
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

/// A control in the toolbar: inherits the toolbar's ink, uses a translucent
/// wash on hover, and never starts a window drag.
fn toolbar_button(id: &'static str, title: &str, icon: IconName, wash: u32) -> Stateful<Div> {
    toolbar_control(id, title, icon, wash, 28.)
}

/// `toolbar_button` at another size; smaller ones nest inside pills.
fn toolbar_control(
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
        .tooltip(placed(move || Tooltip::new(tip.clone())))
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

/// Whether two page colors look the same. Later samples of a page often
/// differ by a step or two; repainting the backdrop for that is wasted work.
pub(crate) fn same_color(a: Option<u32>, b: Option<u32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => [16, 8, 0]
            .iter()
            .all(|shift| ((a >> shift) & 0xff).abs_diff((b >> shift) & 0xff) <= 4),
        (a, b) => a == b,
    }
}

/// Agents' color: the accent for a shared tab, amber for a trusted site.
fn agent_color(p: Palette, dark: bool, trusted: bool) -> u32 {
    match (trusted, dark) {
        (false, _) => p.accent,
        (true, false) => 0xd97706,
        (true, true) => 0xf59e0b,
    }
}

/// Relative luminance of a `0xRRGGBB` color, 0 (black) to 1 (white), for
/// choosing ink that reads on it.
pub(crate) fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xff) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// The color behind a page: its own tint, else the card's.
pub(crate) fn page_color(tint: Option<u32>, dark: bool) -> u32 {
    tint.unwrap_or(palette(dark).card)
}

/// A light veil of our palette over the system sidebar material, so the frost
/// keeps the shell's tone while the desktop still shows through.
/// Space colors as (name, light, dark). Gray is the untinted default.
const SPACE_COLORS: [(&str, u32, u32); model::SPACE_COLORS as usize] = [
    ("Gray", 0x8b919c, 0x8b919c),
    ("Blue", 0x3b82f6, 0x60a5fa),
    ("Purple", 0x8b5cf6, 0xa78bfa),
    ("Pink", 0xec4899, 0xf472b6),
    ("Orange", 0xf97316, 0xfb923c),
    ("Yellow", 0xeab308, 0xfacc15),
    ("Green", 0x22c55e, 0x4ade80),
    ("Teal", 0x14b8a6, 0x2dd4bf),
];
/// The Private Space's own color, outside the user's choices.
const PRIVATE_COLOR: (u32, u32) = (0x6d5bd0, 0x8b7cf0);

pub(crate) fn space_color_name(color: u8) -> &'static str {
    SPACE_COLORS
        .get(color as usize)
        .map_or("Gray", |(name, _, _)| name)
}

fn space_color(space: &spaces::Space, dark: bool) -> u32 {
    if space.private {
        return if dark {
            PRIVATE_COLOR.1
        } else {
            PRIVATE_COLOR.0
        };
    }
    let (_, light, night) = SPACE_COLORS
        .get(space.color as usize)
        .copied()
        .unwrap_or(SPACE_COLORS[0]);
    if dark { night } else { light }
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
        .tooltip(placed(move || Tooltip::new(tip.clone())))
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

/// A tab being dragged in the sidebar.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub id: u64,
    title: SharedString,
    ink: u32,
    ground: u32,
}

/// What follows the pointer while a tab is dragged: the tab, lifted.
struct TabPreview {
    tab: DraggedTab,
    /// Where the tab was grabbed, so it stays under the pointer there.
    offset: Point<Pixels>,
}

impl Render for TabPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .ml(-self.offset.x)
            .mt(-self.offset.y)
            .w(px(SIDEBAR - 16.))
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(R_CONTROL))
            .bg(rgb(self.tab.ground))
            .shadow_lg()
            .opacity(0.95)
            .text_size(px(13.))
            .text_color(rgb(self.tab.ink))
            .font_family(if cfg!(target_os = "macos") {
                ".AppleSystemUIFont"
            } else {
                "Inter"
            })
            .child(div().min_w_0().truncate().child(self.tab.title.clone()))
    }
}

/// A square control in the compact sidebar. No tooltip: from a column this
/// narrow, one would open under the page.
fn square_button(id: &'static str, label: &str, icon: IconName, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_string())
        .size(px(36.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_CONTROL))
        .cursor_pointer()
        .text_color(rgb(p.muted))
        .hover(|s| s.bg(rgb(p.hover)).text_color(rgb(p.ink)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(Icon::new(icon).size_4())
}

/// A small round button on a corner of a compact tab's square.
fn badge(id: (&'static str, u64), label: &str, icon: IconName, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_string())
        .absolute()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .bg(rgb(p.card))
        .border_1()
        .border_color(rgb(p.line))
        .text_color(rgb(p.muted))
        .hover(|s| s.text_color(rgb(p.ink)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(Icon::new(icon).size(px(10.)))
}

/// A tab's name in the sidebar.
fn tab_title(tab: &Tab) -> SharedString {
    // Start tabs saved before "New Tab" was title-cased say "New tab".
    if tab.saved.url == START {
        "New Tab".into()
    } else if tab.saved.title.is_empty() {
        model::display_host(&tab.saved.url).into()
    } else {
        tab.saved.title.clone().into()
    }
}

impl Browser {
    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.settings.compact_sidebar = !self.settings.compact_sidebar;
        if let Err(error) = self.settings.save(self.store.dir()) {
            self.notice = Some(format!("Setting not saved: {error}"));
        }
        cx.notify();
    }

    /// The compact sidebar: each tab a square with its icon, a corner badge
    /// to mute one that plays, and one to close on hover; then the Spaces as
    /// dots and the footer's tools, stacked.
    fn compact_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let access = self.agent_access(&self.tabs[self.active]);
        let agent_ink = agent_color(p, self.dark, access.as_ref().is_some_and(|a| a.trusted()));
        let lab = self.tool == Some(Tool::Lab);
        let lit = move |button: Stateful<Div>, on: bool| {
            button.when(on, |d| {
                d.bg(rgb(p.selected)).text_color(rgb(p.ink)).shadow_xs()
            })
        };
        let wash = if self.dark { 0xffffff14 } else { 0x0f172a0f };
        let squares: Vec<_> = (0..self.tabs.len())
            .map(|index| self.tab_square(index, p, window, cx))
            .collect();
        let spaces = self.spaces.iter().enumerate().map(|(index, space)| {
            let current = index == self.space;
            let id = space.id;
            let color = space_color(space, self.dark);
            let mark = if space.private {
                Icon::new(IconName::Lock)
                    .size_3()
                    .text_color(rgb(color))
                    .into_any_element()
            } else {
                div()
                    .size(px(8.))
                    .rounded_full()
                    .bg(rgb(color))
                    .into_any_element()
            };
            div()
                .id(("compact-space", index))
                .role(Role::Tab)
                .aria_label(space.name.clone())
                .aria_selected(current)
                .size(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .when(current, |chip| chip.bg(rgba(wash)))
                .when(!current, |chip| chip.hover(move |s| s.bg(rgba(wash))))
                .child(mark)
                .when(!current && !space.private, |chip| {
                    chip.drag_over::<DraggedTab>(move |style, _, _, _| style.bg(rgba(wash)))
                        .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                            this.dragging = None;
                            this.request_discard(
                                lifecycle::Discard::MoveTab {
                                    id: dragged.id,
                                    space: id,
                                },
                                window,
                                cx,
                            );
                        }))
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.space_menu(id, event.position, window, cx);
                    }),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.switch_space(index, window, cx)),
                )
        });
        div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(COMPACT))
            .flex_shrink_0()
            .h_full()
            .child(
                div()
                    .id("sidebar-titlebar")
                    .h(px(CONTROL_ZONE))
                    .w_full()
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, move_window),
            )
            .child(
                div()
                    .id("tab-list")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        square_button("new-tab", "Open New Tab", IconName::Plus, p)
                            .size(px(SQUARE))
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.new_tab(START.into(), false, w, cx)
                            })),
                    )
                    .children(squares)
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                        this.space_swipe(event, window, cx)
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .py_2()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .children(spaces)
                    .child(
                        square_button("new-space", "New Space", IconName::Plus, p)
                            .size(px(28.))
                            .rounded_full()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_space(window, cx)),
                            ),
                    ),
            )
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
                    .items_center()
                    .gap_1()
                    // Downloads need the full sidebar; this opens it.
                    .when(
                        self.downloads.iter().any(|download| download.active()),
                        |footer| {
                            footer.child(
                                square_button("downloads", "Downloads", IconName::Download, p)
                                    .text_color(rgb(p.accent))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    ),
                            )
                        },
                    )
                    .child(lit(
                        square_button("request-lab", "Request lab", IconName::Terminal, p)
                            .aria_selected(lab)
                            .on_click(
                                cx.listener(|this, _, w, cx| this.show_tool(Tool::Lab, w, cx)),
                            ),
                        lab,
                    ))
                    .child(lit(
                        square_button("show-history", "History", IconName::Clock, p).on_click(
                            cx.listener(|this, _, w, cx| this.show_tool(Tool::History, w, cx)),
                        ),
                        self.tool == Some(Tool::History),
                    ))
                    .child(lit(
                        square_button("show-settings", "Settings", IconName::Settings, p).on_click(
                            cx.listener(|this, _, w, cx| this.show_tool(Tool::Settings, w, cx)),
                        ),
                        self.tool == Some(Tool::Settings),
                    ))
                    .child(
                        square_button(
                            "agent-access",
                            "Share this tab with agents",
                            IconName::Bot,
                            p,
                        )
                        .when(access.is_some(), |d| {
                            d.bg(rgba((agent_ink << 8) | 0x24))
                                .text_color(rgb(agent_ink))
                        })
                        .on_click(cx.listener(|this, _, w, cx| this.toggle_agent(w, cx))),
                    ),
            )
            .into_any_element()
    }

    /// A tab in the compact sidebar: its icon in a square.
    fn tab_square(
        &self,
        index: usize,
        p: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tab = &self.tabs[index];
        let id = tab.id;
        let selected = index == self.active && self.tool.is_none();
        let background = motion::transition(
            format!("tab-bg-{id}"),
            hsla(if selected { p.selected } else { p.chrome }),
            Transition::new(NORMAL),
            window,
            cx,
        );
        let title = tab_title(tab);
        let group: SharedString = format!("tab-square-{id}").into();
        let media = tab
            .media
            .as_ref()
            .filter(|media| tab.page.is_some() && media.shown);
        let square = div()
            .id(("tab-square", id))
            .group(group.clone())
            .role(Role::Tab)
            .aria_selected(selected)
            .aria_label(title.clone())
            .relative()
            .size(px(SQUARE))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(R_CONTROL))
            .cursor_pointer()
            .bg(background)
            .when(selected, |d| d.shadow_xs())
            .when(!selected, |d| d.hover(|s| s.bg(rgb(p.hover))))
            .child(self.tab_icon(tab, selected, p))
            // The one media control here: mute, on a playing tab.
            .when_some(media, |square, media| {
                square.child(
                    badge(
                        ("mute-square", id),
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
                        p,
                    )
                    .bottom(px(-2.))
                    .right(px(-2.))
                    .when(media.muted, |d| d.text_color(rgb(p.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_media_muted(id, cx);
                    })),
                )
            })
            .child(
                badge(("close-square", id), "Close tab", IconName::X, p)
                    .top(px(-2.))
                    .right(px(-2.))
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        cx.stop_propagation();
                        this.close(index, w, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, w, cx| this.activate(index, w, cx)))
            .on_aux_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                if event.is_middle_click() {
                    this.close(index, w, cx);
                }
            }));
        self.reorderable(square, id, title, p, cx).on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.tab_menu(id, event.position, window, cx);
            }),
        )
    }

    /// A tab's icon: a spinner while loading, else its site's icon, else a
    /// symbol for what it is.
    fn tab_icon(&self, tab: &Tab, selected: bool, p: Palette) -> AnyElement {
        let site_icon = (!tab.private)
            .then(|| favicon::host_key(&tab.saved.url))
            .flatten()
            .and_then(|key| self.icons.get(&key))
            .map(|(image, _)| image.clone());
        let resting = tab.page.is_none() && tab.saved.url != START;
        if tab.loading {
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
            } else if resting {
                IconName::Moon
            } else if model::is_file_url(&tab.saved.url) {
                IconName::FileText
            } else {
                IconName::Globe
            })
            .size_4()
            .text_color(rgb(if selected { p.ink } else { p.muted }))
            .into_any_element()
        }
    }

    /// Lets a tab's cell (a row, or a square in the compact sidebar) be
    /// dragged to a new place: the list rearranges as it passes, and the
    /// tab's own place stays, faint.
    fn reorderable(
        &self,
        cell: Stateful<Div>,
        id: u64,
        title: SharedString,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let lifted = self.dragging == Some(id) && cx.has_active_drag();
        let preview = DraggedTab {
            id,
            title,
            ink: p.ink,
            ground: p.selected,
        };
        cell.when(lifted, |cell| cell.opacity(0.35))
            .on_drag(preview, |tab, offset, _, cx| {
                cx.new(|_| TabPreview {
                    tab: tab.clone(),
                    offset,
                })
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                    let position = event.event.position;
                    if !event.bounds.contains(&position) {
                        return;
                    }
                    let dragged = event.drag(cx).id;
                    this.dragging = Some(dragged);
                    this.drag_tab_over(dragged, id, position.y >= event.bounds.center().y, cx);
                }),
            )
            .on_drop(cx.listener(|this, _: &DraggedTab, _, cx| {
                this.dragging = None;
                this.save();
                cx.notify();
            }))
    }

    /// The dragged tab takes `over`'s place once the pointer crosses that
    /// tab's middle, so a small move doesn't make the two bounce.
    fn drag_tab_over(&mut self, dragged: u64, over: u64, lower_half: bool, cx: &mut Context<Self>) {
        if dragged == over {
            return;
        }
        let position = |id| self.tabs.iter().position(|tab| tab.id == id);
        let (Some(from), Some(to)) = (position(dragged), position(over)) else {
            return;
        };
        if (from < to && !lower_half) || (from > to && lower_half) {
            return;
        }
        let active = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self
            .tabs
            .iter()
            .position(|tab| tab.id == active)
            .unwrap_or(0);
        self.dirty = true;
        cx.notify();
    }
}

/// Builds a tooltip that keeps clear of the page card. The page is a native
/// view above everything GPUI draws, and GPUI opens every tooltip below and
/// right of the pointer, so near the card a tip opened underneath the page
/// and was cut off. Tips in the sidebar now open toward its middle (and
/// above the pointer in its Spaces row and footer); tips in the toolbar open beside the
/// pointer, centered on the bar.
pub(crate) fn placed(
    tooltip: impl Fn() -> Tooltip + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| {
        let tip = tooltip().build(window, cx);
        let (at, viewport) = (window.mouse_position(), window.viewport_size());
        cx.new(|_| Placed { tip, at, viewport }).into()
    }
}

/// A tooltip moved off GPUI's spot. GPUI lays it out at the pointer; this
/// root takes no room, and the tip hangs off it on the chosen side, so its
/// size needn't be known in advance. (`Tooltip` brings a 12 px margin.)
struct Placed {
    tip: AnyView,
    at: Point<Pixels>,
    viewport: Size<Pixels>,
}

impl Render for Placed {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (x, y) = (f32::from(self.at.x), f32::from(self.at.y));
        let (width, height) = (
            f32::from(self.viewport.width),
            f32::from(self.viewport.height),
        );
        let tip = div().absolute().whitespace_nowrap().child(self.tip.clone());
        let sidebar = sidebar_width();
        let tip = if x >= sidebar && y < CONTROL_ZONE {
            // The toolbar: beside the pointer, past the button it's on.
            let middle = CONTROL_ZONE / 2. - y;
            let tip = tip.top(px(middle - 30.)).h(px(60.)).flex().items_center();
            if x > width / 2. {
                tip.right(px(14.))
            } else {
                tip.left(px(14.))
            }
        } else {
            let leftward = if x < sidebar {
                x > sidebar / 2.
            } else {
                x > width - 240.
            };
            let tip = if leftward {
                tip.right_0()
            } else {
                tip.left_0()
            };
            // The sidebar's Spaces row and footer: opening below would
            // cover the buttons beneath.
            let bottom = if x < sidebar { 120. } else { 56. };
            if y > height - bottom {
                tip.bottom_0()
            } else {
                tip.top_0()
            }
        };
        div().size_0().child(tip)
    }
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
    fn tiny_sampling_differences_are_the_same_color() {
        assert!(same_color(Some(0x1f1f1f), Some(0x1e2021)));
        assert!(!same_color(Some(0x1f1f1f), Some(0x2a2a2a)));
        assert!(same_color(None, None));
        assert!(!same_color(None, Some(0xffffff)));
    }

    #[core::prelude::v1::test]
    fn colors_blend_in_rgb_not_through_other_hues() {
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
