// Native WebKit checks for popups and site permissions.
// Run from the repo root: swift scripts/check-popups-permissions.swift
//
// Browser's UI delegate (src/engine/site.rs) builds popups from the
// configuration WebKit hands it and asks the user before a site uses a
// device. This checks the WebKit behavior that relies on, against the real
// fixtures (fixtures/sites.html) with real AppKit clicks: a popup built that
// way keeps `window.opener` and the opener's data store, posts back and
// closes itself through `webViewDidClose:`; a timer can't open one; links and
// ⌘-clicks are told apart; device requests carry the requesting frame's own
// origin in its frame info (so a cross-origin frame can be refused), and
// WebKit honors Grant and Deny. Cameras are WebKit's mock devices, so no camera, microphone
// or TCC prompt is involved.
import AppKit
import WebKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
setvbuf(stdout, nil, _IOLBF, 0)

// The fixture server, on a free loopback port.
let server = Process()
server.executableURL = URL(fileURLWithPath: "/usr/bin/env")
server.arguments = ["python3", "scripts/test-server.py", "--port", "0"]
let output = Pipe()
server.standardOutput = output
server.standardError = FileHandle.nullDevice
try server.run()
let banner = String(data: output.fileHandleForReading.availableData, encoding: .utf8) ?? ""
guard let address = banner.split(separator: " ").last.map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) }) else {
    fail("fixture server: \(banner)")
}
atexit { server.terminate() }
func fail(_ message: String) -> Never {
    fputs("FAIL \(message)\n", stderr)
    exit(1)
}

func setPreference(_ preferences: WKPreferences, _ name: String, _ value: Bool) {
    let selector = NSSelectorFromString(name)
    guard preferences.responds(to: selector) else { fail("WebKit lacks \(name)") }
    typealias Setter = @convention(c) (AnyObject, Selector, Bool) -> Void
    unsafeBitCast(preferences.method(for: selector), to: Setter.self)(preferences, selector, value)
}

struct Request { let kind: String; let origin: String; let main: Bool; let frame: String }

@MainActor final class Checks: NSObject, WKNavigationDelegate, WKUIDelegate {
    let base: String
    init(base: String) { self.base = base }
    var view: WKWebView!
    var window: NSWindow!
    var popup: WKWebView?
    var popupWindow: NSWindow?
    var created: [(url: String, type: WKNavigationType, command: Bool)] = []
    var closed = 0
    var requests: [Request] = []
    var answer = false
    var loaded = 0
    var count = 0

    func start() {
        // As Browser's `page_configuration`, with an ephemeral store like a
        // private tab so the popup's shared store is visible.
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        setPreference(configuration.preferences, "_setMediaDevicesEnabled:", true)
        setPreference(configuration.preferences, "_setMockCaptureDevicesEnabled:", true)
        view = WKWebView(frame: CGRect(x: 0, y: 0, width: 900, height: 1000), configuration: configuration)
        view.navigationDelegate = self
        view.uiDelegate = self
        window = offscreen(view)
        view.load(URLRequest(url: URL(string: base + "/sites")!))
    }

    func offscreen(_ view: WKWebView) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: view.frame.width, height: view.frame.height),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        app.activate(ignoringOtherApps: true)
        window.makeFirstResponder(view)
        return window
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loaded += 1
        if webView == view && loaded == 1 { Task { await run() } }
    }

    func text(_ id: String, in view: WKWebView? = nil) async throws -> String {
        try await (view ?? self.view).evaluateJavaScript("document.getElementById('\(id)').textContent") as? String ?? ""
    }

    /// A real mouse click at an element's center; `frame` reaches into an iframe.
    func click(_ id: String, in target: WKWebView? = nil, frame: String? = nil, command: Bool = false) async throws {
        let target = target ?? view!
        let script = frame.map { "(() => { const f = document.getElementById('\($0)'), r = f.getBoundingClientRect(), e = f.contentDocument ? f.contentDocument.getElementById('\(id)').getBoundingClientRect() : {x: 8, y: 8, width: 120, height: 20}; return [r.x + e.x + e.width / 2, r.y + e.y + e.height / 2]; })()" }
            ?? "(() => { const r = document.getElementById('\(id)').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()"
        let point = try await target.evaluateJavaScript(script) as! [Double]
        let location = NSPoint(x: point[0], y: target.bounds.height - point[1])
        let host = target.window!
        // An inactive window would take the first click as activation.
        host.makeKeyAndOrderFront(nil)
        for kind in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = NSEvent.mouseEvent(with: kind, location: location, modifierFlags: command ? [.command] : [],
                                           timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: host.windowNumber,
                                           context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
            host.sendEvent(event)
        }
    }

    func wait(until condition: () async throws -> Bool, _ what: String) async throws {
        for _ in 0..<80 {
            if try await condition() { return }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        fail("timed out: \(what); devices: \((try? await text("devices")) ?? "?"); requests: \(requests)")
    }

    func pass(_ name: String) { count += 1; print("PASS \(name)") }

    func run() async {
        do {
            let origin = base
            let other = base.replacingOccurrences(of: "127.0.0.1", with: "localhost")
            let visits = try await text("storage")

            try await click("late")
            try await Task.sleep(nanoseconds: 3_600_000_000)
            guard try await text("popup") == "Unrequested popup was blocked.", created.isEmpty else {
                fail("a timer opened a window: \(created)")
            }
            pass("a script can't open a window 3 s after a click")

            try await click("sign-in")
            try await wait(until: { self.popup != nil }, "popup")
            guard created.last?.type == .other, created.last?.url == origin + "/sign-in" else {
                fail("window.open arrived as \(String(describing: created.last))")
            }
            pass("window.open arrives as navigation type Other")
            try await wait(until: { self.loaded >= 2 }, "popup load")
            guard try await text("opener", in: popup) == "window.opener is the page that opened this." else {
                fail("popup has no opener")
            }
            pass("the popup keeps window.opener")
            let shared = try await text("storage", in: popup)
            guard shared == visits else { fail("popup store differs: \(visits) vs \(shared)") }
            pass("the popup shares the opener's (private) data store")
            try await click("finish", in: popup)
            try await wait(until: { self.closed == 1 }, "webViewDidClose")
            pass("window.close() reaches webViewDidClose:")
            // As Browser does when the tab closes: drop the view.
            popupWindow?.contentView = nil
            popupWindow?.close()
            popup = nil
            popupWindow = nil
            try await wait(until: { try await self.text("popup").hasSuffix("Popup closed.") }, "opener sees the popup closed")
            let said = try await text("popup")
            guard said.hasPrefix("Popup said: {\"token\":\"fixture-"), said.contains("from \(origin)") else {
                fail("opener got \(said)")
            }
            pass("the opener received the popup's postMessage, then saw it close")

            try await click("link")
            try await wait(until: { self.created.count == 2 }, "link window")
            guard created[1].type == .linkActivated, !created[1].command else { fail("link arrived as \(created[1])") }
            try await click("link", command: true)
            try await wait(until: { self.created.count == 3 }, "⌘-click window")
            guard created[2].type == .linkActivated, created[2].command else { fail("⌘-click arrived as \(created[2])") }
            pass("target=_blank links arrive as LinkActivated, ⌘-click with Command")
            popupWindow?.close(); popup = nil

            answer = false
            try await click("av")
            try await wait(until: { try await self.text("devices").hasPrefix("camera and microphone: NotAllowedError") }, "denied camera")
            guard let first = requests.first, first.kind == "camera+microphone", first.origin == origin, first.main else {
                fail("camera request: \(requests)")
            }
            pass("camera and microphone arrive as one request with the page's origin; Deny refuses")
            answer = true
            try await click("mic")
            try await wait(until: { try await self.text("devices").hasPrefix("microphone: allowed, audio live") }, "granted microphone")
            pass("Grant gives the page a live (mock) microphone")
            await view.setMicrophoneCaptureState(.none)
            try await wait(until: { try await self.text("devices").contains("(audio ended)") }, "capture stopped")
            pass("setting the capture state to None ends the track (Reset)")

            answer = false
            try await click("geo")
            try await wait(until: { try await self.text("devices").hasPrefix("location: error 1") }, "denied location")
            guard requests.last?.kind == "location", requests.last?.origin == origin else { fail("location: \(requests)") }
            pass("geolocation reaches the SPI delegate with the page's origin; Deny refuses")

            try await click("screen")
            try await wait(until: { try await self.text("devices").hasPrefix("screen: NotAllowedError") }, "denied screen")
            guard requests.last?.kind == "screen", requests.last?.origin == origin else { fail("screen: \(requests)") }
            pass("getDisplayMedia reaches the SPI delegate; Deny refuses")

            let before = requests.count
            try await click("ask", frame: "frame")
            try await wait(until: { self.requests.count > before }, "frame request")
            // WebKit names the top page's origin for an embedded frame's
            // request; only the frame info tells the frame's own origin.
            guard let framed = requests.last, framed.origin == origin, framed.frame == other, !framed.main else { fail("frame request: \(requests)") }
            pass("a cross-origin frame's request names the page, but its frame info has \(other), so it can be refused")

            print("PASS \(count) popup and permission cases")
            exit(0)
        } catch { fail("\(error)") }
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        created.append((action.request.url?.absoluteString ?? "", action.navigationType, action.modifierFlags.contains(.command)))
        let opened = WKWebView(frame: CGRect(x: 0, y: 0, width: 480, height: 640), configuration: configuration)
        opened.navigationDelegate = self
        opened.uiDelegate = self
        popup = opened
        popupWindow = offscreen(opened)
        return opened
    }

    func webViewDidClose(_ webView: WKWebView) { closed += 1 }

    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin, initiatedByFrame frame: WKFrameInfo,
                 type: WKMediaCaptureType, decisionHandler: @escaping (WKPermissionDecision) -> Void) {
        let kind = [WKMediaCaptureType.camera: "camera", .microphone: "microphone", .cameraAndMicrophone: "camera+microphone"][type] ?? "?"
        requests.append(Request(kind: kind, origin: Self.origin(origin), main: frame.isMainFrame, frame: Self.origin(frame.securityOrigin)))
        decisionHandler(answer ? .grant : .deny)
    }

    @objc(_webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:)
    func location(_ webView: WKWebView, origin: WKSecurityOrigin, frame: WKFrameInfo, decisionHandler: @escaping (WKPermissionDecision) -> Void) {
        requests.append(Request(kind: "location", origin: Self.origin(origin), main: frame.isMainFrame, frame: Self.origin(frame.securityOrigin)))
        decisionHandler(answer ? .grant : .deny)
    }

    @objc(_webView:requestDisplayCapturePermissionForOrigin:initiatedByFrame:withSystemAudio:decisionHandler:)
    func display(_ webView: WKWebView, origin: WKSecurityOrigin, frame: WKFrameInfo, audio: Bool, decisionHandler: @escaping (Int) -> Void) {
        requests.append(Request(kind: "screen", origin: Self.origin(origin), main: frame.isMainFrame, frame: Self.origin(frame.securityOrigin)))
        decisionHandler(0)
    }

    static func origin(_ origin: WKSecurityOrigin) -> String {
        "\(origin.protocol)://\(origin.host)" + (origin.port == 0 ? "" : ":\(origin.port)")
    }
}
let checks = MainActor.assumeIsolated { let checks = Checks(base: address); checks.start(); return checks }
DispatchQueue.main.asyncAfter(deadline: .now() + 90) { fail("timeout") }
app.run()
