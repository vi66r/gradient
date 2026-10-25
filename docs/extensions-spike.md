# Web extensions spike (2026-10-25)

Question: can Browser run real web extensions through WebKit's public
`WKWebExtension` API (macOS 15.4+), and does that solve content blocking and
password managers? Tested on macOS 26.6.2 with a standalone Objective-C harness
(`scripts/extension-harness.m`) and in Browser itself behind
`BROWSER_EXTENSIONS` (`src/engine/extensions.rs`, this branch only).

## Verdict

- **Content blocking: yes.** uBlock Origin Lite (MV3) blocks ads and trackers
  in Browser's own pages, after one manifest workaround (below). Recommended
  over hand-rolling `WKContentRuleList` blocking.
- **Passwords: not yet, and the path is clear.** 1Password's Safari extension
  loads and runs, but it is inert until Browser relays its native messages to
  the extension's app-side handler, as Safari does. That relay is the main
  piece of work for passwords. Bitwarden's Chrome build gets further after a
  user-agent fix but its popup never finishes loading.
- The API is broad enough for a real extension system. Browser has to supply
  a tab and window model, popups, permission prompts and an install UI.

## What works

| Check | Result |
|---|---|
| Load an unpacked MV3 extension, grant permissions, attach the controller to a page configuration | Works (harness and Browser) |
| Content script on page load | Runs |
| `declarativeNetRequest` static rules | Block as declared |
| Content script → background `runtime.sendMessage` | Fails with "Tab not found" until the app describes tabs and windows (`WKWebExtensionTab`/`Window`, `didOpenTab`, `didOpenWindow`, `didFocusWindow`, controller delegate `openWindows`/`focusedWindow`). With a minimal model the reply arrives with `sender.tab.id`. |
| Toolbar action popup | `performActionForTab` → delegate `presentPopupForAction` with a ready `popupWebView`, shown in an `NSPopover` |
| Native messaging | Delegate receives `sendMessage:toApplicationWithIdentifier:` and `connectUsingMessagePort:` |
| Safari web extension from an installed app (`extensionWithAppExtensionBundle:`) | Loads (1Password for Safari 8.11.22) |
| Extension on a page using a Space's own `dataStoreForIdentifier` store | Works |

## uBlock Origin Lite 2026.1021.1354 (Safari build)

- As shipped it **blocks nothing** in a WKWebView host, although WebKit reports
  its rules loaded and full host access granted. Bisected: the combination of
  the `declarativeNetRequestWithHostAccess` permission and a toolbar `action`
  disables its rules. EasyList/EasyPrivacy/uBO filters alone (112k+ rules,
  static and regex, plus uBOL's 110 dynamic rules and its Safari ruleset
  toggle) all block fine. Granting again after load, or a tab returning
  `shouldBypassPermissions`, doesn't help. Looks like a WebKit bug worth
  filing.
- **Workaround:** drop `declarativeNetRequestWithHostAccess` from the manifest
  at install. Block rules don't need host access; only redirect rules do, so
  uBOL's surrogate-script redirects are lost. With that, the full extension
  (background included) blocked AdSense and Google Analytics in the harness
  and in Browser.
- The first launch compiles ~170k rules in the background; a page opened in
  the first ~15 s isn't filtered yet. A persistent controller
  (`WKWebExtensionControllerConfiguration configurationWithIdentifier:`)
  should keep the compiled lists across launches.
- Generic cosmetic filtering (hiding `.adsbygoogle` slots) didn't apply in its
  default mode; network blocking did.

## 1Password for Safari 8.11.22 (MV2, from the installed app)

- Loads, background runs, content is injected into pages. Its action has no
  web popup; the UI comes through its native side.
- On start it immediately messages its app: `request-os-version`, a `core`
  message asking for a device UUID, and ports to `com.1password.1password`.
  In Safari these go to the extension's own `SafariWebExtensionHandler` in the
  `.appex`, which relays to the 1Password app. A host that is not Safari has
  to do that relay: start the app extension and pass each message as an
  `NSExtensionItem` (`SFExtensionMessageKey`). The host side of that is
  `NSExtension`, private Foundation API (the project already accepts private
  SPI where it's the only way). Not yet tried.
- Nothing was signed in and the harness refused every native message, so no
  vault data was involved.

## Bitwarden 2026.10.0 (Chrome build)

- Background crashed at startup: Bitwarden picks its code paths by
  `navigator.userAgent`, and extension pages get WebKit's default agent
  without ` Safari/`. Fixed by giving extension pages Safari's agent through
  `WKWebExtensionControllerConfiguration.webViewConfiguration` (Browser's
  pages already send it).
- After that the background runs, but the popup page never finishes loading.
  Not chased further. Bitwarden's Safari build ships inside its Mac App Store
  app and would load the same way as 1Password's.

## What a real implementation needs

1. One persistent controller per Space (`configurationWithIdentifier:` with
   the Space ID; the Private Space non-persistent), extension pages sending
   Safari's user agent.
2. A tab and window model over `Browser::tabs` / `spaces` (needed for any
   messaging), with `didOpenTab`/`didActivateTab`/`didCloseTab` calls from the
   shell.
3. Toolbar actions in the control strip, with popups in a popover.
4. Permission prompts (`promptForPermissions…`) in the shell's calm style.
5. Install/enable/remove UI: unpacked folder or zip, plus Safari extensions
   found in installed apps (`.appex` with `com.apple.Safari.web-extension`).
6. The Safari native-messaging relay, for 1Password and other app-backed
   extensions.
7. The uBOL manifest workaround, and a WebKit bug report.

## Reproducing

```
clang -fobjc-arc -framework AppKit -framework WebKit scripts/extension-harness.m -o /tmp/extension-harness
python3 -m http.server 8773 --bind 127.0.0.1 --directory fixtures/extensions/site
/tmp/extension-harness http://127.0.0.1:8773/ 5 fixtures/extensions/probe
HARNESS_WARMUP=12 /tmp/extension-harness http://127.0.0.1:8773/ads.html 4 path/to/uBOL
```

In Browser: `BROWSER_EXTENSIONS=/path/one:/path/two RUST_LOG=warn` (unpacked
folders; grants everything requested).
