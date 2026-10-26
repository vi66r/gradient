//! History in the card: the current Space's pages, newest first under day
//! headings, searchable, with each page removable and recent ranges
//! clearable. The list is virtual (`uniform_list`), so a Space's full
//! history costs only the rows on screen.
use crate::Browser;
use crate::history::Entry;
use crate::suggest::{bare, find_term, terms};
use chrono::{DateTime, Datelike, Local, TimeZone};
use gpui::{Action, Context, Focusable, Window};
use std::rc::Rc;

/// One line of the list: a day's heading or a page under it. Both are the
/// same height, as `uniform_list` needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Day(String),
    Page {
        url: String,
        title: String,
        detail: String,
        time: String,
    },
}

/// History ▸ Clear: how far back to forget.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub(crate) enum Range {
    Hour,
    Today,
    Yesterday,
    All,
}

impl Range {
    pub(crate) const ALL: [Self; 4] = [Self::Hour, Self::Today, Self::Yesterday, Self::All];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Hour => "The Last Hour",
            Self::Today => "Today",
            Self::Yesterday => "Today and Yesterday",
            Self::All => "All History",
        }
    }

    /// The first second this range covers; `None` is everything.
    fn since(self, now: DateTime<Local>) -> Option<u64> {
        let midnight = |days: i64| {
            let day = now.date_naive() - chrono::Days::new(days as u64);
            Local
                .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
                .earliest()
                .map(|start| start.timestamp().max(0) as u64)
        };
        match self {
            Self::Hour => Some(now.timestamp().max(0) as u64 - 60 * 60),
            Self::Today => midnight(0),
            Self::Yesterday => midnight(1),
            Self::All => None,
        }
    }
}

#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct ClearRange {
    pub range: Range,
}

/// "Today", "Yesterday", a weekday this week, then a date.
fn day_name(day: DateTime<Local>, now: DateTime<Local>) -> String {
    let days = (now.date_naive() - day.date_naive()).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => day.format("%A").to_string(),
        _ if day.year() == now.year() => day.format("%A, %B %-d").to_string(),
        _ => day.format("%B %-d, %Y").to_string(),
    }
}

/// The list for `query` (every typed word must match the title or address),
/// newest first under day headings.
pub(crate) fn items(entries: &[Entry], query: &str, now: DateTime<Local>) -> Vec<Item> {
    let terms = terms(query);
    let mut pages: Vec<&Entry> = entries
        .iter()
        .filter(|entry| {
            let (title, url) = (entry.title.to_lowercase(), bare(&entry.url).to_lowercase());
            terms
                .iter()
                .all(|term| find_term(&title, term).is_some() || find_term(&url, term).is_some())
        })
        .collect();
    pages.sort_by_key(|entry| std::cmp::Reverse(entry.last));
    let mut items = Vec::new();
    let mut heading = String::new();
    for entry in pages {
        let Some(when) = Local.timestamp_opt(entry.last as i64, 0).single() else {
            continue;
        };
        let day = day_name(when, now);
        if day != heading {
            items.push(Item::Day(day.clone()));
            heading = day;
        }
        let detail = bare(&entry.url);
        items.push(Item::Page {
            url: entry.url.clone(),
            title: if entry.title.trim().is_empty() {
                detail.clone()
            } else {
                entry.title.trim().to_string()
            },
            detail,
            time: when.format("%-H:%M").to_string(),
        });
    }
    items
}

impl Browser {
    /// History opened: a fresh list and the search field focused.
    pub(crate) fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh_history();
        let focus = self.history_search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Rebuilds the list from the Space's history and the search text.
    pub(crate) fn refresh_history(&mut self) {
        let space = self.current_space();
        let (private, id) = (space.private, space.id);
        let query = self.history_query.clone();
        self.history_items = Rc::new(if private {
            Vec::new()
        } else {
            items(self.history.entries(id), &query, Local::now())
        });
    }

    /// A click on a row: the page opens in this tab, or in a new one with ⌘.
    pub(crate) fn open_history_page(
        &mut self,
        url: String,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_tab {
            self.new_tab(url, false, window, cx);
        } else {
            self.tool = None;
            self.navigate(url, window, cx);
        }
    }

    pub(crate) fn remove_history_page(&mut self, url: &str, cx: &mut Context<Self>) {
        let space = self.current_space();
        if space.private {
            return;
        }
        let id = space.id;
        self.history.remove(id, url);
        if let Err(error) = self.history.flush() {
            self.notice = Some(format!("History not saved: {error}"));
        }
        self.refresh_history();
        cx.notify();
    }

    /// History's Clear menu: asks first, then forgets the range in this
    /// Space only.
    pub(crate) fn clear_history_range(
        &mut self,
        range: Range,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let space = self.current_space();
        if space.private || self.discard_pending {
            return;
        }
        let (id, name) = (space.id, space.name.clone());
        let what = match range {
            Range::Hour => "the last hour",
            Range::Today => "today",
            Range::Yesterday => "today and yesterday",
            Range::All => "all time",
        };
        self.discard_pending = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Clear the history of “{name}” from {what}?"),
            Some(
                "Those pages stop showing up here and as suggestions. Open tabs, sign-ins and other Spaces stay as they are.",
            ),
            &["Cancel", "Clear"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            let _ = this.update_in(cx, |this, _, cx| {
                this.discard_pending = false;
                if !confirmed {
                    return;
                }
                let result = match range.since(Local::now()) {
                    Some(since) => {
                        this.history.clear_since(id, since);
                        this.history.flush()
                    }
                    None => this.history.clear(id),
                };
                if let Err(error) = result {
                    this.notice = Some(format!("History not cleared: {error}"));
                }
                this.refresh_history();
                cx.notify();
            });
        })
        .detach();
    }

    /// Settings ▸ Website data: erases the Space's cookies, storage and
    /// caches after asking.
    pub(crate) fn clear_website_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.current_space();
        if space.private || self.discard_pending {
            return;
        }
        let (id, name) = (space.id, space.name.clone());
        self.discard_pending = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Clear the website data of “{name}”?"),
            Some(
                "Sites in this Space forget you: you’re signed out everywhere, and their saved data and caches are erased. History, permissions and other Spaces stay as they are.",
            ),
            &["Cancel", "Clear Website Data"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            let _ = this.update_in(cx, |this, _, cx| {
                this.discard_pending = false;
                if confirmed {
                    crate::engine::clear_website_data(id, this.events.clone());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Site icons from the disk cache for rows coming on screen.
    pub(crate) fn load_history_icons(&mut self, range: std::ops::Range<usize>) {
        let items = self.history_items.clone();
        for item in items.get(range).unwrap_or_default() {
            let Item::Page { url, .. } = item else {
                continue;
            };
            let Some(key) = crate::favicon::host_key(url) else {
                continue;
            };
            if self.icons.contains_key(&key) || self.icon_requests.contains(&key) {
                continue;
            }
            if self.icon_misses.contains(&key) {
                continue;
            }
            match self.icon_cache.read(&key) {
                Some((bytes, fresh)) if let Some(format) = crate::favicon::sniff(&bytes) => {
                    let image = std::sync::Arc::new(gpui::Image::from_bytes(format, bytes));
                    self.icons.insert(key, (image, fresh));
                }
                // Not cached: a globe stands in, without asking the disk again.
                _ => {
                    self.icon_misses.insert(key);
                }
            }
        }
    }
}

/// Unix seconds for tests.
#[cfg(test)]
fn at(now: DateTime<Local>, days_ago: i64, hour: u32) -> u64 {
    let day = now.date_naive() - chrono::Days::new(days_ago as u64);
    Local
        .from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .timestamp() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, title: &str, last: u64) -> Entry {
        Entry {
            url: url.into(),
            title: title.into(),
            visits: 1,
            last,
        }
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 26, 18, 30, 0).unwrap()
    }

    #[test]
    fn pages_list_newest_first_under_their_days() {
        let now = now();
        let entries = [
            entry("https://old.example/", "", at(now, 40, 9)),
            entry("https://github.com/", "GitHub", at(now, 0, 9)),
            entry("https://news.example/a", "News", at(now, 1, 20)),
            entry("https://docs.rs/", "Docs.rs", at(now, 0, 17)),
        ];
        let list = items(&entries, "", now);
        let names: Vec<String> = list
            .iter()
            .map(|item| match item {
                Item::Day(day) => format!("# {day}"),
                Item::Page { title, time, .. } => format!("{title} {time}"),
            })
            .collect();
        assert_eq!(
            names,
            [
                "# Today",
                "Docs.rs 17:00",
                "GitHub 9:00",
                "# Yesterday",
                "News 20:00",
                "# Wednesday, September 16",
                "old.example 9:00",
            ]
        );
    }

    #[test]
    fn search_matches_every_word_in_titles_or_addresses() {
        let now = now();
        let entries = [
            entry(
                "https://github.com/rafi/browser",
                "rafi/browser",
                at(now, 0, 9),
            ),
            entry("https://docs.rs/gpui", "gpui - Rust", at(now, 0, 10)),
        ];
        let pages = |query| {
            items(&entries, query, now)
                .into_iter()
                .filter(|item| matches!(item, Item::Page { .. }))
                .count()
        };
        assert_eq!(pages("github browser"), 1);
        assert_eq!(pages("rust gpui"), 1);
        assert_eq!(pages("rust browser"), 0);
        assert_eq!(pages(""), 2);
    }

    #[test]
    fn ranges_start_at_the_hour_or_local_midnight() {
        let now = now();
        assert_eq!(Range::Hour.since(now), Some(now.timestamp() as u64 - 3600));
        assert_eq!(Range::Today.since(now), Some(at(now, 0, 0)));
        assert_eq!(Range::Yesterday.since(now), Some(at(now, 1, 0)));
        assert_eq!(Range::All.since(now), None);
    }
}
