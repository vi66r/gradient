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
- On macOS, closing, suspending, returning to the start screen, quitting, and browser-initiated navigation (Back/Forward including swipes, Reload, addresses loaded from the address field or an agent in a shared tab; agents on trusted sites skip it) check for edits the user made. Only work the user produced counts: typing or changes in form fields (search fields excepted), rich-text edits, and clicks or typing inside content Browser can't inspect. Settled pages proceed quietly. A page's own links, redirects and frame loads are never delayed or guarded. Cancel preserves the page. Quit also checks Request Lab drafts; session-write failures require a separate explicit choice. See the work-protection limits below.
- App links, as in Safari (macOS). When a page's link, form or redirect goes to a non-web scheme (`mailto:`, `tel:`, `sms:`, `facetime:`, `zoommtg:`, `slack:`, `spotify:`, `vscode:`…), WebKit cancels the navigation and Browser asks "Open “Mail”?", naming the site and the scheme. Cancel is the default; Open hands the link to the app macOS has registered. A link typed into the address field takes the same path.
  - Only the active tab's main frame can ask. Embedded frames, such as ads, are cancelled silently.
  - One prompt per tab at a time. After Cancel, that tab can't ask for the same scheme again until its next page commits.
  - Schemes with no app show a notice.
  - Never handed off: `file:`, `javascript:`, `data:`, `blob:`, `about:`, `x-apple…`, `applescript:`, `help:`, `x-man-page:`, and network-volume and shell schemes (`smb:`, `afp:`, `ftp:`, `ssh:`, `telnet:`, `vnc:`…). Links over 8 KiB are refused.
  - Private tabs can hand off too.
- Manual suspension under **Tools → Suspend Current Tab**. Resuming reloads the URL.
- System WebKit's GPU rendering; the local WebGPU cube renders successfully on the tested Mac. No application-side screenshot or texture-copy pipeline.
- **Settings** (⌘, or the gear in the sidebar footer) and **History** (⌘Y or the clock) open in the page card, as the Request Lab does. Settings holds the default browser, the search engine and search suggestions for every Space, and the current Space's history, website data, site permissions (agent trust included) and extensions. History lists the Space's pages by day, searchable, with each removable and recent ranges clearable.
- Address suggestions from history, open tabs and, unless turned off in Settings, the search engine. Only searches are sent to the engine (never addresses), without cookies, and never from the Private Space.
- Native **Request lab**: HTTP methods, custom headers and body, status/timing/size, formatted JSON responses, and cURL export. Requests run off the UI thread, have a 30-second timeout, and cap displayed responses at 2 MiB. Redirects are returned for inspection rather than automatically followed.
- WebKit Inspector and a small opt-in local MCP server.

On Mac, use ⌘T, ⌘O (open file), ⌘W, ⌘⇧T, ⌘L, ⌘R, ⌘[ / ⌘], Control-Tab, **⌘⇧C** (copy URL), ⌘⇧N (private tab), ⌘⇧J (request lab), ⌘Y (history), ⌘, (settings), and ⌘⌥I (Inspector). Linux uses Control in place of Command.

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

Agents see nothing until you open a tab to them with the agent button (beside Request Lab) or a tab's agent icon. There are two levels, and only you can grant either, in Browser:

- **Shared** (untrusted): one tab, in memory, on its exact origin (scheme/host/port), until you stop it, the tab changes origin, is suspended or closed, or Browser quits. Agents read only the text a person can see (hidden, off-screen, tiny, transparent and background-colored text is left out), click and type as real input (`isTrusted` events, which need the tab on screen; otherwise they fall back to scripted events and say so), and stay on the origin. Before a click that may submit or change something (a form's submit button, or labels like Pay, Send, Delete), Browser asks you. Password and file fields can't be filled. The card glows in the accent color.
- **Trusted**: a site you trust, remembered with its site permissions (per Space) until you stop trusting it there or reset the site's permissions. Every tab on it is open to agents, with no prompts: screenshots, HTML and an accessibility outline, console messages and uncaught errors, network loads (URL, method, status, type, size, time), JavaScript in the page, real key presses (tab in front), uploads of local files (25 MB in all), and navigation anywhere, skipping the unsaved-work prompt. The card glows amber. Trust only sites you develop or fully control.

Agent scripts run in a WebKit content world of their own, which shares the page's document but none of its JavaScript, so a page can't rewrite what an agent reads or detect it reading. Their inputs travel as data, never spliced into code. The console hook and network log run only while a site is trusted. Each action checks the tab's access before every step, so stopping access in Browser stops an action partway; a brief brightening of the card's glow shows each one.

Tools: `browser_tabs`, `browser_read_page`, `browser_navigate`, `browser_click`, `browser_fill` for both levels; `browser_screenshot`, `browser_read_dom`, `browser_console`, `browser_network`, `browser_eval`, `browser_press_key`, `browser_upload` for trusted sites. Selectors must match exactly one element. Page content is untrusted website data.

`browser_navigate` acknowledges `navigation_requested`, not a completed load: in a shared tab a local confirmation may still be required. Actions return an error while a page check or confirmation is pending; callers can retry after the user decides.

The transport is a Unix socket with mode `0600` inside a `0700` profile directory, with bounded messages, concurrency, and request deadlines. There is no TCP listener or website-to-native command bridge. This restricts access to your OS account, **not to one particular agent client**: any process running as you can connect and use tabs that are shared or on trusted sites. A local transport does not imply the model runs locally.

## Development checks

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
# macOS: real WebKit checks in an ephemeral data store
swift scripts/check-work-protection.swift
swift scripts/check-app-links.swift
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

For agent access end to end, run a debug build against the fixture server with `BROWSER_TEST_AGENT=share` or `trust` (debug builds only: it opens the loopback fixture page to agents without the prompts), open `http://127.0.0.1:8765/agent`, keep the window on screen, and run:

```bash
python3 scripts/check-agent.py --profile /tmp/browser-dev --mode share
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
