use crate::{Browser, engine, mcp};
use gpui::{Context, PromptLevel, Window};
use serde_json::{Value, json};
use std::time::Instant;

impl Browser {
    pub(crate) fn toggle_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[self.active];
        if tab.grant.take().is_some() {
            self.notice = Some("Agent access revoked.".into());
            cx.notify();
            return;
        }
        let origin = tab
            .page
            .as_ref()
            .and_then(|p| p.raw.url().ok())
            .and_then(|u| mcp::origin(&u));
        let Some(origin) = origin.filter(|_| !tab.private && !tab.loading) else {
            self.notice = Some(
                "Open a regular web page before sharing. Private tabs cannot be shared.".into(),
            );
            cx.notify();
            return;
        };
        let id = tab.id;
        let description = format!(
            "Connected local agents may read and interact with this tab on {origin}. Content may be sent to their model provider. Access ends when revoked, when the tab changes site, or when Browser quits."
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Share this tab with agents?",
            Some(&description),
            &["Cancel", "Share tab"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |this, _, cx| {
                    if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id)
                        && !tab.private
                        && !tab.loading
                        && tab
                            .page
                            .as_ref()
                            .and_then(|p| p.raw.url().ok())
                            .and_then(|u| mcp::origin(&u))
                            .as_ref()
                            == Some(&origin)
                    {
                        this.grant_epoch += 1;
                        tab.grant = Some((origin, this.grant_epoch));
                        this.notice =
                            Some("Tab shared. Use Revoke agent access to disconnect it.".into());
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn revoke_changed_origins(&mut self) {
        for tab in &mut self.tabs {
            if let Some((origin, _)) = &tab.grant
                && tab
                    .page
                    .as_ref()
                    .and_then(|p| p.raw.url().ok())
                    .and_then(|u| mcp::origin(&u))
                    .as_ref()
                    != Some(origin)
            {
                tab.grant = None;
            }
        }
    }

    pub(crate) fn agent_command(&mut self, command: mcp::Command, cx: &mut Context<Self>) {
        if Instant::now() > command.expires {
            let _ = command.reply.send(Err("Request expired.".into()));
            return;
        }
        self.revoke_changed_origins();
        let result = self.dispatch_agent(&command);
        match result {
            Ok(Some(value)) => {
                let _ = command.reply.send(Ok(value));
            }
            Ok(None) => {}
            Err(error) => {
                let _ = command.reply.send(Err(error.to_string()));
            }
        }
        cx.notify();
    }

    fn dispatch_agent(&mut self, command: &mcp::Command) -> anyhow::Result<Option<Value>> {
        let args = &command.args;
        let schema = mcp::tools()
            .into_iter()
            .find(|t| t["name"] == command.name)
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
            return Ok(Some(
                json!({"tabs":self.tabs.iter().filter(|t|t.grant.is_some()).map(|t|json!({"tab_id":t.id,"title":t.saved.title,"url":t.saved.url})).collect::<Vec<_>>()}),
            ));
        }
        let id = args["tab_id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("tab_id must be a positive integer"))?;
        let tab = self
            .tabs
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("Tab is unavailable or not shared"))?;
        let (origin, epoch) = tab
            .grant
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Tab is unavailable or not shared"))?;
        anyhow::ensure!(
            !tab.private && !tab.loading,
            "Wait for the shared page to finish loading"
        );
        let page = tab
            .page
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Tab is suspended"))?;
        anyhow::ensure!(
            mcp::origin(&page.raw.url()?).as_ref() == Some(&origin),
            "Tab changed origin"
        );
        if command.name == "browser_navigate" {
            let url = args["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("url must be a string"))?;
            anyhow::ensure!(
                url.len() <= 8192 && mcp::origin(url).as_ref() == Some(&origin),
                "Navigation must stay within the shared origin"
            );
            page.raw.load_url(url)?;
            tab.loading = true;
            return Ok(Some(json!({"navigation_started":true})));
        }
        let script = mcp::script(&command.name, args, &origin)?;
        let events = self.events.clone();
        let reply = command.reply.clone();
        page.raw
            .evaluate_script_with_callback(&script, move |value| {
                let _ = events.try_send(engine::Event::AgentResult {
                    id,
                    epoch,
                    origin: origin.clone(),
                    reply: reply.clone(),
                    value,
                });
            })?;
        self.notice = Some(format!("Agent used {}.", command.name));
        Ok(None)
    }

    pub(crate) fn finish_agent(
        &mut self,
        id: u64,
        epoch: u64,
        origin: String,
        reply: std::sync::mpsc::SyncSender<Result<Value, String>>,
        value: String,
    ) {
        self.revoke_changed_origins();
        let valid = self
            .tabs
            .iter()
            .any(|tab| tab.id == id && tab.grant.as_ref() == Some(&(origin.clone(), epoch)));
        let result = if !valid {
            Err("Agent access was revoked or the tab changed origin.".into())
        } else {
            serde_json::from_str::<Value>(&value)
                .map_err(|_| "Could not read page result.".into())
                .and_then(|value| {
                    if let Some(error) = value.get("error").and_then(Value::as_str) {
                        Err(error.to_string())
                    } else {
                        Ok(value)
                    }
                })
        };
        let _ = reply.send(result);
    }
}
