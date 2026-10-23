// Native WebKit and LaunchServices checks for app-link handoff (mailto:, tel:…).
// Run from the repo root: swift scripts/check-app-links.swift
//
// Browser decides app links in its navigation delegate from the frame WebKit
// reports (src/engine/protection.rs `handoff`, unit-tested in Rust). This
// checks the WebKit facts that policy relies on: main-frame script redirects
// and link clicks arrive with a main target frame, embedded frames with a
// subframe target, and `target=_blank` links with no target frame but the
// clicking frame as source. Every app link is cancelled, so the page stays put.
// It also checks the macOS handler lookup Browser uses. No app is opened.
import AppKit
import WebKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

struct Seen { let url: String; let target: Bool?; let source: Bool? }

@MainActor final class Checks: NSObject, WKNavigationDelegate, WKUIDelegate {
    var view: WKWebView!
    var window: NSWindow!
    var seen: [Seen] = []
    var count = 0
    var started = false
    func start() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        view = WKWebView(frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
        view.navigationDelegate = self
        view.uiDelegate = self
        window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 800, height: 600),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        app.activate(ignoringOtherApps: true)
        window.makeFirstResponder(view)
        view.loadHTMLString("""
        <style>*{display:block;margin:6px} iframe{width:300px;height:80px}</style>
        <a id="tel" href="tel:+15551234567">Call</a>
        <a id="blank" target="_blank" href="sms:+15551234567">Text</a>
        <iframe id="frame" srcdoc="<a id=inner target=_blank href='mailto:ad@example.com'>Ad</a>"></iframe>
        """, baseURL: URL(string: "https://fixture.invalid/"))
    }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        let url = action.request.url?.absoluteString ?? ""
        let scheme = url.split(separator: ":").first.map { $0.lowercased() } ?? ""
        if ["mailto", "tel", "sms"].contains(scheme) {
            // `sourceFrame` is nullable in practice; read it defensively.
            let source = (action.perform(NSSelectorFromString("sourceFrame"))?.takeUnretainedValue() as? WKFrameInfo)?.isMainFrame
            seen.append(Seen(url: url, target: action.targetFrame?.isMainFrame, source: source))
            decisionHandler(.cancel)
        } else {
            decisionHandler(.allow)
        }
    }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        fatalError("FAIL a cancelled app link still asked for a new window: \(action.request.url?.absoluteString ?? "")")
    }
    func settle() async throws { try await Task.sleep(nanoseconds: 400_000_000) }
    func page(_ script: String) async throws {
        _ = try await view.evaluateJavaScript("(() => { \(script); return true; })()")
        try await settle()
    }
    func click(_ rect: String) async throws {
        let point = try await view.evaluateJavaScript(
            "(() => { const r = \(rect); return [r.x + r.width / 2, r.y + r.height / 2]; })()") as! [Double]
        let location = NSPoint(x: point[0], y: view.bounds.height - point[1])
        for kind in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            window.sendEvent(NSEvent.mouseEvent(with: kind, location: location, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
        try await settle()
    }
    func expect(_ name: String, url prefix: String, target: Bool?, source: Bool?) {
        guard let last = seen.last, last.url.hasPrefix(prefix) else {
            fatalError("FAIL \(name): no decision for \(prefix); saw \(seen.map(\.url))")
        }
        guard last.target == target, target != nil || last.source == source else {
            fatalError("FAIL \(name): target main=\(String(describing: last.target)) source main=\(String(describing: last.source))")
        }
        count += 1
        print("PASS \(name)")
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !started else { return }
        started = true
        Task { @MainActor in
            do {
                try await page("location.href = 'mailto:someone@example.com?subject=hi'")
                expect("main-frame script redirect has a main target frame (prompts)", url: "mailto:", target: true, source: nil)

                try await click("document.getElementById('tel').getBoundingClientRect()")
                expect("main-frame link click has a main target frame (prompts)", url: "tel:", target: true, source: nil)

                try await click("document.getElementById('blank').getBoundingClientRect()")
                expect("target=_blank click: no target frame, main source (prompts)", url: "sms:", target: nil, source: true)

                try await page("const f = document.createElement('iframe'); f.src = 'mailto:frame@example.com'; document.body.append(f)")
                expect("embedded frame load has a subframe target (silently dropped)", url: "mailto:frame@", target: false, source: nil)

                try await click("(() => { const f = document.getElementById('frame').getBoundingClientRect(), a = document.getElementById('frame').contentDocument.getElementById('inner').getBoundingClientRect(); return {x: f.x + a.x, y: f.y + a.y, width: a.width, height: a.height}; })()")
                expect("target=_blank click inside a frame: subframe source (silently dropped)", url: "mailto:ad@", target: nil, source: false)

                guard view.url?.absoluteString == "https://fixture.invalid/" else {
                    fatalError("FAIL page moved to \(view.url?.absoluteString ?? "nil")")
                }
                count += 1
                print("PASS cancelled app links leave the page in place")

                let workspace = NSWorkspace.shared
                guard let mail = workspace.urlForApplication(toOpen: URL(string: "mailto:someone@example.com")!) else {
                    fatalError("FAIL no mailto: handler on this Mac")
                }
                let name = FileManager.default.displayName(atPath: mail.path)
                count += 1
                print("PASS mailto: handler resolves to “\(name)”")
                guard workspace.urlForApplication(toOpen: URL(string: "browser-no-such-scheme-\(getpid()):x")!) == nil else {
                    fatalError("FAIL an unregistered scheme resolved to an app")
                }
                count += 1
                print("PASS unregistered scheme has no handler (notice path)")
                print("PASS \(count) app-link cases")
                exit(0)
            } catch { fputs("FAIL \(error)\n", stderr); exit(1) }
        }
    }
}
let checks = MainActor.assumeIsolated { let checks = Checks(); checks.start(); return checks }
DispatchQueue.main.asyncAfter(deadline: .now() + 60) { fputs("FAIL timeout\n", stderr); exit(1) }
app.run()
