//! History and Settings, drawn in the card in place of the page as the
//! Request Lab is. Settings groups what applies everywhere (default browser,
//! search) above what belongs to the Space on screen (history, website data,
//! site permissions, extensions), as each Space keeps its own.
use super::*;
use crate::extensions::{ExtensionCommand, ExtensionMenu};
use crate::history_view::{ClearRange, Item, Range};
use gpui::component::switch::Switch;

const ROW: f32 = 36.;

impl Browser {
    /// A tool's title, a line under it, its own buttons and Back to page.
    fn tool_header(
        &self,
        title: &'static str,
        subtitle: String,
        actions: Vec<AnyElement>,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(subtitle),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .gap_2()
                    .children(actions)
                    .child(
                        pill_button("back-to-page", "Back to page", IconName::ArrowLeft, p)
                            .on_click(
                                cx.listener(|this, _, w, cx| this.activate(this.active, w, cx)),
                            ),
                    ),
            )
    }

    pub(super) fn history_view(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let space = self.current_space();
        let (private, name) = (space.private, space.name.clone());
        let pages = self
            .history_items
            .iter()
            .filter(|item| matches!(item, Item::Page { .. }))
            .count();
        let searching = !self.history_query.trim().is_empty();
        let subtitle = if private {
            "The Private Space keeps no history.".to_string()
        } else {
            format!("Pages “{name}” visited. Other Spaces keep their own.")
        };
        let clear = pill_button("history-clear", "Clear…", IconName::Eraser, p)
            .on_click(cx.listener(|_, event: &ClickEvent, window, cx| {
                use gpui::component::native_menu::NativeMenu;
                let mut menu = NativeMenu::new();
                for range in Range::ALL {
                    menu = menu.menu(range.name(), Box::new(ClearRange { range }));
                }
                menu.show(event.position(), window, cx);
            }))
            .into_any_element();
        let header = self.tool_header(
            "History",
            subtitle,
            if private { Vec::new() } else { vec![clear] },
            p,
            cx,
        );
        let body = if private {
            div().into_any_element()
        } else if pages == 0 {
            div()
                .pt_10()
                .flex()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(p.muted))
                .child(if searching {
                    "No pages match."
                } else {
                    "Pages you visit in this Space show up here."
                })
                .into_any_element()
        } else {
            uniform_list(
                "history-list",
                self.history_items.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    this.load_history_icons(range.clone());
                    let p = palette(this.dark);
                    range
                        .map(|index| this.history_row(index, p, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.history_scroll)
            .flex_1()
            .min_h_0()
            .pb_4()
            .into_any_element()
        };
        div()
            .id("history")
            .size_full()
            .px_8()
            .pt_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .when(!private, |view| {
                view.child(Input::new(&self.history_search).aria_label("Search history"))
            })
            .child(body)
            .into_any_element()
    }

    fn history_row(&self, index: usize, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.history_items.get(index) else {
            return div().h(px(ROW)).into_any_element();
        };
        let Item::Page {
            url,
            title,
            detail,
            time,
        } = item
        else {
            let Item::Day(day) = item else { unreachable!() };
            return div()
                .h(px(ROW))
                .px_2()
                .pb_1()
                .flex()
                .items_end()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.muted))
                .child(day.clone())
                .into_any_element();
        };
        let icon = favicon::host_key(url)
            .and_then(|key| self.icons.get(&key))
            .map(|(image, _)| image.clone());
        let icon = match icon {
            Some(image) => img(image)
                .size(px(16.))
                .flex_shrink_0()
                .rounded(px(4.))
                .into_any_element(),
            None => Icon::new(IconName::Globe)
                .size_4()
                .flex_shrink_0()
                .text_color(rgb(p.muted))
                .into_any_element(),
        };
        let group: SharedString = format!("history-row-{index}").into();
        let terms = suggest::terms(&self.history_query);
        let (open, remove) = (url.clone(), url.clone());
        div()
            .id(("history-row", index))
            .group(group.clone())
            .role(Role::Link)
            .aria_label(title.clone())
            .w_full()
            .h(px(ROW))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(R_CONTROL))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(p.field)))
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
                            .max_w(relative(0.65))
                            .truncate()
                            .text_size(px(13.))
                            .child(emphasized(title, &terms)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(emphasized(detail, &terms)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(time.clone()),
            )
            .child(
                icon_button("history-remove", "Remove from History", IconName::X, p)
                    .id(("history-remove", index))
                    .size(px(24.))
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_history_page(&remove, cx);
                    })),
            )
            .on_click(cx.listener({
                let open = open.clone();
                move |this, event: &ClickEvent, w, cx| {
                    let new_tab = event.modifiers().platform;
                    this.open_history_page(open.clone(), new_tab, w, cx)
                }
            }))
            // Behind the current tab, as a middle-clicked link opens, so
            // History stays up for the next one.
            .on_aux_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                if event.is_middle_click() {
                    this.new_background_tab(open.clone(), false, w, cx);
                }
            }))
            .into_any_element()
    }

    pub(super) fn settings_view(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.dark);
        let space = self.current_space();
        let (private, name, id) = (space.private, space.name.clone(), space.id);
        let header = self.tool_header(
            "Settings",
            format!("Gradient {}", env!("CARGO_PKG_VERSION")),
            Vec::new(),
            p,
            cx,
        );
        let mut sections = vec![
            section("General", None, vec![self.default_browser_row(p, cx)], p),
            section("Search", None, self.search_rows(p, cx), p),
        ];
        let space_note = format!("Each Space keeps its own. These belong to “{name}”.");
        if private {
            sections.push(section(
                "Private Space",
                Some("History, website data and permissions are never kept here: they’re gone with the last private tab.".into()),
                Vec::new(),
                p,
            ));
        } else {
            sections.push(section(
                "Privacy",
                Some(space_note),
                self.privacy_rows(id, p, cx),
                p,
            ));
            sections.push(section(
                "Site Permissions",
                None,
                self.permission_rows(p, cx),
                p,
            ));
            sections.push(section(
                "Extensions",
                None,
                self.extension_rows(id, p, cx),
                p,
            ));
        }
        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_6()
            .child(
                div()
                    .max_w(px(680.))
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(header)
                    .children(sections),
            )
            .into_any_element()
    }

    fn default_browser_row(&self, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let control = match engine::is_default_browser() {
            Some(true) => div()
                .flex()
                .items_center()
                .gap_1p5()
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(Icon::new(IconName::Check).size_3p5())
                .child("Gradient is your default")
                .into_any_element(),
            Some(false) => pill_button("make-default", "Make Default", IconName::Globe, p)
                .on_click(
                    cx.listener(|this, _, _, _| engine::make_default_browser(this.events.clone())),
                )
                .into_any_element(),
            None => div()
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child("Only the installed app can be the default")
                .into_any_element(),
        };
        setting_row(
            "Default browser",
            Some("Links from other apps open in Gradient.".into()),
            control,
            p,
        )
    }

    fn search_rows(&self, p: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.settings.search;
        let engines = div()
            .flex()
            .flex_wrap()
            .p_0p5()
            .gap_0p5()
            .rounded(px(R_BUTTON))
            .bg(rgb(p.field))
            .children(model::SearchEngine::ALL.into_iter().map(|engine| {
                let chosen = engine == current;
                div()
                    .id(engine.name())
                    .role(Role::Button)
                    .aria_label(format!("Search with {}", engine.name()))
                    .aria_selected(chosen)
                    .px_2()
                    .py_1()
                    .rounded(px(R_BUTTON - 2.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .text_color(rgb(if chosen { p.ink } else { p.muted }))
                    .when(chosen, |d| d.bg(rgb(p.card)).shadow_xs())
                    .when(!chosen, |d| d.hover(|s| s.text_color(rgb(p.ink))))
                    .child(engine.name())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_search_engine(engine, cx)))
            }));
        let suggest = Switch::new("suggest-searches")
            .checked(self.settings.suggest_searches)
            .color(hsla(p.accent))
            .accessibility_label("Suggest searches as you type")
            .on_click(cx.listener(|this, on: &bool, _, cx| {
                this.settings.suggest_searches = *on;
                if let Err(error) = this.settings.save(this.store.dir()) {
                    this.notice = Some(format!("Setting not saved: {error}"));
                }
                cx.notify();
            }));
        vec![
            setting_row("Search engine", None, engines.into_any_element(), p),
            setting_row(
                "Suggest searches",
                Some(format!(
                    "What you type in the address bar goes to {} as you type, never from the Private Space. Addresses are never sent.",
                    current.name()
                )),
                suggest.into_any_element(),
                p,
            ),
        ]
    }

    fn privacy_rows(
        &mut self,
        id: [u8; 16],
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let pages = self.history.entries(id).len();
        let history = div()
            .flex()
            .gap_2()
            .child(
                pill_button("settings-history", "Show History", IconName::Clock, p)
                    .on_click(cx.listener(|this, _, w, cx| this.show_tool(Tool::History, w, cx))),
            )
            .child(
                pill_button("settings-clear-history", "Clear…", IconName::Eraser, p)
                    .on_click(cx.listener(|this, _, w, cx| this.clear_history(w, cx))),
            );
        let data = pill_button("clear-website-data", "Clear…", IconName::Eraser, p)
            .on_click(cx.listener(|this, _, w, cx| this.clear_website_data(w, cx)));
        vec![
            setting_row(
                "History",
                Some(match pages {
                    0 => "Nothing yet.".into(),
                    1 => "1 page.".into(),
                    pages => format!("{pages} pages."),
                }),
                history.into_any_element(),
                p,
            ),
            setting_row(
                "Website data",
                Some("Cookies, sign-ins, site storage and caches.".into()),
                data.into_any_element(),
                p,
            ),
        ]
    }

    fn permission_rows(&self, p: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let sites = self.permissions.all(&self.profile);
        if sites.is_empty() {
            return vec![note_row(
                "Sites you allow to use your camera, microphone, location or notifications, and sites you trust with agents, show up here.",
                p,
            )];
        }
        sites
            .into_iter()
            .enumerate()
            .map(|(index, (site, kinds))| {
                let host = model::display_host(&site);
                let reset = pill_button("reset-site", "Reset…", IconName::RotateCcw, p)
                    .id(("reset-site", index))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.reset_site_permissions(site.clone(), w, cx)
                    }));
                setting_row_owned(
                    host,
                    Some(permissions::summary(&kinds)),
                    reset.into_any_element(),
                    p,
                )
            })
            .collect()
    }

    fn extension_rows(&self, id: [u8; 16], p: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = self
            .extensions
            .space(&id)
            .iter()
            .enumerate()
            .map(|(index, installed)| {
                let command = |command| ExtensionMenu {
                    space: id,
                    install: installed.id.clone(),
                    command,
                };
                let (toggle, remove) = (
                    command(ExtensionCommand::Toggle),
                    command(ExtensionCommand::Remove),
                );
                let controls = div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Switch::new(("extension-on", index))
                            .checked(installed.enabled)
                            .color(hsla(p.accent))
                            .accessibility_label(format!("{} turned on", installed.name))
                            .on_click(cx.listener(move |this, _: &bool, w, cx| {
                                this.extension_command(&toggle, w, cx)
                            })),
                    )
                    .child(
                        pill_button("remove-extension", "Remove…", IconName::Trash, p)
                            .id(("remove-extension", index))
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.extension_command(&remove, w, cx)
                            })),
                    );
                setting_row_owned(
                    installed.name.clone(),
                    Some(format!(
                        "Version {}{}",
                        installed.version,
                        if installed.private {
                            " · also in the Private Space"
                        } else {
                            ""
                        }
                    )),
                    controls.into_any_element(),
                    p,
                )
            })
            .collect();
        rows.push(setting_row(
            if rows.is_empty() {
                "No extensions in this Space"
            } else {
                "Add another"
            },
            Some(
                "From a folder or a .zip, such as an unpacked Chrome or Safari web extension."
                    .into(),
            ),
            pill_button(
                "settings-add-extension",
                "Add Extension…",
                IconName::Plus,
                p,
            )
            .on_click(cx.listener(|this, _, w, cx| this.add_extension(w, cx)))
            .into_any_element(),
            p,
        ));
        rows
    }
}

/// A titled group of rows on a quiet ground, as in System Settings.
fn section(title: &'static str, note: Option<String>, rows: Vec<AnyElement>, p: Palette) -> Div {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_1()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(note, |section, note| {
            section.child(
                div()
                    .px_1()
                    .text_size(px(12.))
                    .text_color(rgb(p.muted))
                    .child(note),
            )
        })
        .when(count > 0, |section| {
            section.child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(R_LIST))
                    .bg(rgb(p.field))
                    .children(rows.into_iter().enumerate().map(move |(index, row)| {
                        div()
                            .when(index > 0, |row| row.border_t_1().border_color(rgb(p.line)))
                            .child(row)
                    })),
            )
        })
}

fn setting_row(
    title: &'static str,
    detail: Option<String>,
    control: AnyElement,
    p: Palette,
) -> AnyElement {
    setting_row_owned(title.to_string(), detail, control, p)
}

fn setting_row_owned(
    title: String,
    detail: Option<String>,
    control: AnyElement,
    p: Palette,
) -> AnyElement {
    div()
        .min_h(px(52.))
        .px_4()
        .py_2p5()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().text_size(px(13.)).child(title))
                .when_some(detail, |text, detail| {
                    text.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.muted))
                            .child(detail),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

fn note_row(text: &'static str, p: Palette) -> AnyElement {
    div()
        .px_4()
        .py_3()
        .text_size(px(12.))
        .text_color(rgb(p.muted))
        .child(text)
        .into_any_element()
}
