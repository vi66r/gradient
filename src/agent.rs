//! Agents' access to tabs, which only the user grants, in Gradient's own UI:
//! - Shared: one tab, for this session, on its current site. Untrusted:
//!   the agent reads only what a person could see, acts by real clicks and
//!   typing, stays on the site, and Gradient asks the user before a click
//!   that may submit or change something.
//! - Trusted: a site the user trusts, remembered with its other site
//!   permissions until they withdraw it. Every tab on it is open to agents
//!   as fully as to the user: screenshots, the DOM, console and network
//!   logs, JavaScript, keys and uploads, with no questions asked.
//!
//! Either way, an agent's scripts run in a content world of their own,
//! which the page can't see into or tamper with.
use crate::{Browser, Tab, engine, mcp, permissions::Kind};
use gpui::{AsyncWindowContext, Context, PromptLevel, WeakEntity, Window};
use serde_json::{Value, json};
use std::{rc::Rc, time::Instant};

/// How far agents may go in a tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Access {
    /// Shared by the user for this session, while on `origin`.
    Shared { origin: String, epoch: u64 },
    /// On a site the user trusts.
    Trusted { origin: String },
}

impl Access {
    pub(crate) fn trusted(&self) -> bool {
        matches!(self, Access::Trusted { .. })
    }

    fn origin(&self) -> &str {
        match self {
            Access::Shared { origin, .. } | Access::Trusted { origin } => origin,
        }
    }
}

/// Files an agent may upload at once, in all.
const UPLOAD_LIMIT: u64 = 25 * 1024 * 1024;

impl Browser {
    fn profile_of(&self, id: u64) -> Option<[u8; 16]> {
        self.space_of(id).map(|index| self.spaces[index].id)
    }

    /// The tab's site, while it shows a web page.
    fn tab_site(tab: &Tab) -> Option<String> {
        let url = match &tab.page {
            Some(page) => page.url().ok()?,
            None => tab.saved.url.clone(),
        };
        mcp::origin(&url)
    }

    /// What agents may do in `tab`, if anything.
    pub(crate) fn agent_access(&self, tab: &Tab) -> Option<Access> {
        if tab.private {
            return None;
        }
        let origin = Self::tab_site(tab)?;
        let profile = self.profile_of(tab.id)?;
        if self.permissions.allowed(&profile, &origin, &[Kind::Agents]) {
            return Some(Access::Trusted { origin });
        }
        match &tab.grant {
            Some((shared, epoch)) if *shared == origin => Some(Access::Shared {
                origin,
                epoch: *epoch,
            }),
            _ => None,
        }
    }

    pub(crate) fn toggle_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_agent_for(self.active, window, cx);
    }

    /// The agent button and a tab's agent icon: stops trusting the site, or
    /// stops sharing the tab, or offers to share it or trust its site.
    pub(crate) fn toggle_agent_for(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = &self.tabs[index];
        let id = tab.id;
        match self.agent_access(tab) {
            Some(Access::Trusted { origin }) => self.stop_trusting(id, origin, window, cx),
            Some(Access::Shared { .. }) => {
                self.tabs[index].grant = None;
                cx.notify();
            }
            None => self.offer_sharing(index, window, cx),
        }
    }

    fn offer_sharing(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &self.tabs[index];
        // Grants are origin-based and a file has none; local files can also
        // be sensitive. Refuse explicitly rather than by accident.
        if crate::model::is_file_url(&tab.saved.url) {
            self.notice = Some("Local files can’t be shared with agents.".into());
            cx.notify();
            return;
        }
        let origin = Self::tab_site(tab).filter(|_| tab.page.is_some());
        let Some(origin) = origin.filter(|_| !tab.private && !tab.loading) else {
            self.notice = Some(
                "Open a regular web page before sharing. Private tabs cannot be shared.".into(),
            );
            cx.notify();
            return;
        };
        let (id, host) = (tab.id, crate::model::display_host(&origin));
        let answer = window.prompt(
            PromptLevel::Warning,
            "Share this tab with agents?",
            Some(&format!(
                "Connected local agents may read what you can see in this tab on {host} and click and type in it; Gradient asks you before anything that may submit or change something. Content may be sent to their model provider. Sharing ends when you stop it, the tab leaves {host}, or Gradient quits.\n\nTrusting {host} instead gives agents full access in every tab on it, as for a site you're developing."
            )),
            &["Cancel", "Share Tab", "Trust Site…"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| match answer.await {
            Ok(1) => {
                let _ = this.update(cx, |this, cx| {
                    let epoch = this.grant_epoch + 1;
                    let shareable = this.tab_mut(id).is_some_and(|tab| {
                        !tab.private
                            && !tab.loading
                            && Self::tab_site(tab).as_ref() == Some(&origin)
                    });
                    if shareable {
                        this.grant_epoch = epoch;
                        if let Some(tab) = this.tab_mut(id) {
                            tab.grant = Some((origin, epoch));
                        }
                        cx.notify();
                    }
                });
            }
            Ok(2) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.offer_trust(id, origin, window, cx)
                });
            }
            _ => {}
        })
        .detach();
    }

    fn offer_trust(
        &mut self,
        id: u64,
        origin: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = crate::model::display_host(&origin);
        let answer = window.prompt(
            PromptLevel::Critical,
            &format!("Trust agents on “{host}”?"),
            Some(&format!(
                "Connected agents get full access to {host} in every tab, until you stop trusting it: screenshots, the page's code and data, console and network logs, running JavaScript, and clicking, typing and uploading files without asking. Only trust a site you're developing or fully control."
            )),
            &["Cancel", &format!("Trust {host}")],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    let Some(profile) = this.profile_of(id) else {
                        return;
                    };
                    if let Err(error) = this.permissions.allow(&profile, &origin, &[Kind::Agents]) {
                        this.notice = Some(error.to_string());
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn stop_trusting(
        &mut self,
        id: u64,
        origin: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = crate::model::display_host(&origin);
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Stop trusting agents on “{host}”?"),
            Some(&format!(
                "Agents lose access to {host} in every tab. You can trust it again later."
            )),
            &["Cancel", "Stop Trusting"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    let Some(profile) = this.profile_of(id) else {
                        return;
                    };
                    if let Err(error) = this.permissions.revoke(&profile, &origin, Kind::Agents) {
                        this.notice = Some(error.to_string());
                    }
                    // Stopping means stopping: no tab on it stays shared.
                    for tab in this.tabs.iter_mut() {
                        if tab
                            .grant
                            .as_ref()
                            .is_some_and(|(shared, _)| *shared == origin)
                        {
                            tab.grant = None;
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Shared tabs in every Space keep their grant only while on its origin.
    pub(crate) fn revoke_changed_origins(&mut self) {
        let parked = self
            .parked
            .values_mut()
            .flat_map(|parked| parked.tabs.iter_mut());
        for tab in self.tabs.iter_mut().chain(parked) {
            if let Some((origin, _)) = &tab.grant
                && Self::tab_site(tab).as_ref() != Some(origin)
            {
                tab.grant = None;
            }
        }
    }

    pub(crate) fn agent_command(
        &mut self,
        command: mcp::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Instant::now() > command.expires {
            let _ = command.reply.send(Err("Request expired.".into()));
            return;
        }
        self.revoke_changed_origins();
        if let Err(error) = self.start_agent(&command, window, cx) {
            let _ = command.reply.send(Err(error.to_string()));
        }
        cx.notify();
    }

    fn start_agent(
        &mut self,
        command: &mcp::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let args = &command.args;
        let schema = mcp::tools()
            .into_iter()
            .find(|tool| tool["name"] == command.name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool"))?;
        let fields = schema["inputSchema"]["properties"].as_object().unwrap();
        let arguments = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?;
        anyhow::ensure!(
            arguments.len() == fields.len() && arguments.keys().all(|key| fields.contains_key(key)),
            "Unexpected or missing arguments"
        );
        if command.name == "browser_tabs" {
            let tabs: Vec<Value> = self
                .all_tabs()
                .filter_map(|tab| {
                    let access = self.agent_access(tab)?;
                    let space = self
                        .space_of(tab.id)
                        .map(|index| self.spaces[index].name.clone())
                        .unwrap_or_default();
                    Some(json!({
                        "tab_id": tab.id,
                        "title": tab.saved.title,
                        "url": tab.saved.url,
                        "space": space,
                        "access": if access.trusted() { "trusted" } else { "shared" },
                    }))
                })
                .collect();
            let _ = command.reply.send(Ok(json!({ "tabs": tabs })));
            return Ok(());
        }
        let id = args["tab_id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("tab_id must be a positive integer"))?;
        let unavailable = || anyhow::anyhow!("Tab is unavailable or not shared");
        let tab = self
            .all_tabs()
            .find(|tab| tab.id == id)
            .ok_or_else(unavailable)?;
        let access = self.agent_access(tab).ok_or_else(unavailable)?;
        anyhow::ensure!(
            !mcp::needs_trust(&command.name) || access.trusted(),
            "{} needs the site to be trusted. The user can trust it from the agent button in Gradient.",
            command.name
        );
        anyhow::ensure!(!tab.loading, "Wait for the page to finish loading");
        let page = tab
            .page
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Tab is suspended"))?;
        let reads = [
            "browser_read_page",
            "browser_screenshot",
            "browser_read_dom",
            "browser_console",
            "browser_network",
        ];
        anyhow::ensure!(
            reads.contains(&command.name.as_str())
                || (!self.discard_pending && !page.navigation_pending()),
            "Wait for the browser's pending page check or confirmation"
        );
        // Every agent action shows as a brief brightening of the card's glow.
        page.agent_flare();
        let name = command.name.clone();
        match name.as_str() {
            "browser_navigate" => {
                let url = args["url"].as_str().unwrap_or_default();
                let allowed = match &access {
                    Access::Shared { origin, .. } => mcp::origin(url).as_ref() == Some(origin),
                    Access::Trusted { .. } => mcp::origin(url).is_some(),
                };
                anyhow::ensure!(
                    url.len() <= 8192 && allowed,
                    "Navigation must stay within the shared site"
                );
                let trusted = access.trusted();
                if let Some(tab) = self.tab_mut(id) {
                    tab.revision = tab.revision.wrapping_add(1);
                    // Shared tabs keep the unsaved-work prompt for the user.
                    tab.agent_navigated = trusted.then(Instant::now);
                }
                page.load_url(url)?;
                let _ = command
                    .reply
                    .send(Ok(json!({"navigation_requested": true})));
                return Ok(());
            }
            "browser_console" | "browser_network" => {
                let (console, network) = page.agent_logs();
                let _ = command.reply.send(Ok(if name == "browser_console" {
                    console
                } else {
                    network
                }));
                return Ok(());
            }
            _ => {}
        }
        let job = Job {
            id,
            access,
            page,
            args: args.clone(),
            browser: cx.entity().downgrade(),
        };
        let reply = command.reply.clone();
        cx.spawn_in(window, async move |_, cx| {
            let result = job.run(&name, cx).await;
            let _ = reply.send(result);
        })
        .detach();
        Ok(())
    }

    /// For automated checks of debug builds only: shares or trusts a page
    /// on the loopback fixture server as it loads, without the prompts.
    #[cfg(debug_assertions)]
    pub(crate) fn test_agent_access(&mut self, id: u64) {
        let Ok(level) = std::env::var("GRADIENT_TEST_AGENT") else {
            return;
        };
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let Some(origin) =
            Self::tab_site(tab).filter(|origin| origin.starts_with("http://127.0.0.1:"))
        else {
            return;
        };
        match level.as_str() {
            "trust" => {
                if let Some(profile) = self.profile_of(id) {
                    let _ = self.permissions.allow(&profile, &origin, &[Kind::Agents]);
                }
            }
            "share" => {
                self.grant_epoch += 1;
                let epoch = self.grant_epoch;
                if let Some(tab) = self.tab_mut(id) {
                    tab.grant = Some((origin, epoch));
                }
            }
            _ => {}
        }
    }
}

/// One agent action in progress. It checks the tab's access again before
/// each step, so stopping access in Gradient stops it partway.
struct Job {
    id: u64,
    access: Access,
    page: Rc<engine::Page>,
    args: Value,
    browser: WeakEntity<Browser>,
}

impl Job {
    async fn run(&self, name: &str, cx: &mut AsyncWindowContext) -> Result<Value, String> {
        match name {
            "browser_read_page" => self.script(mcp::READ, json!({}), cx).await,
            "browser_read_dom" => self.script(mcp::READ_DOM, json!({}), cx).await,
            "browser_click" => self.click(cx).await,
            "browser_fill" => self.fill(cx).await,
            "browser_screenshot" => {
                let (send, receive) = async_channel::bounded(1);
                self.page.agent_screenshot(move |result| {
                    let _ = send.try_send(result);
                });
                let png = receive
                    .recv()
                    .await
                    .map_err(|_| "The page closed.".to_string())??;
                Ok(json!({ "png": png }))
            }
            "browser_eval" => {
                let code = self.args["code"].as_str().unwrap_or_default();
                if code.len() > 200_000 {
                    return Err("The code is too long.".into());
                }
                self.call(code, json!({}), true).await
            }
            "browser_press_key" => {
                self.ensure_access(cx)?;
                if !self.page.agent_can_input() {
                    return Err("Bring the tab to the front to press keys in it.".into());
                }
                let combo = self.args["key"].as_str().unwrap_or_default();
                let mut parts: Vec<&str> = combo.split('+').collect();
                // "+" itself, or "cmd++".
                let key = match parts.pop() {
                    Some("") if combo.ends_with('+') => {
                        parts.pop();
                        "+"
                    }
                    Some(key) => key,
                    None => return Err("Name a key.".into()),
                };
                self.page.agent_press(key, &parts)?;
                Ok(json!({ "pressed": combo }))
            }
            "browser_upload" => self.upload(cx).await,
            _ => Err("Unknown tool".into()),
        }
    }

    fn ensure_access(&self, cx: &mut AsyncWindowContext) -> Result<(), String> {
        let current = self
            .browser
            .update(cx, |browser, _| {
                browser
                    .all_tabs()
                    .find(|tab| tab.id == self.id)
                    .and_then(|tab| browser.agent_access(tab))
            })
            .ok()
            .flatten();
        if current.as_ref() == Some(&self.access) {
            Ok(())
        } else {
            Err("Agent access was stopped or the tab changed site.".into())
        }
    }

    /// Runs a script in the agent's world (or the page's) and parses its
    /// result.
    async fn call(&self, body: &str, args: Value, page_world: bool) -> Result<Value, String> {
        let (send, receive) = async_channel::bounded(1);
        self.page
            .agent_call(body, &args.to_string(), page_world, move |result| {
                let _ = send.try_send(result);
            });
        let text = receive
            .recv()
            .await
            .map_err(|_| "The page closed.".to_string())??;
        serde_json::from_str(&text).map_err(|_| "Could not read the page's result.".into())
    }

    /// One of the agent scripts, with the expected site and the call's own
    /// arguments as its inputs.
    async fn script(
        &self,
        script: &str,
        extra: Value,
        cx: &mut AsyncWindowContext,
    ) -> Result<Value, String> {
        self.ensure_access(cx)?;
        let mut args = self.args.clone();
        args["origin"] = json!(self.access.origin());
        if let (Some(args), Some(extra)) = (args.as_object_mut(), extra.as_object()) {
            args.extend(extra.clone());
        }
        let result = self.call(&mcp::body(script), args, false).await;
        self.ensure_access(cx)?;
        result
    }

    async fn click(&self, cx: &mut AsyncWindowContext) -> Result<Value, String> {
        let aim = self.script(mcp::AIM_CLICK, json!({}), cx).await?;
        if !self.access.trusted() && aim["consequential"].as_bool() == Some(true) {
            self.ask(aim["label"].as_str().unwrap_or("this"), cx)
                .await?;
        }
        if !self.page.agent_can_input() {
            return self.script(mcp::CLICK_SCRIPTED, json!({}), cx).await;
        }
        self.ensure_access(cx)?;
        let (x, y) = (
            aim["x"].as_f64().unwrap_or(0.),
            aim["y"].as_f64().unwrap_or(0.),
        );
        self.page.agent_click(x, y)?;
        Ok(json!({ "clicked": true, "input": "native" }))
    }

    async fn fill(&self, cx: &mut AsyncWindowContext) -> Result<Value, String> {
        let value = self.args["value"].as_str().unwrap_or_default();
        if value.len() > 20_000 {
            return Err("The value is too long.".into());
        }
        if self.page.agent_can_input() {
            let aim = self.script(mcp::AIM_FILL, json!({}), cx).await?;
            let (x, y) = (
                aim["x"].as_f64().unwrap_or(0.),
                aim["y"].as_f64().unwrap_or(0.),
            );
            self.page.agent_click(x, y)?;
            let focused = self.script(mcp::SELECT_FIELD, json!({}), cx).await?;
            if focused.as_bool() == Some(true) {
                self.page.agent_type(value)?;
                return Ok(json!({ "filled": true, "input": "native" }));
            }
        }
        self.script(mcp::FILL_SCRIPTED, json!({}), cx).await
    }

    async fn upload(&self, cx: &mut AsyncWindowContext) -> Result<Value, String> {
        let paths = self.args["paths"]
            .as_array()
            .ok_or("paths must be a list")?;
        let mut files = Vec::new();
        let mut total = 0;
        for path in paths {
            let path = std::path::Path::new(path.as_str().ok_or("Each path must be text")?);
            if !path.is_absolute() {
                return Err(format!("{} isn't an absolute path.", path.display()));
            }
            let meta =
                std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
            if !meta.is_file() {
                return Err(format!("{} isn't a file.", path.display()));
            }
            total += meta.len();
            if total > UPLOAD_LIMIT {
                return Err("The files are more than 25 MB in all.".into());
            }
            let bytes =
                std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push(json!({ "name": name, "type": mime(path), "data": mcp::base64(&bytes) }));
        }
        self.script(mcp::UPLOAD, json!({ "files": files }), cx)
            .await
    }

    /// Asks the user before an untrusted agent's consequential click.
    async fn ask(&self, label: &str, cx: &mut AsyncWindowContext) -> Result<(), String> {
        let host = crate::model::display_host(self.access.origin());
        let label: String = label.chars().take(80).collect();
        let answer = self
            .browser
            .update_in(cx, |browser, window, cx| {
                let title = browser
                    .all_tabs()
                    .find(|tab| tab.id == self.id)
                    .map(|tab| tab.saved.title.clone())
                    .unwrap_or_default();
                window.prompt(
                    PromptLevel::Warning,
                    &format!("Allow an agent to click “{label}”?"),
                    Some(&format!(
                        "In “{title}” on {host}. This may submit a form or make a change you can't undo."
                    )),
                    &["Don't Allow", "Allow"],
                    cx,
                )
            })
            .map_err(|_| "The browser closed.".to_string())?;
        if answer.await != Ok(1) {
            return Err("The user didn't allow this click.".into());
        }
        self.ensure_access(cx)
    }
}

/// A file's type, from its extension, for the page's `File` object.
fn mime(path: &std::path::Path) -> &'static str {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        _ => "",
    }
}
