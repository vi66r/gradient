# Browser

A native browser prototype: Rust + GPUI for the interface, system WebKit for web content. macOS is running and tested; Linux's WebKitGTK/X11 adapter is implemented but has not been built or run on Linux yet.

## Run on macOS

Install Rust and Xcode Command Line Tools, then:

```sh
scripts/bundle-macos.sh release
open dist/Browser.app
```

The repository pins Rust 1.96 and dependency versions through `Cargo.lock`. Use `scripts/bundle-macos.sh` for a faster debug build. The bundle is locally ad-hoc signed, not notarized or installed as your default browser.

For an isolated development profile:

```sh
BROWSER_DATA_DIR=/tmp/browser-dev dist/Browser.app/Contents/MacOS/browser https://webkit.org
```

Normal app state lives in `~/Library/Application Support/RafiBrowser`. A profile UUID selects a separate persistent WKWebView website data store, managed by WebKit in its own system location. Moving only `session.json` does not move cookies or website storage. Only one app instance may open a profile.

## What works

- Native vertical tabs, URL/search field, back/forward/reload, new-window links, and URL copying. Copy buttons briefly show a checkmark without moving the page.
- Normal tab/session recovery with atomic saves. Restored background tabs stay unloaded until selected. Live tabs retain their page state when switched away from.
- Site icons in the sidebar. After a normal tab loads, the shell reads the page's declared icon, fetches it without cookies (256 KiB cap), and caches it per host in the profile's `icons/` folder for 7 days, keeping at most 500. Private tabs never fetch icons.
- Private tabs use ephemeral engine storage, are omitted from saved sessions and recently closed tabs, and cannot be shared with agents.
- Manual suspension under **Tools → Suspend Current Tab**, with an explicit warning about losing live page state. Resuming reloads the URL.
- System WebKit's GPU rendering; the local WebGPU cube renders successfully on the tested Mac. No application-side screenshot or texture-copy pipeline.
- Native **Request lab**: HTTP methods, custom headers and body, status/timing/size, formatted JSON responses, and cURL export. Requests run off the UI thread, have a 30-second timeout, and cap displayed responses at 2 MiB. Redirects are returned for inspection rather than automatically followed.
- WebKit Inspector and a small opt-in local MCP server.

On Mac, use ⌘T, ⌘W, ⌘⇧T, ⌘L, ⌘R, ⌘[ / ⌘], Control-Tab, **⌘⇧C** (copy URL), ⌘⇧N (private tab), ⌘⇧J (request lab), and ⌘⌥I (Inspector). Linux uses Control in place of Command.

Request lab data stays in memory and is separate from page cookies and sessions. cURL exports can contain credentials you entered; copying is explicit. Collections, environments, cURL import, and request cancellation are not implemented yet. Proxyman-style interception remains a later milestone.

## Local MCP

Keep Browser running. Configure your MCP client to start this stdio command (replace the absolute path for your checkout):

```json
{
  "mcpServers": {
    "browser": {
      "command": "/Users/rafi/Developer/browser/dist/Browser.app/Contents/MacOS/browser",
      "args": ["mcp"]
    }
  }
}
```

For a development profile, set the client's `BROWSER_DATA_DIR` environment variable to the same value as the app. The bridge does not start the GUI or acquire its profile lock.

Click **Share tab with agent** inside Browser and confirm the specific origin. Initially, clients see no tabs. Tools are `browser_tabs`, `browser_read_page`, `browser_navigate`, `browser_click`, and `browser_fill`. Selectors must match exactly one visible, enabled element. Password and file inputs are excluded from fill; arbitrary JavaScript evaluation and shell execution are not exposed. Page text is untrusted website data.

Grants are in memory, scoped to a tab and exact origin (scheme/host/port), and revoked on explicit revocation, cross-origin navigation, suspension, closing, or app exit. Agent navigation is restricted to the granted origin. Action results are checked again before returning. A page click can still cause a consequential website action, so the agent must obtain the user's authorization for that action.

The transport is a Unix socket with mode `0600` inside a `0700` profile directory, with bounded messages, concurrency, and request deadlines. There is no TCP listener or website-to-native command bridge. This restricts access to your OS account, **not to one particular agent client**: any process running as you can connect and access currently shared tabs. A local transport does not imply the model runs locally.

## Development checks

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 scripts/test-server.py --port 8765
```

To check the unavailable-page crash regression against a built app:

```sh
python3 scripts/check-offline-start.py
```

The demo tabs require this local server to stay running. If it stops, a newly opened demo shows an error with **Try again**; restart the server and retry.

Open `http://127.0.0.1:8765/` for text input, tab preservation, links, scroll, and website storage checks. `/gpu` runs a WebGPU 3D cube and reports API availability; its displayed frame rate is a diagnostic, not a controlled benchmark. `/api/echo` is the request-lab target.

```sh
python3 scripts/check-mcp.py --profile /tmp/browser-dev
# After sharing only the localhost interaction-test tab:
python3 scripts/check-mcp.py --profile /tmp/browser-dev --shared-fixture
```

The shared-fixture check changes only the disposable fixture's counter and text input. The default check verifies no tabs are exposed. See [VALIDATION.md](VALIDATION.md) for what was actually exercised.

## Linux path

The source selects GPUI's X11 backend and embeds WebKitGTK via Wry. XWayland is required in a Wayland session. It needs GTK 3, WebKitGTK 4.1, and the native build dependencies required by GPUI, including X11/XKB, graphics, and font libraries. Build with `cargo build --locked` on Linux. Native Wayland embedding, distro packaging, IME, accessibility, and GPU compatibility remain unverified. WebGPU parity with macOS is not assumed.

## Current boundaries

This is a working first prototype, not a hardened daily-driver browser. Safari-level privacy and aggressive performance are acceptance targets, not achieved claims. It has no custom tracking/content blocker, privacy settings, full browser sandbox distribution, password manager, extensions, download manager, or camera/microphone permission UI. Site permission requests and downloads are denied for now. Network failures still need a complete error-page flow.

Automatic tab eviction is intentionally not enabled until we can reliably protect unsaved forms, media, calls, and downloads. Explicit tab close, app exit, and restoration do not preserve live forms or run a complete browser-level before-unload confirmation flow. The repeatable macOS regression suite measures optimized builds including attributed WebKit/Metal helpers; see [PERFORMANCE.md](PERFORMANCE.md). Safari comparisons, battery/energy measurements, and complex real-site workloads remain unverified.
