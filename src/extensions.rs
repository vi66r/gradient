//! Installed web extensions, per Space. Each saved Space keeps its own list,
//! as it keeps its own website data; adding an extension copies it into
//! Browser's data folder (`extensions/<install ID>/`), so the original can
//! move or go. The Private Space runs only the extensions the user allowed
//! there, from any Space, with storage that is forgotten when it closes.
//! The engine side (controllers, tabs, toolbar actions) is
//! `engine/extensions.rs`; the design is in docs/extensions.md.
use crate::{Browser, engine};
use anyhow::{Context as _, Result, bail};
use gpui::{Action, App, Bounds, Context, Image, ImageFormat, Pixels, Point, PromptLevel, Window};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// WebKit ignores an extension's blocking rules when it has this permission
/// and a toolbar action (uBlock Origin Lite blocks nothing). Block rules
/// don't need host access, so it is replaced with plain
/// `declarativeNetRequest` at install; only redirect rules are lost.
const HOST_ACCESS: &str = "declarativeNetRequestWithHostAccess";
/// Copying stops past these; no real extension comes close.
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The folder holding `manifest.json`, relative to `extensions/`. Its
    /// first component is always `id`.
    pub folder: String,
    pub enabled: bool,
    /// Also runs in the Private Space.
    #[serde(default)]
    pub private: bool,
    /// What the user granted, as WebKit names permissions and patterns.
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    /// Space UUID → its extensions, in toolbar order.
    spaces: BTreeMap<String, Vec<Installed>>,
}

pub(crate) struct Store {
    /// `None` when the saved file couldn't be read: changes then last only
    /// this session and the original file is preserved.
    path: Option<PathBuf>,
    root: PathBuf,
    saved: Saved,
}

fn space_key(space: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*space).to_string()
}

impl Store {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("extensions.json");
        let root = dir.join("extensions");
        let read = || -> Result<Saved> {
            if !path.exists() {
                return Ok(Saved {
                    version: 1,
                    ..Saved::default()
                });
            }
            anyhow::ensure!(
                std::fs::metadata(&path)?.len() <= 1024 * 1024,
                "Extensions file is too large."
            );
            let saved: Saved = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(saved.version == 1, "Unknown extensions version.");
            Ok(saved)
        };
        match read() {
            Ok(saved) => Self {
                path: Some(path),
                root,
                saved,
            },
            Err(error) => {
                log::error!("Installed extensions unavailable: {error}");
                Self {
                    path: None,
                    root,
                    saved: Saved {
                        version: 1,
                        ..Saved::default()
                    },
                }
            }
        }
    }

    /// Where installed extensions' files live.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn space(&self, space: &[u8; 16]) -> &[Installed] {
        self.saved
            .spaces
            .get(&space_key(space))
            .map_or(&[], Vec::as_slice)
    }

    pub fn get(&self, space: &[u8; 16], id: &str) -> Option<&Installed> {
        self.space(space)
            .iter()
            .find(|installed| installed.id == id)
    }

    /// The Space an install belongs to, among `spaces`.
    pub fn owner(&self, spaces: &[[u8; 16]], id: &str) -> Option<[u8; 16]> {
        spaces
            .iter()
            .find(|space| self.get(space, id).is_some())
            .copied()
    }

    pub fn add(&mut self, space: &[u8; 16], installed: Installed) -> Result<()> {
        self.saved
            .spaces
            .entry(space_key(space))
            .or_default()
            .push(installed);
        self.write()
    }

    pub fn update(
        &mut self,
        space: &[u8; 16],
        id: &str,
        change: impl FnOnce(&mut Installed),
    ) -> Result<()> {
        let Some(installed) = self
            .saved
            .spaces
            .get_mut(&space_key(space))
            .and_then(|list| list.iter_mut().find(|installed| installed.id == id))
        else {
            return Ok(());
        };
        change(installed);
        self.write()
    }

    /// Drops an install and deletes its files.
    pub fn remove(&mut self, space: &[u8; 16], id: &str) -> Result<()> {
        let key = space_key(space);
        let Some(list) = self.saved.spaces.get_mut(&key) else {
            return Ok(());
        };
        let removed: Vec<Installed> = list
            .extract_if(.., |installed| installed.id == id)
            .collect();
        if list.is_empty() {
            self.saved.spaces.remove(&key);
        }
        for installed in &removed {
            self.delete_files(&installed.id);
        }
        self.write()
    }

    /// A Space was deleted: its extensions and their files go with it.
    pub fn forget_space(&mut self, space: &[u8; 16]) -> Result<()> {
        let Some(removed) = self.saved.spaces.remove(&space_key(space)) else {
            return Ok(());
        };
        for installed in &removed {
            self.delete_files(&installed.id);
        }
        self.write()
    }

    /// Deletes an install's folder; only ever a UUID-named child of `root`.
    pub fn delete_files(&self, id: &str) {
        if uuid::Uuid::parse_str(id).is_err() {
            return;
        }
        let dir = self.root.join(id);
        if dir.exists()
            && let Err(error) = std::fs::remove_dir_all(&dir)
        {
            log::warn!("Removing an extension's files: {error}");
        }
    }

    /// The engine's view of a Space's enabled extensions.
    pub fn specs(&self, space: &[u8; 16]) -> Vec<engine::ExtensionSpec> {
        self.space(space)
            .iter()
            .filter(|installed| installed.enabled)
            .map(|installed| self.spec(installed))
            .collect()
    }

    /// What runs in the Private Space: every enabled extension allowed
    /// there, from the Spaces in order, once per name.
    pub fn private_specs(&self, spaces: &[[u8; 16]]) -> Vec<engine::ExtensionSpec> {
        let mut names = Vec::new();
        let mut specs = Vec::new();
        for space in spaces {
            for installed in self.space(space) {
                if installed.enabled && installed.private && !names.contains(&installed.name) {
                    names.push(installed.name.clone());
                    specs.push(self.spec(installed));
                }
            }
        }
        specs
    }

    fn spec(&self, installed: &Installed) -> engine::ExtensionSpec {
        engine::ExtensionSpec {
            install: installed.id.clone(),
            dir: self.root.join(&installed.folder),
            permissions: installed.permissions.clone(),
            patterns: installed.patterns.clone(),
        }
    }

    fn write(&self) -> Result<()> {
        use std::io::Write;
        let Some(path) = &self.path else {
            bail!("Saved extensions are unavailable; this change lasts until quit.");
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(&serde_json::to_vec_pretty(&self.saved)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

/// An extension copied into Browser's data folder, not yet added.
#[derive(Debug)]
pub(crate) struct Staged {
    pub id: String,
    /// See `Installed::folder`.
    pub folder: String,
    pub dir: PathBuf,
}

/// Copies an unpacked extension folder, or unpacks a `.zip`, into `root`
/// under a new install ID, and applies the manifest workaround.
pub(crate) fn stage(source: &Path, root: &Path) -> Result<Staged> {
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(root)?;
    let dest = root.join(&id);
    let result = (|| {
        if source.is_dir() {
            copy_tree(source, &dest, &mut Budget::default())?;
        } else if source
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            unzip(source, &dest)?;
        } else {
            bail!("Choose an extension’s folder or a .zip file.");
        }
        let folder = manifest_folder(&dest)?;
        let dir = dest.join(&folder);
        let manifest = dir.join("manifest.json");
        if let Ok(text) = std::fs::read(&manifest)
            && let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&text)
            && adjust_manifest(&mut value)
        {
            let name = value
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or("");
            log::warn!(
                "Dropped {HOST_ACCESS} from {name}'s manifest: WebKit ignores its blocking rules with that permission and a toolbar action. Its redirect rules won't apply."
            );
            std::fs::write(&manifest, serde_json::to_vec_pretty(&value)?)?;
        }
        let folder = if folder.as_os_str().is_empty() {
            id.clone()
        } else {
            format!("{id}/{}", folder.to_string_lossy())
        };
        Ok(Staged {
            id: id.clone(),
            folder,
            dir,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dest);
    }
    result
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

impl Budget {
    fn spend(&mut self, bytes: u64) -> Result<()> {
        self.files += 1;
        self.bytes += bytes;
        if self.files > MAX_FILES || self.bytes > MAX_BYTES {
            bail!("That extension is too large.");
        }
        Ok(())
    }
}

/// Copies files and folders only: links are skipped, so nothing outside the
/// extension can become part of it.
fn copy_tree(source: &Path, dest: &Path, budget: &mut Budget) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if kind.is_dir() {
            budget.spend(0)?;
            copy_tree(&entry.path(), &target, budget)?;
        } else if kind.is_file() {
            budget.spend(entry.metadata()?.len())?;
            std::fs::copy(entry.path(), &target)?;
        } else {
            log::warn!(
                "Skipped {} while copying an extension",
                entry.path().display()
            );
        }
    }
    Ok(())
}

/// Unpacks with the system's `ditto`, after checking that no entry would
/// land outside `dest`; links it created are removed afterwards.
fn unzip(archive: &Path, dest: &Path) -> Result<()> {
    let listing = std::process::Command::new("/usr/bin/zipinfo")
        .arg("-1")
        .arg(archive)
        .output()
        .context("The archive couldn’t be read.")?;
    if !listing.status.success() {
        bail!("That file isn’t a readable .zip archive.");
    }
    let names = String::from_utf8_lossy(&listing.stdout);
    if names.lines().count() > MAX_FILES {
        bail!("That extension is too large.");
    }
    for name in names.lines() {
        if !safe_entry(name) {
            bail!("The archive contains a file outside its own folder.");
        }
    }
    std::fs::create_dir_all(dest)?;
    let status = std::process::Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(dest)
        .status()
        .context("The archive couldn’t be unpacked.")?;
    if !status.success() {
        bail!("The archive couldn’t be unpacked.");
    }
    remove_links(dest, &mut Budget::default())
}

fn safe_entry(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn remove_links(dir: &Path, budget: &mut Budget) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            std::fs::remove_file(entry.path())?;
        } else if kind.is_dir() {
            budget.spend(0)?;
            remove_links(&entry.path(), budget)?;
        } else {
            budget.spend(entry.metadata()?.len())?;
        }
    }
    Ok(())
}

/// The folder (relative to `dest`) holding `manifest.json`: `dest` itself,
/// or the one folder an archive wrapped it in.
fn manifest_folder(dest: &Path) -> Result<PathBuf> {
    if dest.join("manifest.json").is_file() {
        return Ok(PathBuf::new());
    }
    let folders: Vec<PathBuf> = std::fs::read_dir(dest)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| PathBuf::from(entry.file_name()))
        .filter(|name| name != Path::new("__MACOSX"))
        .collect();
    match folders.as_slice() {
        [only] if dest.join(only).join("manifest.json").is_file() => Ok(only.clone()),
        _ => bail!("That isn’t a web extension: it has no manifest.json."),
    }
}

/// Replaces `declarativeNetRequestWithHostAccess` (see `HOST_ACCESS`).
/// Returns whether the manifest changed.
pub(crate) fn adjust_manifest(manifest: &mut serde_json::Value) -> bool {
    let mut changed = false;
    for key in ["permissions", "optional_permissions"] {
        let Some(list) = manifest.get_mut(key).and_then(|list| list.as_array_mut()) else {
            continue;
        };
        let before = list.len();
        list.retain(|permission| permission.as_str() != Some(HOST_ACCESS));
        if list.len() != before {
            changed = true;
            if !list
                .iter()
                .any(|permission| permission.as_str() == Some("declarativeNetRequest"))
            {
                list.push("declarativeNetRequest".into());
            }
        }
    }
    changed
}

/// The host a match pattern or URL names: `*` for every site, without a
/// leading `*.`; `None` for extension and other non-web patterns.
fn pattern_host(pattern: &str) -> Option<&str> {
    if pattern == "<all_urls>" {
        return Some("*");
    }
    let (scheme, rest) = pattern.split_once("://")?;
    if !matches!(scheme, "*" | "http" | "https" | "ws" | "wss") {
        return None;
    }
    let host = rest.split('/').next()?.split(':').next()?;
    Some(host.strip_prefix("*.").unwrap_or(host)).filter(|host| !host.is_empty())
}

/// What granting these lets an extension do, in the prompts' words. Grants
/// that read or change nothing of the user's (storage, alarms, `activeTab`)
/// say nothing, as in Chrome.
pub(crate) fn describe(permissions: &[String], patterns: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let hosts: Vec<&str> = patterns
        .iter()
        .filter_map(|pattern| pattern_host(pattern))
        .collect();
    if hosts.contains(&"*") {
        lines.push("Read and change everything on every website you visit".into());
    } else if !hosts.is_empty() {
        let mut hosts = hosts;
        hosts.sort();
        hosts.dedup();
        lines.push(format!("Read and change your data on {}", sites(&hosts)));
    }
    for permission in permissions {
        let line = match permission.as_str() {
            "tabs" | "webNavigation" => "See the address of every page you open",
            "declarativeNetRequest" | HOST_ACCESS => "Block content on any page",
            "declarativeNetRequestFeedback" => "See what it blocks on the pages you visit",
            "webRequest" => "See the requests pages make",
            "nativeMessaging" => "Talk to apps on this Mac",
            "clipboardRead" => "Read what you copy",
            "clipboardWrite" => "Change what you copy",
            "notifications" => "Show notifications",
            "history" => "Read and change your browsing history",
            "downloads" => "Manage your downloads",
            "bookmarks" => "Read and change your bookmarks",
            _ => continue,
        };
        if !lines.iter().any(|existing| existing == line) {
            lines.push(line.into());
        }
    }
    lines
}

/// "a.com", "a.com and b.com", "a.com, b.com and 3 more sites".
fn sites(hosts: &[&str]) -> String {
    match hosts {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, second] => format!("{first} and {second}"),
        [first, second, third] => format!("{first}, {second} and {third}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more sites", rest.len()),
    }
}

/// "Read and change…" becomes "read and change…" inside a question.
fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

fn bullets(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| format!("• {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A toolbar button, ready to draw.
#[derive(Clone)]
pub(crate) struct ActionButton {
    pub install: String,
    pub label: String,
    pub badge: String,
    pub enabled: bool,
    pub icon: Option<Arc<Image>>,
}

/// Commands from an extension's menu (the strip's extensions button, or a
/// right-click on its toolbar button).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub(crate) enum ExtensionCommand {
    Toggle,
    TogglePrivate,
    Permissions,
    Remove,
}

#[derive(Clone, PartialEq, serde::Deserialize, Action)]
#[action(namespace = browser, no_json)]
pub(crate) struct ExtensionMenu {
    pub space: [u8; 16],
    pub install: String,
    pub command: ExtensionCommand,
}

impl Browser {
    /// The saved Spaces, in order (the Private Space's sources).
    fn saved_spaces(&self) -> Vec<[u8; 16]> {
        self.spaces
            .iter()
            .filter(|space| !space.private)
            .map(|space| space.id)
            .collect()
    }

    /// Hands every Space's enabled extensions to the engine (at launch).
    pub(crate) fn start_extensions(&self, cx: &App) {
        if !engine::extensions_available() {
            return;
        }
        for space in self.saved_spaces() {
            self.apply_extensions(space, cx);
        }
    }

    /// Reloads one Space's extensions after a change, and the Private
    /// Space's, which draws on every Space.
    pub(crate) fn apply_extensions(&self, space: [u8; 16], cx: &App) {
        engine::set_extensions(
            space,
            false,
            self.extensions.specs(&space),
            self.events.clone(),
            cx,
        );
        self.apply_private_extensions(cx);
    }

    pub(crate) fn apply_private_extensions(&self, cx: &App) {
        if let Some(private) = self.spaces.iter().find(|space| space.private) {
            engine::set_extensions(
                private.id,
                true,
                self.extensions.private_specs(&self.saved_spaces()),
                self.events.clone(),
                cx,
            );
        }
    }

    /// Describes every Space's tabs to its extensions. Called each frame;
    /// the engine only acts on changes.
    pub(crate) fn sync_extension_tabs(&self) {
        for space in &self.spaces {
            let (tabs, active) = if space.id == self.current_space().id {
                (&self.tabs[..], Some(self.active))
            } else {
                match self.parked.get(&space.id) {
                    Some(parked) => (&parked.tabs[..], Some(parked.active)),
                    None => (&[][..], None),
                }
            };
            let active = active.and_then(|index| tabs.get(index)).map(|tab| tab.id);
            engine::sync_extension_tabs(
                space.id,
                tabs.iter().map(|tab| engine::ExtensionTab {
                    id: tab.id,
                    page: tab.page.as_deref(),
                    title: &tab.saved.title,
                    url: (tab.saved.url != crate::model::START).then_some(tab.saved.url.as_str()),
                    loading: tab.loading,
                }),
                active,
            );
        }
    }

    /// Rereads the toolbar buttons for the tab in front.
    pub(crate) fn refresh_extension_actions(&mut self) {
        let tab = self.tabs.get(self.active).map(|tab| tab.id);
        let actions = engine::extension_actions(self.current_space().id, tab);
        self.extension_actions = actions
            .into_iter()
            .map(|action| ActionButton {
                icon: action
                    .icon
                    .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))),
                install: action.install,
                label: action.label,
                badge: action.badge,
                enabled: action.enabled,
            })
            .collect();
    }

    /// The strip's extensions button is shown when the Space has any, or
    /// when the Private Space runs some.
    pub(crate) fn has_extensions(&self) -> bool {
        let space = self.current_space();
        if space.private {
            !self
                .extensions
                .private_specs(&self.saved_spaces())
                .is_empty()
        } else {
            !self.extensions.space(&space.id).is_empty()
        }
    }

    pub(crate) fn perform_extension_action(&mut self, install: &str) {
        let anchor = self
            .extension_anchors
            .borrow()
            .get(install)
            .copied()
            .unwrap_or_default();
        let tab = self.tabs.get(self.active).map(|tab| tab.id);
        engine::perform_extension_action(self.current_space().id, install, tab, anchor);
    }

    /// Tools ▸ Add Extension…: an unpacked extension's folder or a `.zip`.
    pub(crate) fn add_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !engine::extensions_available() {
            self.notice = Some("Extensions need macOS 15.4 or later.".into());
            cx.notify();
            return;
        }
        if self.current_space().private {
            self.notice = Some(
                "Add extensions in one of your Spaces, then allow them in the Private Space from their menu."
                    .into(),
            );
            cx.notify();
            return;
        }
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.install_extension(path, window, cx)
                });
            }
        })
        .detach();
    }

    /// Copies the extension in, reads it as WebKit will, and asks before
    /// adding it with what it requests. Cancel removes the copy.
    pub(crate) fn install_extension(
        &mut self,
        source: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let space = self.current_space().id;
        let space_name = self.current_space().name.clone();
        let root = self.extensions.root().to_path_buf();
        let background = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let staged = background.spawn(async move { stage(&source, &root) }).await;
            let staged = match staged {
                Ok(staged) => staged,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.notice = Some(format!("The extension couldn’t be added. {error}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let Ok(manifest) = cx.update(|_, _| engine::inspect_extension(&staged.dir)) else {
                return;
            };
            let manifest = manifest.recv().await.unwrap_or_else(|_| Err(String::new()));
            let _ = this.update_in(cx, |this, window, cx| {
                let manifest = match manifest {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        this.extensions.delete_files(&staged.id);
                        this.notice = Some(format!("The extension couldn’t be added. {error}"));
                        cx.notify();
                        return;
                    }
                };
                for problem in &manifest.problems {
                    log::warn!("Extension {}: {problem}", manifest.name);
                }
                this.confirm_install(space, space_name, staged, manifest, window, cx);
            });
        })
        .detach();
    }

    fn confirm_install(
        &mut self,
        space: [u8; 16],
        space_name: String,
        staged: Staged,
        manifest: engine::ExtensionManifest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.discard_pending {
            self.extensions.delete_files(&staged.id);
            return;
        }
        let name = if manifest.name.is_empty() {
            "this extension".to_string()
        } else {
            format!("“{}”", manifest.name)
        };
        let abilities = describe(&manifest.permissions, &manifest.patterns);
        let detail = if abilities.is_empty() {
            format!("It doesn’t ask to read or change your pages. It runs only in “{space_name}”.")
        } else {
            format!(
                "It will be able to:\n{}\n\nIt runs only in “{space_name}”.",
                bullets(&abilities)
            )
        };
        self.discard_pending = true;
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Add {name} to “{space_name}”?"),
            Some(&detail),
            &["Cancel", "Add Extension"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                this.discard_pending = false;
                if !confirmed {
                    this.extensions.delete_files(&staged.id);
                    return;
                }
                let installed = Installed {
                    id: staged.id.clone(),
                    name: manifest.name,
                    version: manifest.version,
                    folder: staged.folder,
                    enabled: true,
                    private: false,
                    permissions: manifest.permissions,
                    patterns: manifest.patterns,
                };
                if let Err(error) = this.extensions.add(&space, installed) {
                    this.notice = Some(error.to_string());
                }
                this.apply_extensions(space, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The strip's extensions button: each extension's menu, then Add.
    pub(crate) fn extensions_menu(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        use gpui::component::native_menu::NativeMenu;
        let current = self.current_space();
        let mut menu = NativeMenu::new();
        if current.private {
            // The Private Space borrows its extensions from the Spaces.
            for spec in self.extensions.private_specs(&self.saved_spaces()) {
                if let Some(owner) = self.extensions.owner(&self.saved_spaces(), &spec.install)
                    && let Some(installed) = self.extensions.get(&owner, &spec.install)
                {
                    menu = menu.submenu(
                        installed.name.clone(),
                        self.extension_items(owner, installed),
                    );
                }
            }
            menu.show(position, window, cx);
            return;
        }
        for installed in self.extensions.space(&current.id) {
            menu = menu.submenu(
                installed.name.clone(),
                self.extension_items(current.id, installed),
            );
        }
        menu.separator()
            .menu("Add Extension…", Box::new(crate::AddExtension))
            .show(position, window, cx);
    }

    /// A right-click on an extension's toolbar button.
    pub(crate) fn extension_menu(
        &self,
        install: &str,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(owner) = self.extensions.owner(&self.saved_spaces(), install) else {
            return;
        };
        let Some(installed) = self.extensions.get(&owner, install) else {
            return;
        };
        self.extension_items(owner, installed)
            .show(position, window, cx);
    }

    fn extension_items(
        &self,
        space: [u8; 16],
        installed: &Installed,
    ) -> gpui::component::native_menu::NativeMenu {
        use gpui::component::native_menu::NativeMenu;
        let item = |command| {
            Box::new(ExtensionMenu {
                space,
                install: installed.id.clone(),
                command,
            }) as Box<dyn Action>
        };
        NativeMenu::new()
            .menu_with_check(
                "Turned On",
                installed.enabled,
                item(ExtensionCommand::Toggle),
            )
            .menu_with_check(
                "Allow in Private Space",
                installed.private,
                item(ExtensionCommand::TogglePrivate),
            )
            .separator()
            .menu("Permissions…", item(ExtensionCommand::Permissions))
            .separator()
            .menu("Remove…", item(ExtensionCommand::Remove))
    }

    pub(crate) fn extension_command(
        &mut self,
        action: &ExtensionMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let space = action.space;
        let Some(installed) = self.extensions.get(&space, &action.install).cloned() else {
            return;
        };
        match action.command {
            ExtensionCommand::Toggle => {
                if let Err(error) = self.extensions.update(&space, &installed.id, |installed| {
                    installed.enabled = !installed.enabled
                }) {
                    self.notice = Some(error.to_string());
                }
                self.apply_extensions(space, cx);
            }
            ExtensionCommand::TogglePrivate => {
                if let Err(error) = self.extensions.update(&space, &installed.id, |installed| {
                    installed.private = !installed.private
                }) {
                    self.notice = Some(error.to_string());
                }
                self.apply_private_extensions(cx);
            }
            ExtensionCommand::Permissions => {
                let abilities = describe(&installed.permissions, &installed.patterns);
                let detail = if abilities.is_empty() {
                    "It can’t read or change your pages.".to_string()
                } else {
                    format!("It can:\n{}", bullets(&abilities))
                };
                let detail = format!(
                    "{detail}\n\nVersion {}. Remove it to take its access back.",
                    installed.version
                );
                if self.discard_pending {
                    return;
                }
                self.discard_pending = true;
                let answer = window.prompt(
                    PromptLevel::Info,
                    &format!("“{}”", installed.name),
                    Some(&detail),
                    &["OK"],
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    let _ = answer.await;
                    let _ = this.update(cx, |this, _| this.discard_pending = false);
                })
                .detach();
            }
            ExtensionCommand::Remove => self.confirm_remove(space, installed, window, cx),
        }
        cx.notify();
    }

    fn confirm_remove(
        &mut self,
        space: [u8; 16],
        installed: Installed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.discard_pending {
            return;
        }
        let space_name = self
            .spaces
            .iter()
            .find(|candidate| candidate.id == space)
            .map(|space| space.name.clone())
            .unwrap_or_default();
        self.discard_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Remove “{}”?", installed.name),
            Some(&format!(
                "Its settings and anything it stored in “{space_name}” are erased."
            )),
            &["Cancel", "Remove"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                this.discard_pending = false;
                if !confirmed {
                    return;
                }
                engine::remove_extension(space, &installed.id);
                if let Some(private) = this.spaces.iter().find(|space| space.private) {
                    engine::remove_extension(private.id, &installed.id);
                }
                if let Err(error) = this.extensions.remove(&space, &installed.id) {
                    this.notice = Some(error.to_string());
                }
                this.apply_extensions(space, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn extension_event(
        &mut self,
        event: engine::ExtensionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use engine::ExtensionEvent::*;
        match event {
            Loaded {
                space,
                install,
                error: Some(error),
            } => {
                let name = self
                    .extensions
                    .get(&space, &install)
                    .map(|installed| installed.name.clone())
                    .unwrap_or_else(|| "An extension".into());
                self.notice = Some(format!("“{name}” couldn’t start. {error}"));
            }
            Loaded { .. } | ActionsChanged => self.refresh_extension_actions(),
            OpenTab {
                space,
                url,
                active,
                token,
            } => {
                let id = self.open_extension_tab(space, url, active, window, cx);
                engine::extension_tab_opened(token, id);
            }
            Navigate { tab, url } => self.extension_navigate(tab, url, window, cx),
            Activate { tab } => {
                if let Some(space) = self.space_of(tab) {
                    self.switch_space(space, window, cx);
                }
                if let Some(index) = self.tabs.iter().position(|candidate| candidate.id == tab) {
                    self.activate(index, window, cx);
                }
            }
            Close { tab } => {
                if let Some(index) = self.tabs.iter().position(|candidate| candidate.id == tab) {
                    self.close(index, window, cx);
                }
            }
            FocusSpace { space } => {
                if let Some(index) = self
                    .spaces
                    .iter()
                    .position(|candidate| candidate.id == space)
                {
                    self.switch_space(index, window, cx);
                }
                window.activate_window();
            }
            Prompt {
                space,
                install,
                name,
                permissions,
                patterns,
                reply,
            } => self.extension_prompt(
                space,
                install,
                name,
                permissions,
                patterns,
                reply,
                window,
                cx,
            ),
            Granted {
                install,
                permissions,
                patterns,
            } => {
                // A private context's grants belong to the Space it came from.
                if let Some(owner) = self.extensions.owner(&self.saved_spaces(), &install)
                    && let Err(error) = self.extensions.update(&owner, &install, |installed| {
                        installed.permissions = permissions;
                        installed.patterns = patterns;
                    })
                {
                    self.notice = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    /// `tabs.create`: opens in the extension's Space, on top; in front only
    /// when asked and the Space is on screen.
    fn open_extension_tab(
        &mut self,
        space: [u8; 16],
        url: Option<String>,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let url = url.unwrap_or_else(|| crate::model::START.into());
        if self.all_tabs().count() >= 1000 {
            self.notice = Some("Tab limit reached.".into());
            return None;
        }
        let private = self
            .spaces
            .iter()
            .find(|candidate| candidate.id == space)?
            .private;
        if space == self.current_space().id {
            if active {
                let before = self.ids.get();
                self.new_tab(url, private, window, cx);
                return (self.ids.get() > before).then(|| self.tabs[self.active].id);
            }
            let id = engine::next_id(&self.ids);
            self.tabs.insert(0, crate::Tab::new(id, url, private));
            self.active += 1;
            self.dirty = true;
            return Some(id);
        }
        let id = engine::next_id(&self.ids);
        let parked = self.parked.entry(space).or_default();
        if !parked.tabs.is_empty() {
            parked.active += 1;
        }
        parked.tabs.insert(0, crate::Tab::new(id, url, private));
        self.dirty = true;
        Some(id)
    }

    fn extension_navigate(
        &mut self,
        id: u64,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::model::allowed_navigation(&url) {
            return;
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id)
            && index == self.active
        {
            self.navigate_unchecked(url, window, cx);
            return;
        }
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        match &tab.page {
            Some(page) => {
                if let Err(error) = page.load_url(&url) {
                    log::warn!("An extension's navigation: {error}");
                }
            }
            None => {
                tab.saved.title = crate::model::display_host(&url);
                tab.saved.url = url;
            }
        }
        self.dirty = true;
    }

    /// An extension asks for more access. Requests that read or change
    /// nothing of the user's are granted without asking, as in Chrome;
    /// otherwise only the Space on screen asks, never over another dialog,
    /// with "Don't Allow" as the default.
    #[allow(clippy::too_many_arguments)]
    fn extension_prompt(
        &mut self,
        space: [u8; 16],
        _install: String,
        name: String,
        permissions: Vec<String>,
        patterns: Vec<String>,
        reply: async_channel::Sender<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let abilities = describe(&permissions, &patterns);
        if abilities.is_empty() {
            let _ = reply.try_send(true);
            return;
        }
        if self.discard_pending || space != self.current_space().id {
            return;
        }
        let title = match abilities.as_slice() {
            [one] => format!("Allow “{name}” to {}?", lowercase_first(one)),
            _ => format!("Allow “{name}” more access?"),
        };
        let detail = match abilities.as_slice() {
            [_] => None,
            _ => Some(format!("It asks to:\n{}", bullets(&abilities))),
        };
        self.discard_pending = true;
        let answer = window.prompt(
            PromptLevel::Info,
            &title,
            detail.as_deref(),
            &[
                gpui::PromptButton::cancel("Don’t Allow"),
                gpui::PromptButton::new("Allow"),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let allow = answer.await == Ok(1);
            let _ = this.update(cx, |this, _| this.discard_pending = false);
            let _ = reply.try_send(allow);
        })
        .detach();
    }

    /// A Space was deleted: its extensions go, with their files and
    /// everything they stored.
    pub(crate) fn forget_space_extensions(&mut self, space: [u8; 16], cx: &App) {
        engine::forget_extensions(space, true);
        if let Err(error) = self.extensions.forget_space(&space) {
            self.notice = Some(error.to_string());
        }
        self.apply_private_extensions(cx);
    }
}

/// A button's frame, recorded as it is drawn, for its popup's anchor.
pub(crate) type Anchors =
    std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, Bounds<Pixels>>>>;

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: [u8; 16] = [7; 16];
    const OTHER: [u8; 16] = [9; 16];

    fn installed(id: &str, name: &str) -> Installed {
        Installed {
            id: id.into(),
            name: name.into(),
            version: "1.0".into(),
            folder: id.into(),
            enabled: true,
            private: false,
            permissions: vec!["storage".into()],
            patterns: vec!["<all_urls>".into()],
        }
    }

    #[test]
    fn host_access_is_dropped_but_blocking_stays() {
        let mut manifest = serde_json::json!({
            "permissions": ["activeTab", "declarativeNetRequest", HOST_ACCESS, "storage"],
            "optional_permissions": [HOST_ACCESS],
        });
        assert!(adjust_manifest(&mut manifest));
        assert_eq!(
            manifest["permissions"],
            serde_json::json!(["activeTab", "declarativeNetRequest", "storage"])
        );
        assert_eq!(
            manifest["optional_permissions"],
            serde_json::json!(["declarativeNetRequest"])
        );
        let mut alone = serde_json::json!({ "permissions": [HOST_ACCESS] });
        assert!(adjust_manifest(&mut alone));
        assert_eq!(
            alone["permissions"],
            serde_json::json!(["declarativeNetRequest"])
        );
        let mut plain = serde_json::json!({ "permissions": ["storage"] });
        assert!(!adjust_manifest(&mut plain));
        assert!(!adjust_manifest(&mut serde_json::json!({ "name": "x" })));
    }

    #[test]
    fn descriptions_say_what_access_means() {
        let all = describe(
            &[
                "storage".into(),
                "declarativeNetRequest".into(),
                "activeTab".into(),
            ],
            &["<all_urls>".into()],
        );
        assert_eq!(
            all,
            [
                "Read and change everything on every website you visit",
                "Block content on any page"
            ]
        );
        assert_eq!(
            describe(&[], &["*://*/*".into()]),
            ["Read and change everything on every website you visit"]
        );
        assert_eq!(
            describe(
                &["tabs".into(), "webNavigation".into()],
                &["https://*.example.com/*".into(), "https://b.org/x/*".into()]
            ),
            [
                "Read and change your data on b.org and example.com",
                "See the address of every page you open"
            ]
        );
        assert_eq!(
            sites(&["a.com", "b.com", "c.com", "d.com", "e.com"]),
            "a.com, b.com and 3 more sites"
        );
        // Grants that touch nothing of the user's say nothing.
        assert!(describe(&["storage".into(), "alarms".into()], &[]).is_empty());
        assert!(describe(&[], &["webkit-extension://abc/*".into()]).is_empty());
        assert_eq!(
            lowercase_first("Read and change your data on a.com"),
            "read and change your data on a.com"
        );
    }

    #[test]
    fn installs_persist_per_space() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(store.root().join(&id)).unwrap();
        store.add(&SPACE, installed(&id, "Blocker")).unwrap();
        store
            .update(&SPACE, &id, |installed| installed.enabled = false)
            .unwrap();
        let mut store = Store::open(dir.path());
        assert_eq!(store.space(&SPACE).len(), 1);
        assert!(!store.space(&SPACE)[0].enabled);
        assert!(store.space(&OTHER).is_empty());
        assert!(store.specs(&SPACE).is_empty());
        store
            .update(&SPACE, &id, |installed| installed.enabled = true)
            .unwrap();
        let specs = store.specs(&SPACE);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].dir, dir.path().join("extensions").join(&id));
        assert_eq!(specs[0].patterns, ["<all_urls>"]);
        assert_eq!(store.owner(&[OTHER, SPACE], &id), Some(SPACE));
        store.remove(&SPACE, &id).unwrap();
        assert!(Store::open(dir.path()).space(&SPACE).is_empty());
        assert!(!dir.path().join("extensions").join(&id).exists());
    }

    #[test]
    fn private_space_runs_only_allowed_extensions_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        let (a, b, c) = (
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        );
        store
            .add(
                &SPACE,
                Installed {
                    private: true,
                    ..installed(&a, "Blocker")
                },
            )
            .unwrap();
        store.add(&SPACE, installed(&b, "Notes")).unwrap();
        // The same extension allowed from a second Space runs once.
        store
            .add(
                &OTHER,
                Installed {
                    private: true,
                    ..installed(&c, "Blocker")
                },
            )
            .unwrap();
        let specs = store.private_specs(&[SPACE, OTHER]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].install, a);
        store
            .update(&SPACE, &a, |installed| installed.enabled = false)
            .unwrap();
        let specs = store.private_specs(&[SPACE, OTHER]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].install, c);
        store.forget_space(&OTHER).unwrap();
        assert!(store.private_specs(&[SPACE]).is_empty());
    }

    #[test]
    fn staging_copies_folders_and_unpacks_archives() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("probe");
        std::fs::create_dir_all(source.join("js")).unwrap();
        std::fs::write(
            source.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "manifest_version": 3,
                "name": "Probe",
                "version": "1",
                "permissions": ["declarativeNetRequestWithHostAccess"],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(source.join("js/background.js"), "").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", source.join("hosts")).unwrap();
        let root = dir.path().join("extensions");
        let staged = stage(&source, &root).unwrap();
        assert_eq!(staged.folder, staged.id);
        assert!(staged.dir.join("js/background.js").is_file());
        // Links are never copied.
        assert!(!staged.dir.join("hosts").exists());
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(staged.dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["permissions"],
            serde_json::json!(["declarativeNetRequest"])
        );

        // A zip of the folder unpacks into its one wrapping folder.
        let archive = dir.path().join("probe.zip");
        let zipped = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&source)
            .arg(&archive)
            .status();
        if zipped.is_ok_and(|status| status.success()) {
            let staged = stage(&archive, &root).unwrap();
            assert_eq!(staged.folder, format!("{}/probe", staged.id));
            assert!(staged.dir.join("manifest.json").is_file());
        }
        // Not an extension: nothing is left behind.
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let before = std::fs::read_dir(&root).unwrap().count();
        assert!(stage(&empty, &root).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
    }

    #[test]
    fn archive_entries_stay_inside() {
        assert!(safe_entry("probe/manifest.json"));
        assert!(safe_entry("manifest.json"));
        assert!(!safe_entry("../evil"));
        assert!(!safe_entry("probe/../../evil"));
        assert!(!safe_entry("/etc/passwd"));
        assert!(!safe_entry("a\\..\\b"));
    }
}
