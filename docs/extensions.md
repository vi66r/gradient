# Web extensions

Browser runs Safari-style web extensions through WebKit's public
`WKWebExtension` API (macOS 15.4+). This builds on the spike
(docs/extensions-spike.md). Code: `src/engine/extensions.rs` (WebKit side) and
`src/extensions.rs` (installs, menus, prompts).

## One controller per Space

- Each saved Space has its own `WKWebExtensionController`, made with
  `configurationWithIdentifier:` and the Space's ID. Extension storage and the
  compiled content-blocking rules survive relaunch.
- The controller's `defaultWebsiteDataStore` is the Space's own store, so an
  extension's background requests carry that Space's cookies and no other
  Space's.
- Every page in a Space joins its controller, including Spaces with no
  extensions. A WKWebView's configuration can't change after creation, so an
  extension added later couldn't reach pages that were already open.
- Extension pages and workers send Safari's user agent, as Browser's pages
  do. Extensions choose code paths by user agent; Bitwarden's background
  crashed without it.

## The Private Space

Extensions run in the Private Space only if the user allows them there. Each
extension's menu has "Allow in Private Space", off by default, as in Safari
and Chrome. An extension sees every page you open, so it takes an explicit
choice to bring one into private browsing.

- The Private Space gets a non-persistent controller and its throwaway data
  store. Anything an extension stores there is gone when the Space closes.
- It runs every enabled extension allowed there from any saved Space, once
  per name (the first Space in switcher order wins). Its contexts have
  `hasAccessToPrivateData` set.
- Extensions are added in a saved Space. The Private Space's menu manages
  the ones running there through the Space they came from.
- Cost: a content blocker recompiles its rules each time the Private Space
  opens, so the first pages there aren't filtered yet.

## Tabs and windows

Extensions see each Space as one window holding that Space's tabs, whether
the Space is on screen or parked. That window is always the focused one,
because it is the only window those extensions know.

- WebKit asks about tabs synchronously, sometimes in the middle of the calls
  Browser makes to it. So the model lives in the engine and never reaches into
  the shell. Every render, the shell describes each Space's tabs (ID, page,
  title, address, loading). The engine diffs that against its model and calls
  `didOpenTab`, `didCloseTab`, `didActivateTab` and `didChangeTabProperties`.
  An unchanged frame costs one comparison per tab.
- Tabs without a page (resting or suspended, the start page) are tabs too,
  with no web view.
- Requests from extensions reach the shell as `ExtensionEvent`s: open,
  activate, close and navigate tabs, focus a Space, permission prompts.
  `tabs.create` completes once the new tab has been announced to WebKit.
  `windows.create` is refused, because a window shows one Space at a time.
- Clicking a toolbar button grants `activeTab` for the tab in front.

## Extension pages in tabs

`webkit-extension://` pages (options pages, dashboards) open in tabs and are
saved with the session. WebKit serves them only to a web view built from the
owning context's `webViewConfiguration`. So a tab on an extension URL gets a
view built for that page, and a tab navigating onto one gets a new view. A
restored extension tab opens before its extension has started, so it waits
and loads once the extension reports in. Web pages still can't load extension
pages unless the manifest exposes them (WebKit enforces
`web_accessible_resources`).

## Installing

- Tools ▸ Add Extension… accepts an unpacked extension's folder or a `.zip`.
  Browser copies it into its data folder (`extensions/<install ID>/`), so the
  original can move or go.
  - Copying skips symbolic links.
  - Archives are checked for entries that would land outside the folder
    before `ditto` unpacks them, and any links in them are removed.
  - Both are capped at 20,000 files and 512 MB.
- Browser reads the extension the way WebKit will, then asks: "Add “uBlock
  Origin Lite” to “Personal”?" The sheet lists what it will be able to do, in
  plain words ("Read and change everything on every website you visit",
  "Block content on any page"), and Cancel is the default. Cancel deletes the
  copy.
- The list is saved per Space in `extensions.json`. Each entry records whether
  the extension is on, whether it is allowed in the Private Space, and what it
  was granted (WebKit's permission names and match patterns). Grants are
  applied before an extension starts.
- **uBlock Origin Lite workaround.** WebKit ignores an extension's blocking
  rules when it has `declarativeNetRequestWithHostAccess` together with a
  toolbar action. So at install, Browser removes that permission from any
  manifest that requests it. Plain `declarativeNetRequest` is added if it is
  missing, and the change is logged. Block rules keep working; redirect rules
  (uBOL's surrogate scripts) no longer apply. This should be filed with WebKit.
- Each extension's menu (the strip's extensions button, or a right-click on
  its toolbar button) has: Turned On, Allow in Private Space, Permissions…
  (what it can do) and Remove…. Remove asks first, erases what the extension
  stored in that Space (its WebKit data records), unloads it and deletes its
  files.
- Deleting a Space deletes its extensions and their data.
- Not yet: Safari extensions inside installed apps (`.appex`, e.g. 1Password
  for Safari), `.crx` files, or update checks.

## Toolbar and popups

- Extensions with an action get a button at the right end of the page strip:
  - their own icon, in full color (rendered to a 32 px PNG, cached per image);
  - a badge as a small pill in the strip's ink, so it reads on any page
    color;
  - a disabled action fades.
- The extensions button (a puzzle piece) appears only while the Space has
  extensions.
- A click runs the action. A popup opens in WebKit's own `popupPopover`,
  under the button, sized to the popup page; dismissing it closes the popup.
  A popup an extension opens by itself drops from the strip's right end.

## Permission prompts

Requests made after install (`permissions.request`, access to more sites) are
answered by the shell. Requests that touch nothing of the user's (storage,
alarms, `activeTab`) are granted without asking, as in Chrome. Anything else
gets one sheet in the shell's style, "Allow “Browser probe” to change what
you copy?", with Don't Allow as the default.

- Only the Space on screen asks, and never over another dialog. Anything
  else is denied quietly and the extension may ask again later.
- "Allow" is saved with the install. "Don't Allow" is not saved.

## Native messaging

`runtime.sendNativeMessage` and `runtime.connectNative` go to a
`NativeMessaging` hook (`engine::extensions::set_native_messaging`). Safari
extensions talk to their app through their `.appex`, and the 1Password relay
fills this in. Until a relay is set, extensions are told that no app answers.

## Known WebKit behavior

- WebKit stops the app on a failed assertion if it reads an extension from a
  relative file URL. Browser always passes absolute paths, which matters with
  a relative `BROWSER_DATA_DIR`. The spike harness needs absolute paths too.
- After an extension is added, a large rule set takes a few seconds to
  compile: about 8 s for uBOL's 170k rules on this Mac. Pages opened before
  then aren't filtered. With the persistent controller, the first page after
  relaunch is filtered.
- Generic cosmetic filtering (hiding ad slots) didn't apply in uBOL's default
  mode; network blocking does. Same as in the spike.
