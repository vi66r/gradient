//! Address suggestions while typing: the first row is what Enter does (the
//! top history hit, completed inline as in Safari, or the typed search or
//! address). When a hit took the first place, searching for the typed text
//! follows it, one ↓ away. Then the search engine's suggestions (searches
//! only, never from the Private Space), this Space's open tabs, and its
//! history by frecency with host-prefix matches first.
//!
//! WebKit draws above GPUI, so while the list is open the page is shown as
//! a still picture (`Cover`) and its native view hidden: nothing reflows
//! and nothing visibly changes beneath the list.
use crate::Browser;
use crate::history::{self, Entry};
use crate::model::{self, SearchEngine};
use gpui::{Context, RenderImage, Window};
use std::sync::Arc;

pub(crate) const MAX_ROWS: usize = 8;
const MAX_TABS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Bring this tab in the Space on screen to the front.
    Tab(u64),
    Open(String),
    /// What Enter in the field does anyway: search, an address, an app link.
    Typed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Look {
    Tab,
    Page,
    Search,
    Address,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub action: Action,
    pub look: Look,
    pub title: String,
    pub detail: String,
    /// The page the row stands for, for its site icon.
    pub url: Option<String>,
}

pub(crate) struct OpenTab<'a> {
    pub id: u64,
    pub title: &'a str,
    pub url: &'a str,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Ranked {
    pub rows: Vec<Row>,
    /// Text to show, selected, after what was typed.
    pub completion: String,
}

/// An address as people read it: no scheme, no `www.`, no lone `/`.
pub(crate) fn bare(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    rest.strip_suffix('/')
        .filter(|host| !host.contains('/'))
        .unwrap_or(rest)
        .to_string()
}

/// The typed text as it is matched against addresses.
fn needle(typed: &str) -> String {
    let typed = typed.trim().to_ascii_lowercase();
    let typed = typed
        .strip_prefix("https://")
        .or_else(|| typed.strip_prefix("http://"))
        .unwrap_or(&typed);
    typed.strip_prefix("www.").unwrap_or(typed).to_string()
}

/// The typed words, each matched on its own.
pub(crate) fn terms(typed: &str) -> Vec<String> {
    typed
        .split_whitespace()
        .map(needle)
        .filter(|term| !term.is_empty())
        .collect()
}

/// Where `term` appears in `text` (both lowercase), preferring the start of
/// a word. One or two letters match only at a word's start, so `g` finds
/// "GitHub" but not "Page".
pub(crate) fn find_term(text: &str, term: &str) -> Option<usize> {
    let starts_word = |at: usize| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    };
    let mut found = text.match_indices(term).map(|(at, _)| at);
    let first = found.clone().next()?;
    found
        .find(|&at| starts_word(at))
        .or_else(|| (term.chars().count() >= 3).then_some(first))
}

fn matches(terms: &[String], title: &str, url: &str) -> bool {
    let (title, url) = (title.to_lowercase(), bare(url).to_ascii_lowercase());
    terms
        .iter()
        .all(|term| find_term(&title, term).is_some() || find_term(&url, term).is_some())
}

fn host_prefix(needle: &str, url: &str) -> bool {
    !needle.is_empty()
        && !needle.contains(char::is_whitespace)
        && crate::favicon::host_key(url).is_some_and(|host| host.starts_with(needle))
}

/// The page a completion stands for.
struct Hit {
    /// The completed text, matched case-insensitively against the needle.
    label: String,
    url: String,
    title: String,
}

/// The best page whose address starts with the needle: a whole site
/// (`git` → `github.com`) or, once a path is typed, a page.
fn top_hit(needle: &str, history: &[Entry], now: u64) -> Option<Hit> {
    if needle.is_empty() || needle.contains(char::is_whitespace) {
        return None;
    }
    let score = |entry: &Entry| history::frecency(entry, now);
    if needle.contains('/') {
        let entry = history
            .iter()
            .filter(|entry| bare(&entry.url).to_ascii_lowercase().starts_with(needle))
            .max_by(|a, b| score(a).total_cmp(&score(b)))?;
        return Some(Hit {
            label: bare(&entry.url),
            url: entry.url.clone(),
            title: entry.title.clone(),
        });
    }
    let mut hosts: Vec<(String, f64, &Entry)> = Vec::new();
    for entry in history {
        let Some(host) = crate::favicon::host_key(&entry.url).filter(|h| h.starts_with(needle))
        else {
            continue;
        };
        match hosts.iter_mut().find(|(known, ..)| *known == host) {
            Some((_, total, best)) => {
                *total += score(entry);
                if score(entry) > score(best) {
                    *best = entry;
                }
            }
            None => hosts.push((host, score(entry), entry)),
        }
    }
    // Ties go to the shorter host: `git` completes to the site you meant
    // rather than to one of its subdomains.
    let (host, _, best) = hosts
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.len().cmp(&a.0.len())))?;
    let root = history
        .iter()
        .filter(|entry| bare(&entry.url) == host)
        .max_by(|a, b| score(a).total_cmp(&score(b)));
    let url = match root {
        Some(root) => root.url.clone(),
        None => url::Url::parse(&best.url)
            .map(|url| format!("{}/", url.origin().ascii_serialization()))
            .ok()?,
    };
    Some(Hit {
        title: root.map(|root| root.title.clone()).unwrap_or_default(),
        label: host,
        url,
    })
}

fn titled(title: &str, url: &str) -> String {
    if title.trim().is_empty() {
        bare(url)
    } else {
        title.trim().to_string()
    }
}

/// What Enter does with the typed text, as a row.
fn typed_row(typed: &str, engine: SearchEngine) -> Option<Row> {
    let query = typed.trim();
    let row = |look, title: String, detail: String, url| Row {
        action: Action::Typed,
        look,
        title,
        detail,
        url,
    };
    match model::entry(typed, engine).ok()? {
        model::Entry::Web(url) if url == engine.search(query) => Some(row(
            Look::Search,
            query.to_string(),
            format!("Search {}", engine.name()),
            None,
        )),
        model::Entry::Web(url) if url == model::START => None,
        model::Entry::Web(url) => Some(row(Look::Address, bare(&url), String::new(), Some(url))),
        model::Entry::App(url) => Some(row(
            Look::Address,
            url.to_string(),
            "Open in another app".into(),
            None,
        )),
    }
}

/// At most this many of the search engine's suggestions.
const MAX_REMOTE: usize = 4;
/// Open tabs and history keep at least this many rows when they have them.
const MIN_PERSONAL: usize = 3;

/// Ranks suggestions for `typed`. `complete` allows an inline completion:
/// only while the user is adding text at its end, never after a deletion.
/// `remote` is what the search engine suggested for exactly this text.
///
/// The order is Safari's: what Enter does, then searches (the typed text
/// when a page took the first row, then the engine's suggestions), then
/// open tabs and history.
pub(crate) fn rank(
    typed: &str,
    complete: bool,
    tabs: &[OpenTab],
    history: &[Entry],
    remote: &[String],
    engine: SearchEngine,
    now: u64,
) -> Ranked {
    let needle = needle(typed);
    if needle.is_empty() {
        return Ranked::default();
    }
    let terms = terms(typed);
    let query = typed.trim();
    let hit = (complete && typed == typed.trim_end())
        .then(|| top_hit(&needle, history, now))
        .flatten();
    let mut ranked = Ranked::default();
    let mut shown: Vec<String> = Vec::new();
    // A hit that is already open switches to its tab, as in Safari.
    let hit_tab = hit.as_ref().and_then(|hit| {
        tabs.iter()
            .find(|tab| history::key(tab.url).as_ref() == Some(&hit.url))
    });
    match &hit {
        Some(hit) => {
            ranked.completion = hit
                .label
                .get(needle.len()..)
                .unwrap_or_default()
                .to_string();
            ranked.rows.push(match hit_tab {
                Some(tab) => Row {
                    action: Action::Tab(tab.id),
                    look: Look::Tab,
                    title: titled(tab.title, tab.url),
                    detail: hit.label.clone(),
                    url: Some(hit.url.clone()),
                },
                None => Row {
                    action: Action::Open(hit.url.clone()),
                    look: Look::Page,
                    title: titled(&hit.title, &hit.url),
                    detail: hit.label.clone(),
                    url: Some(hit.url.clone()),
                },
            });
            shown.push(hit.url.clone());
            // Searching for the typed text stays one ↓ away.
            ranked.rows.push(Row {
                action: Action::Open(engine.search(query)),
                look: Look::Search,
                title: query.to_string(),
                detail: format!("Search {}", engine.name()),
                url: None,
            });
        }
        None => ranked.rows.extend(typed_row(typed, engine)),
    }
    let mut open: Vec<&OpenTab> = tabs
        .iter()
        .filter(|tab| hit_tab.is_none_or(|hit| hit.id != tab.id))
        .filter(|tab| model::allowed_address(tab.url) && matches(&terms, tab.title, tab.url))
        .collect();
    open.sort_by_key(|tab| !host_prefix(&needle, tab.url));
    open.truncate(MAX_TABS);
    let open_urls: Vec<String> = open
        .iter()
        .filter_map(|tab| history::key(tab.url))
        .collect();
    let mut pages: Vec<(bool, f64, &Entry)> = history
        .iter()
        .filter(|entry| {
            !shown.contains(&entry.url)
                && !open_urls.contains(&entry.url)
                && matches(&terms, &entry.title, &entry.url)
        })
        .map(|entry| {
            (
                !host_prefix(&needle, &entry.url),
                history::frecency(entry, now),
                entry,
            )
        })
        .collect();
    pages.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.total_cmp(&a.1)));
    let personal = (open.len() + pages.len()).min(MIN_PERSONAL);
    let lower = query.to_lowercase();
    let room = MAX_ROWS.saturating_sub(ranked.rows.len() + personal);
    ranked.rows.extend(
        remote
            .iter()
            .filter(|suggestion| suggestion.to_lowercase() != lower)
            .take(MAX_REMOTE.min(room))
            .map(|suggestion| Row {
                action: Action::Open(engine.search(suggestion)),
                look: Look::Search,
                title: suggestion.clone(),
                detail: String::new(),
                url: None,
            }),
    );
    for tab in open {
        if ranked.rows.len() >= MAX_ROWS {
            break;
        }
        ranked.rows.push(Row {
            action: Action::Tab(tab.id),
            look: Look::Tab,
            title: titled(tab.title, tab.url),
            detail: bare(tab.url),
            url: Some(tab.url.to_string()),
        });
    }
    for (_, _, entry) in pages {
        if ranked.rows.len() >= MAX_ROWS {
            break;
        }
        ranked.rows.push(Row {
            action: Action::Open(entry.url.clone()),
            look: Look::Page,
            title: titled(&entry.title, &entry.url),
            detail: bare(&entry.url),
            url: Some(entry.url.clone()),
        });
    }
    ranked
}

/// The search engine's suggestions for typed text, with recent answers kept
/// so deleting back to earlier text shows them at once.
#[derive(Default)]
pub(crate) struct Remote {
    /// Bumped with each keystroke; a late answer for older text is dropped.
    pub epoch: u64,
    recent: std::collections::VecDeque<(SearchEngine, String, Vec<String>)>,
}

const REMOTE_KEPT: usize = 32;
/// Typing pauses this long before the engine is asked.
const REMOTE_PAUSE: std::time::Duration = std::time::Duration::from_millis(90);

impl Remote {
    fn get(&self, engine: SearchEngine, query: &str) -> Option<&[String]> {
        self.recent
            .iter()
            .find(|(kept, text, _)| *kept == engine && text == query)
            .map(|(_, _, suggestions)| suggestions.as_slice())
    }

    fn keep(&mut self, engine: SearchEngine, query: String, suggestions: Vec<String>) {
        self.recent
            .retain(|(kept, text, _)| !(*kept == engine && *text == query));
        self.recent.push_front((engine, query, suggestions));
        self.recent.truncate(REMOTE_KEPT);
    }
}

/// What the engine may be asked about: a search, never an address, a local
/// name or an app link, so typed addresses don't leave the machine.
pub(crate) fn askable(typed: &str, engine: SearchEngine) -> Option<String> {
    let query = typed.trim();
    if query.is_empty() || query.len() > 200 {
        return None;
    }
    match model::entry(typed, engine).ok()? {
        model::Entry::Web(url) if url == engine.search(query) => Some(query.to_string()),
        _ => None,
    }
}

/// Reads the engine's answer: `["query", ["suggestion", …], …]`.
pub(crate) fn parse_remote(body: &str) -> Vec<String> {
    let Ok(serde_json::Value::Array(parts)) = serde_json::from_str(body) else {
        return Vec::new();
    };
    let Some(serde_json::Value::Array(list)) = parts.get(1) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| item.as_str())
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty() && item.len() <= 200)
        .take(8)
        .collect()
}

/// Asks the engine (blocking; run off the main thread). No cookies are sent:
/// the request isn't the page's and carries nothing but the text.
fn fetch_remote(url: &str) -> Vec<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .max_redirects(0)
        .build()
        .into();
    agent
        .get(url)
        .call()
        .ok()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(64 * 1024)
                .read_to_string()
                .ok()
        })
        .map(|body| parse_remote(&body))
        .unwrap_or_default()
}

/// What the list covers the page with while it is open.
#[derive(Default)]
pub(crate) enum Cover {
    #[default]
    None,
    /// A picture of this tab's page was asked for.
    Waiting(u64),
    /// This tab's page is hidden behind its picture (or behind nothing, if
    /// WebKit couldn't take one).
    Ready(u64, Option<Arc<RenderImage>>),
}

#[derive(Default)]
pub(crate) struct Suggestions {
    pub rows: Vec<Row>,
    pub selected: usize,
    pub open: bool,
    /// The user's own text, without the completion.
    pub typed: String,
    /// The completion showing, selected, after `typed`.
    pub completion: String,
    /// Whether the last edit allowed an inline completion, to rank again
    /// the same way when the engine answers.
    pub complete: bool,
    pub cover: Cover,
}

impl Suggestions {
    /// Open, and nothing native left to draw over it.
    pub(crate) fn shown(&self) -> bool {
        self.open && !matches!(self.cover, Cover::Waiting(_))
    }
}

impl Browser {
    /// The Space a tab's visits belong to; none for private tabs.
    pub(crate) fn history_space(&self, id: u64) -> Option<[u8; 16]> {
        let space = &self.spaces[self.space_of(id)?];
        let tab = self.all_tabs().find(|tab| tab.id == id)?;
        (!space.private && !tab.private).then_some(space.id)
    }

    /// A page committed (or an app changed its address in place).
    pub(crate) fn record_visit(&mut self, id: u64) {
        let Some(space) = self.history_space(id) else {
            return;
        };
        let Some(tab) = self.all_tabs().find(|tab| tab.id == id) else {
            return;
        };
        let Some(url) = tab.page.as_ref().and_then(|page| page.url().ok()) else {
            return;
        };
        // The title still belongs to the previous page; the new one follows.
        self.history.visit(space, &url, "", history::now());
    }

    pub(crate) fn record_title(&mut self, id: u64) {
        let Some(space) = self.history_space(id) else {
            return;
        };
        let Some(tab) = self.all_tabs().find(|tab| tab.id == id) else {
            return;
        };
        let url = tab
            .page
            .as_ref()
            .and_then(|page| page.url().ok())
            .unwrap_or_else(|| tab.saved.url.clone());
        let title = tab.saved.title.clone();
        self.history.retitle(space, &url, &title);
    }

    /// The field's text changed by the user's hand: re-rank, complete
    /// inline if they are typing forward, and cover the page.
    pub(crate) fn address_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui::EntityInputHandler;
        // A late event after the field let go (switching tabs) isn't typing.
        if !self.address_focused(window, cx) {
            return;
        }
        let (value, selection, composing) = self.address.update(cx, |input, cx| {
            let composing = input.marked_text_range(window, cx).is_some();
            (input.value().to_string(), input.selected_range(), composing)
        });
        // Our own completion, read back by a late event, is no edit.
        if !self.suggest.completion.is_empty()
            && value == format!("{}{}", self.suggest.typed, self.suggest.completion)
        {
            return;
        }
        let previous = std::mem::take(&mut self.suggest.typed);
        let grew = value.len() > previous.len() && value.starts_with(&previous);
        let complete = grew && !composing && selection == (value.len()..value.len());
        let engine = self.settings.search;
        let asking = askable(&value, engine)
            .filter(|_| self.settings.suggest_searches && !self.current_space().private);
        let remote = asking
            .as_deref()
            .and_then(|query| self.remote.get(engine, query))
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let ranked = self.ranked(&value, complete, &remote);
        self.suggest.complete = complete;
        self.remote.epoch += 1;
        if let Some(query) = asking
            && self.remote.get(engine, &query).is_none()
        {
            self.ask_engine(engine, query, cx);
        }
        self.suggest.typed = value.clone();
        self.suggest.completion.clear();
        if !ranked.completion.is_empty() {
            let full = format!("{value}{}", ranked.completion);
            self.address.update(cx, |input, cx| {
                input.set_value(full.clone(), window, cx);
                input.set_selected_range(value.len()..full.len(), cx);
            });
            self.suggest.completion = ranked.completion;
        }
        self.suggest.rows = ranked.rows;
        self.suggest.selected = 0;
        self.load_row_icons();
        self.suggest.open = !self.suggest.rows.is_empty();
        if self.suggest.open {
            self.cover_page();
        } else {
            self.close_suggestions();
        }
        cx.notify();
    }

    /// Ranks `value` against this Space's tabs (but the one in front) and
    /// history.
    fn ranked(&mut self, value: &str, complete: bool, remote: &[String]) -> Ranked {
        let active = self.tabs[self.active].id;
        let tabs: Vec<OpenTab> = self
            .tabs
            .iter()
            .filter(|tab| tab.id != active)
            .map(|tab| OpenTab {
                id: tab.id,
                title: &tab.saved.title,
                url: &tab.saved.url,
            })
            .collect();
        let space = self.current_space();
        let entries = if space.private {
            &[][..]
        } else {
            self.history.entries(space.id)
        };
        rank(
            value,
            complete,
            &tabs,
            entries,
            remote,
            self.settings.search,
            history::now(),
        )
    }

    /// Asks the search engine about `query` once typing pauses; the answer
    /// joins the list if the field still says the same.
    fn ask_engine(&mut self, engine: SearchEngine, query: String, cx: &mut Context<Self>) {
        let epoch = self.remote.epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REMOTE_PAUSE).await;
            let still = this
                .update(cx, |this, _| this.remote.epoch == epoch)
                .unwrap_or(false);
            if !still {
                return;
            }
            let url = engine.suggestions(&query);
            let suggestions = cx
                .background_executor()
                .spawn(async move { fetch_remote(&url) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.remote.keep(engine, query.clone(), suggestions.clone());
                if this.remote.epoch != epoch
                    || !this.suggest.open
                    || this.suggest.typed.trim() != query
                {
                    return;
                }
                let selected = this.suggest.rows.get(this.suggest.selected).cloned();
                let typed = this.suggest.typed.clone();
                let ranked = this.ranked(&typed, this.suggest.complete, &suggestions);
                // The row the user moved to stays chosen.
                this.suggest.selected = selected
                    .and_then(|row| ranked.rows.iter().position(|new| new.action == row.action))
                    .unwrap_or(0);
                this.suggest.rows = ranked.rows;
                cx.notify();
            });
        })
        .detach();
    }

    /// Site icons for the rows from the disk cache; history reaches sites
    /// that haven't loaded this session. Never for the Private Space.
    fn load_row_icons(&mut self) {
        if self.current_space().private {
            return;
        }
        for row in &self.suggest.rows {
            let Some(key) = row.url.as_deref().and_then(crate::favicon::host_key) else {
                continue;
            };
            if self.icons.contains_key(&key) || self.icon_requests.contains(&key) {
                continue;
            }
            if let Some((bytes, fresh)) = self.icon_cache.read(&key)
                && let Some(format) = crate::favicon::sniff(&bytes)
            {
                let image = Arc::new(gpui::Image::from_bytes(format, bytes));
                self.icons.insert(key, (image, fresh));
            }
        }
    }

    /// Asks the active page for a picture to stand in for it under the list.
    fn cover_page(&mut self) {
        let tab = &self.tabs[self.active];
        let Some(page) = tab.page.as_ref().filter(|_| self.tool.is_none()) else {
            return;
        };
        match self.suggest.cover {
            Cover::Waiting(id) | Cover::Ready(id, _) if id == tab.id => {}
            _ => {
                self.suggest.cover = Cover::Waiting(tab.id);
                page.snapshot(tab.id, self.events.clone());
            }
        }
    }

    /// WebKit's picture of the page arrived: it takes the page's place.
    pub(crate) fn page_snapshot(&mut self, id: u64, image: Option<Arc<RenderImage>>) {
        if !matches!(self.suggest.cover, Cover::Waiting(waiting) if waiting == id) {
            return;
        }
        if !self.suggest.open || self.tabs[self.active].id != id {
            self.suggest.cover = Cover::None;
            return;
        }
        if let Some(page) = &self.tabs[self.active].page {
            page.hide();
        }
        self.suggest.cover = Cover::Ready(id, image);
    }

    /// Closes the list; it fades out over the picture, then the page returns
    /// (see `Render`).
    pub(crate) fn close_suggestions(&mut self) {
        self.suggest.open = false;
        if matches!(self.suggest.cover, Cover::Waiting(_)) {
            self.suggest.cover = Cover::None;
        }
    }

    /// Drops the list and its picture at once (switching tabs).
    pub(crate) fn reset_suggestions(&mut self) {
        self.suggest = Suggestions::default();
    }

    /// ↑ / ↓ while the list is open. Leaving the first row takes its
    /// completion out of the field, since Enter no longer goes there.
    pub(crate) fn move_suggestion(
        &mut self,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = self.suggest.rows.len().saturating_sub(1);
        let selected = if down {
            (self.suggest.selected + 1).min(last)
        } else {
            self.suggest.selected.saturating_sub(1)
        };
        if selected != 0 && !self.suggest.completion.is_empty() {
            self.drop_completion(window, cx);
        }
        self.suggest.selected = selected;
        cx.notify();
    }

    pub(crate) fn drop_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest.completion.is_empty() {
            return;
        }
        self.suggest.completion.clear();
        let typed = self.suggest.typed.clone();
        self.address
            .update(cx, |input, cx| input.set_value(typed, window, cx));
    }

    /// Enter or a click on a row.
    pub(crate) fn choose_suggestion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.suggest.rows.get(index).cloned() else {
            return;
        };
        let typed = self.suggest.typed.clone();
        self.close_suggestions();
        match row.action {
            Action::Tab(id) => self.switch_to_tab(id, window, cx),
            Action::Open(url) => self.navigate(url, window, cx),
            Action::Typed => self.enter_address(&typed, window, cx),
        }
    }

    /// "Switch to Tab". A blank tab the user was typing in has done its job
    /// and closes, as in Safari.
    fn switch_to_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let current = &self.tabs[self.active];
        if index != self.active && current.saved.url == model::START && current.page.is_none() {
            let blank = self.active;
            self.tabs.remove(blank);
            let index = if blank < index { index - 1 } else { index };
            self.activate(index, window, cx);
            self.save();
        } else {
            self.activate(index, window, cx);
        }
    }

    /// History ▸ Clear History…: asks first; only this Space's history goes.
    pub(crate) fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.current_space();
        if space.private {
            self.notice = Some("The Private Space keeps no history.".into());
            cx.notify();
            return;
        }
        if self.discard_pending {
            return;
        }
        let (id, name) = (space.id, space.name.clone());
        self.discard_pending = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Clear the history of “{name}”?"),
            Some(
                "Browser forgets the pages this Space visited, so they stop showing up as suggestions. Open tabs, sign-ins and other Spaces stay as they are.",
            ),
            &["Cancel", "Clear History"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            let _ = this.update_in(cx, |this, _, cx| {
                this.discard_pending = false;
                if confirmed && let Err(error) = this.history.clear(id) {
                    this.notice = Some(format!("History not cleared: {error}"));
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const DDG: SearchEngine = SearchEngine::DuckDuckGo;

    fn entry(url: &str, title: &str, visits: u32, days_ago: u64) -> Entry {
        Entry {
            url: url.into(),
            title: title.into(),
            visits,
            last: NOW - days_ago * 24 * 60 * 60,
        }
    }

    fn history() -> Vec<Entry> {
        vec![
            entry("https://github.com/", "GitHub", 4, 1),
            entry("https://github.com/rafi/browser", "rafi/browser", 9, 1),
            entry("https://gist.github.com/", "Gists", 2, 1),
            entry("https://news.ycombinator.com/", "Hacker News", 30, 2),
            entry("https://example.com/git-guide", "A guide to git", 1, 3),
            entry("http://localhost:5173/", "Vite App", 3, 0),
        ]
    }

    #[test]
    fn addresses_read_without_scheme_www_or_a_lone_slash() {
        assert_eq!(bare("https://www.github.com/"), "github.com");
        assert_eq!(bare("https://github.com/rafi/"), "github.com/rafi/");
        assert_eq!(bare("http://localhost:5173/"), "localhost:5173");
    }

    #[test]
    fn typing_a_host_prefix_completes_the_site_and_goes_to_its_root() {
        let ranked = rank("git", true, &[], &history(), &[], DDG, NOW);
        assert_eq!(ranked.completion, "hub.com");
        assert_eq!(
            ranked.rows[0].action,
            Action::Open("https://github.com/".into())
        );
        assert_eq!(ranked.rows[0].title, "GitHub");
        // Then searching for the typed text, one ↓ away, and the site's
        // pages, host matches before the rest.
        let urls: Vec<_> = ranked.rows.iter().map(|row| row.action.clone()).collect();
        assert_eq!(
            urls[1..],
            [
                Action::Open(DDG.search("git")),
                Action::Open("https://github.com/rafi/browser".into()),
                Action::Open("https://gist.github.com/".into()),
                Action::Open("https://example.com/git-guide".into()),
            ]
        );
        // Local servers keep their scheme and port.
        let ranked = rank("local", true, &[], &history(), &[], DDG, NOW);
        assert_eq!(ranked.completion, "host:5173");
        assert_eq!(
            ranked.rows[0].action,
            Action::Open("http://localhost:5173/".into())
        );
    }

    #[test]
    fn a_typed_path_completes_to_a_page() {
        let ranked = rank("github.com/r", true, &[], &history(), &[], DDG, NOW);
        assert_eq!(ranked.completion, "afi/browser");
        assert_eq!(
            ranked.rows[0].action,
            Action::Open("https://github.com/rafi/browser".into())
        );
        // Schemes and `www.` typed by hand still match.
        let ranked = rank("https://www.news", true, &[], &history(), &[], DDG, NOW);
        assert_eq!(ranked.completion, ".ycombinator.com");
    }

    #[test]
    fn without_completion_enter_does_what_was_typed() {
        // After a deletion, or for words, the first row is the search.
        for (typed, complete) in [("git", false), ("guide to", true), ("git ", true)] {
            let ranked = rank(typed, complete, &[], &history(), &[], DDG, NOW);
            assert!(ranked.completion.is_empty(), "{typed}");
            assert_eq!(ranked.rows[0].action, Action::Typed, "{typed}");
            assert_eq!(ranked.rows[0].look, Look::Search, "{typed}");
            assert_eq!(ranked.rows[0].detail, "Search DuckDuckGo");
        }
        let ranked = rank("guide to", true, &[], &history(), &[], DDG, NOW);
        assert_eq!(
            ranked.rows[1].action,
            Action::Open("https://example.com/git-guide".into())
        );
        // An address that matches nothing is offered as itself.
        let ranked = rank(
            "example.org/a",
            true,
            &[],
            &history(),
            &[],
            SearchEngine::Kagi,
            NOW,
        );
        assert_eq!(ranked.rows.len(), 1);
        assert_eq!(ranked.rows[0].look, Look::Address);
        assert_eq!(ranked.rows[0].title, "example.org/a");
        assert!(
            rank("   ", true, &[], &history(), &[], DDG, NOW)
                .rows
                .is_empty()
        );
    }

    #[test]
    fn open_tabs_come_before_history_and_are_not_repeated() {
        let tabs = [
            OpenTab {
                id: 4,
                title: "rafi/browser",
                url: "https://github.com/rafi/browser",
            },
            OpenTab {
                id: 5,
                title: "New tab",
                url: model::START,
            },
        ];
        let ranked = rank("browser", true, &tabs, &history(), &[], DDG, NOW);
        assert_eq!(ranked.rows[0].look, Look::Search);
        assert_eq!(ranked.rows[1].action, Action::Tab(4));
        assert!(
            ranked.rows[2..]
                .iter()
                .all(|row| row.action != Action::Open("https://github.com/rafi/browser".into()))
        );
    }

    #[test]
    fn an_open_top_hit_switches_to_its_tab() {
        let tabs = [OpenTab {
            id: 7,
            title: "Hacker News",
            url: "https://news.ycombinator.com/",
        }];
        let ranked = rank("news", true, &tabs, &history(), &[], DDG, NOW);
        assert_eq!(ranked.completion, ".ycombinator.com");
        assert_eq!(ranked.rows[0].action, Action::Tab(7));
        assert_eq!(ranked.rows[0].look, Look::Tab);
        assert_eq!(
            ranked
                .rows
                .iter()
                .filter(|row| row.detail == "news.ycombinator.com")
                .count(),
            1
        );
    }

    #[test]
    fn recent_frequent_sites_win_the_completion() {
        let history = vec![
            entry("https://news.ycombinator.com/", "HN", 40, 1),
            entry("https://news.example.com/", "Example News", 40, 120),
        ];
        let ranked = rank("news", true, &[], &history, &[], DDG, NOW);
        assert_eq!(ranked.completion, ".ycombinator.com");
        let ranked = rank("n", true, &[], &history, &[], DDG, NOW);
        assert!(ranked.rows.len() <= MAX_ROWS);
    }

    #[test]
    fn short_terms_match_only_at_word_starts() {
        assert_eq!(find_term("page basics", "g"), None);
        assert_eq!(find_term("gpui - rust", "g"), Some(0));
        assert_eq!(find_term("docs.rs/gpui", "gp"), Some(8));
        // Longer terms match inside words, but a word start still wins.
        assert_eq!(find_term("rust-lang lang", "ang"), Some(6));
        assert_eq!(find_term("a lang", "lan"), Some(2));
        let ranked = rank("g", true, &[], &history(), &[], DDG, NOW);
        assert!(
            ranked
                .rows
                .iter()
                .all(|row| row.detail != "news.ycombinator.com")
        );
    }

    #[test]
    fn the_list_is_capped() {
        let history: Vec<Entry> = (0..40)
            .map(|index| entry(&format!("https://site{index}.example/"), "Site", 1, 1))
            .collect();
        let ranked = rank("site", true, &[], &history, &[], DDG, NOW);
        assert_eq!(ranked.rows.len(), MAX_ROWS);
        assert_eq!(ranked.rows[1].look, Look::Search);
        assert!(ranked.rows[2..].iter().all(|row| row.look == Look::Page));
    }

    #[test]
    fn engine_suggestions_follow_the_searches_and_leave_room_for_history() {
        let remote: Vec<String> = [
            "git",
            "git commit",
            "github copilot",
            "gitlab",
            "git rebase",
            "gitea",
        ]
        .map(String::from)
        .into();
        let ranked = rank("git", true, &[], &history(), &remote, DDG, NOW);
        let titles: Vec<&str> = ranked.rows.iter().map(|row| row.title.as_str()).collect();
        // The hit, the typed search, then the engine's (the typed text once
        // only), and three of history's pages still fit.
        assert_eq!(
            titles[..5],
            ["GitHub", "git", "git commit", "github copilot", "gitlab"]
        );
        assert_eq!(ranked.rows.len(), MAX_ROWS);
        assert!(ranked.rows[5..].iter().all(|row| row.look == Look::Page));
        assert_eq!(
            ranked.rows[2].action,
            Action::Open(DDG.search("git commit"))
        );
        // With no history, the engine gets its full share.
        let ranked = rank("rust bor", true, &[], &[], &remote, DDG, NOW);
        assert_eq!(ranked.rows.len(), 1 + MAX_REMOTE);
    }

    #[test]
    fn only_searches_are_sent_to_the_engine() {
        assert_eq!(askable("rust borrow", DDG).as_deref(), Some("rust borrow"));
        assert_eq!(askable("  git ", DDG).as_deref(), Some("git"));
        for never in [
            "github.com",
            "localhost:5173",
            "https://example.com/a?q=1",
            "mailto:a@b.com",
            "192.168.1.1",
            "",
        ] {
            assert_eq!(askable(never, DDG), None, "{never}");
        }
    }

    #[test]
    fn engine_answers_are_read_defensively() {
        assert_eq!(
            parse_remote(r#"["rust",["rust lang"," rust book ",""],[],{}]"#),
            ["rust lang", "rust book"]
        );
        assert!(parse_remote("not json").is_empty());
        assert!(parse_remote(r#"{"q":"rust"}"#).is_empty());
        assert!(parse_remote(r#"["rust",[1,2]]"#).is_empty());
    }
}
