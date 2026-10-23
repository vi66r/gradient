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
- Local files: HTML, PDFs, images and text open from the address field (a `file://` URL or an absolute path such as `/Users/…` or `~/…`), from **File → Open File…** (⌘O), or by dropping files from Finder onto the window. Drops on the sidebar or a start/resting page open new tabs. Drops on a page go to the page first, so upload drop zones keep working; if the page doesn't take the drop, the file opens in a new tab instead of replacing it. On macOS a file loads with read access to its own folder only, so a local page's relative images and stylesheets work and it can link to sibling files. The resting address field shows the file name with a document icon; focus shows the full `file://` URL, which Copy Address copies. Local files never fetch icons and can't be shared with agents. Normal file tabs restore after a restart; a file that has since moved shows the error screen with **Try again**. Web pages can't navigate to or open local files: WebKit refuses it, and Browser's own navigation policy refuses it too.
- Normal tab/session recovery with atomic saves. Restored background tabs stay unloaded until selected. Live tabs retain their page state when switched away from.
- Site icons in the sidebar. After a normal tab loads, the shell reads the page's declared icon, fetches it without cookies (256 KiB cap), and caches it per host in the profile's `icons/` folder for 7 days, keeping at most 500. Private tabs never fetch icons.
- Media controls in the sidebar (macOS). When a tab plays audio, its cell expands with play/pause and a page mute that also covers Web Audio and cross-origin frames, including for background tabs. Pages that publish a Media Session (Spotify, YouTube, SoundCloud) also get previous/next track buttons, which run the page's own handlers. A cell you paused stays so you can resume; otherwise it collapses about 2.5 s after playback stops. Playback state and mute use WebKit private SPI (`_isPlayingAudio`, `_setPageMuted:`), as Safari does. Track skips use `navigator.mediaSession` from an isolated script world, which pages can't observe. Each selector is checked first, so the feature switches off if WebKit removes one.
- Private tabs use ephemeral engine storage, are omitted from saved sessions and recently closed tabs, and cannot be shared with agents.
- On macOS, closing, suspending, returning to the start screen, quitting, and browser-initiated navigation (Back/Forward including swipes, Reload, addresses loaded from the address field or an agent) check for edits the user made. Only work the user produced counts: typing or changes in form fields (search fields excepted), rich-text edits, and clicks or typing inside content Browser can't inspect. Settled pages proceed quietly. A page's own links, redirects and frame loads are never delayed or guarded. Cancel preserves the page. Quit also checks Request Lab drafts; session-write failures require a separate explicit choice. See the work-protection limits below.
- Manual suspension under **Tools → Suspend Current Tab**. Resuming reloads the URL.
- System WebKit's GPU rendering; the local WebGPU cube renders successfully on the tested Mac. No application-side screenshot or texture-copy pipeline.
- Native **Request lab**: HTTP methods, custom headers and body, status/timing/size, formatted JSON responses, and cURL export. Requests run off the UI thread, have a 30-second timeout, and cap displayed responses at 2 MiB. Redirects are returned for inspection rather than automatically followed.
- WebKit Inspector and a small opt-in local MCP server.

On Mac, use ⌘T, ⌘O (open file), ⌘W, ⌘⇧T, ⌘L, ⌘R, ⌘[ / ⌘], Control-Tab, **⌘⇧C** (copy URL), ⌘⇧N (private tab), ⌘⇧J (request lab), and ⌘⌥I (Inspector). Linux uses Control in place of Command.

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

`browser_navigate` acknowledges `navigation_requested`, not a completed load: a local confirmation may still be required. Mutation tools return an error while a page check or confirmation is pending; callers can retry after the user decides. Read access remains subject to the existing grant checks.

The transport is a Unix socket with mode `0600` inside a `0700` profile directory, with bounded messages, concurrency, and request deadlines. There is no TCP listener or website-to-native command bridge. This restricts access to your OS account, **not to one particular agent client**: any process running as you can connect and access currently shared tabs. A local transport does not imply the model runs locally.

## Development checks

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
# macOS: real WebKit checks in an ephemeral data store
swift scripts/check-work-protection.swift
swift scripts/check-file-urls.swift
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

Local files have limits. A local page's links to files outside its folder don't load (WebKit grants only that folder), and its `target="_blank"` links to files don't open. Folders can't be shown. On Linux, Browser-started file loads are admitted but file-to-file links and page drops are not, because Wry's Linux handler doesn't say which frame started a navigation.

Automatic tab eviction remains disabled. Work protection is a **heuristic, not the site's `beforeunload` contract**. On macOS an isolated [WKContentWorld](https://developer.apple.com/documentation/webkit/wkcontentworld) checks controls the user changed (trusted input/change events) against their DOM defaults, observed rich-text edits, open shadow roots, and same-origin frames. Values set by the page itself, playing media, and untouched canvas/embed content or cross-origin frames don't count. Resetting a form or updating its defaults after saving can clear its warning. Opaque content the user worked in, unavailable scripts, probe errors/timeouts and exceeded scan limits warn conservatively. Probes run on demand, without background timers or a website-to-native message handler. Temporary field/editor comparisons remain inside the isolated JavaScript world; Rust receives only risk and revision metadata, and no draft values are persisted.

Native navigation holds WebKit's original decision and request through the confirmation, preserving POST bodies, history, reloads, and redirects. Approval is checked against the page again before proceeding; new observed edits invalidate it. It cannot reliably infer arbitrary SPA save state, unobserved programmatic rich-text changes, JavaScript-only editors, closed shadow roots, Web Audio, calls, or all frame lifecycle changes. Crashes and force quit are not protected. Confirmed destruction still loses drafts; session recovery restores addresses only. No private WebKit APIs are used. Linux retains conservative shell discard warnings; this native navigation guard is macOS-only and Linux remains untested.

The repeatable macOS regression suite measures optimized builds including attributed WebKit/Metal helpers; see [PERFORMANCE.md](PERFORMANCE.md). Safari comparisons, battery/energy measurements, and complex real-site workloads remain unverified.
