// Real WebKit checks for local files, in an ephemeral data store.
// Run from the repo root: swift scripts/check-file-urls.swift
//
// Confirms what Gradient relies on: `loadFileURL:allowingReadAccessToURL:` with
// the file's folder loads a local page and its relative image, a local page can
// link to a sibling file, and a web page can't take the main frame to file:
// even when the navigation delegate would allow it (Gradient's own policy in
// `engine/protection.rs` is a second, explicit layer). INFO lines record
// behaviour Gradient doesn't depend on.
import AppKit
import WebKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

struct Decision { let url: String; let main: Bool; let source: String?; let type: WKNavigationType }

@MainActor final class Checks: NSObject, WKNavigationDelegate {
    var view: WKWebView!
    var decisions: [Decision] = []
    var finished = 0
    var failed = 0
    var count = 0
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("browser-file-check-\(UUID().uuidString)")
    var site: URL { root.appendingPathComponent("my site") }
    var index: URL { site.appendingPathComponent("index.html") }

    func makeView() -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 600, height: 400), configuration: configuration)
        view.navigationDelegate = self
        return view
    }

    func fixtures() throws {
        try FileManager.default.createDirectory(at: site, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("other"), withIntermediateDirectories: true)
        let pixel = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1, pixelsHigh: 1, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        try pixel.representation(using: .png, properties: [:])!.write(to: site.appendingPathComponent("pixel 1.png"))
        try """
        <title>Index</title><img id="pixel" src="pixel%201.png">
        <a id="next" href="page2.html">next</a><a id="out" href="../other/outside.html">out</a>
        """.write(to: index, atomically: true, encoding: .utf8)
        try "<title>Page 2</title>two".write(to: site.appendingPathComponent("page2.html"), atomically: true, encoding: .utf8)
        try "<title>Outside</title>outside".write(to: root.appendingPathComponent("other/outside.html"), atomically: true, encoding: .utf8)
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        // `sourceFrame` is nonnull in the SDK but can be nil; read it via KVC.
        let source = (action.value(forKey: "sourceFrame") as? WKFrameInfo)?.request.url?.absoluteString
        decisions.append(Decision(url: action.request.url?.absoluteString ?? "", main: action.targetFrame?.isMainFrame ?? true,
                                  source: source, type: action.navigationType))
        // Allow everything: these checks show what WebKit itself enforces.
        decisionHandler(.allow)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { finished += 1 }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { failed += 1 }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { failed += 1 }

    /// Waits for the next finish or failure; returns whether it finished.
    func settle(_ timeout: Double = 5) async throws -> Bool? {
        let (done, fail) = (finished, failed)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if finished > done { return true }
            if failed > fail { return false }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        return nil
    }
    func js(_ script: String) async throws -> Any? { try await view.evaluateJavaScript(script) }
    func pass(_ name: String) { count += 1; print("PASS \(name)") }
    func fail(_ name: String) -> Never { fputs("FAIL \(name)\n", stderr); exit(1) }

    func run() async throws {
        try fixtures()
        defer { try? FileManager.default.removeItem(at: root) }
        view = makeView()

        view.loadFileURL(index, allowingReadAccessTo: site)
        guard try await settle() == true else { fail("local page did not load") }
        let image = try await js("pixel.complete && pixel.naturalWidth === 1") as? Bool
        guard image == true else { fail("relative image in the granted folder did not load") }
        pass("loadFileURL with folder access loads the page and its relative image (path with a space)")
        let first = decisions.last!
        print("INFO app-initiated loadFileURL decision: main=\(first.main) source=\(first.source ?? "nil") type=\(first.type.rawValue)")

        decisions.removeAll()
        _ = try await js("next.click(); true")
        guard try await settle() == true, view.url?.lastPathComponent == "page2.html" else { fail("local page could not link to a sibling file") }
        guard let link = decisions.first(where: { $0.url.hasSuffix("page2.html") }), link.source?.hasPrefix("file:") == true, link.main
        else { fail("sibling link decision lacked a file: source frame") }
        pass("local page links to a sibling file; the decision's source frame is file:")

        view.goBack()
        _ = try await settle()
        _ = try await js("out.click(); true")
        let outside = try await settle(3)
        print("INFO link outside the granted folder: \(outside == true ? "loaded" : outside == false ? "failed" : "no load") url=\(view.url?.lastPathComponent ?? "nil")")

        let plain = makeView()
        plain.load(URLRequest(url: index))
        view = plain
        let plainResult = try await settle(3)
        let plainImage = plainResult == true ? try await js("pixel.complete && pixel.naturalWidth === 1") as? Bool : nil
        print("INFO plain loadRequest of a file URL (unsandboxed harness): \(plainResult == true ? "finished" : plainResult == false ? "failed" : "no load"), relative image loaded: \(plainImage.map(String.init(describing:)) ?? "n/a")")

        // A real web origin with real history: a loopback HTTP server.
        let target = index.absoluteString
        let served = root.appendingPathComponent("served")
        try FileManager.default.createDirectory(at: served, withIntermediateDirectories: true)
        try "<a id=a href='\(target)'>file</a>".write(to: served.appendingPathComponent("web.html"), atomically: true, encoding: .utf8)
        let port = Int.random(in: 20000...40000)
        let server = Process()
        server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        server.arguments = ["-m", "http.server", String(port), "--bind", "127.0.0.1", "--directory", served.path]
        server.standardOutput = FileHandle.nullDevice
        server.standardError = FileHandle.nullDevice
        try server.run()
        defer { server.terminate() }
        let web = makeView()
        view = web
        var up = false
        for _ in 0..<40 where !up {
            try await Task.sleep(nanoseconds: 250_000_000)
            web.load(URLRequest(url: URL(string: "http://127.0.0.1:\(port)/web.html")!))
            if try await settle() == true { up = try await js("!!document.getElementById('a')") as? Bool == true }
        }
        guard up else { fail("web fixture did not load") }
        for (name, script) in [("location assignment", "location.href = '\(target)'"),
                               ("link click", "a.click()"),
                               ("window.open", "window.open('\(target)', '_self')")] {
            decisions.removeAll()
            _ = try await js("\(script); true")
            _ = try await settle(1.5)
            guard web.url?.scheme == "http" else { fail("web page took the main frame to file: via \(name)") }
            let asked = decisions.contains { $0.url.hasPrefix("file:") }
            pass("web page cannot navigate the main frame to file: (\(name)); delegate asked: \(asked)")
        }
        decisions.removeAll()
        _ = try await js("const f = document.createElement('iframe'); f.src = '\(target)'; document.body.append(f); true")
        try await Task.sleep(nanoseconds: 1_000_000_000)
        print("INFO web page file: iframe reached the delegate: \(decisions.contains { $0.url.hasPrefix("file:") })")

        // The address field on a tab showing a web page: Gradient's own load.
        decisions.removeAll()
        web.loadFileURL(index, allowingReadAccessTo: site)
        guard try await settle() == true, web.url?.isFileURL == true,
              try await js("pixel.complete && pixel.naturalWidth === 1") as? Bool == true
        else { fail("loadFileURL from a web page did not load the file with its image") }
        let fromWeb = decisions.first { $0.url.hasPrefix("file:") }
        pass("loadFileURL replaces a web page (decision source: \(fromWeb?.source ?? "nil"), so Gradient matches its expected path, not the source)")
        web.goBack()
        _ = try await settle()
        guard web.url?.scheme == "http" else { fail("Back to the web page failed") }
        decisions.removeAll()
        web.goForward()
        guard try await settle() == true, web.url?.isFileURL == true,
              try await js("pixel.complete && pixel.naturalWidth === 1") as? Bool == true
        else { fail("Forward to the local page failed") }
        let forward = decisions.first { $0.url.hasPrefix("file:") }
        guard forward?.type == .backForward else { fail("Forward to a file was not a back/forward decision") }
        pass("Forward from a web page back to a local page reloads it with its image (type backForward)")
        print("PASS \(count) local-file WebKit cases")
    }
}

MainActor.assumeIsolated {
    let checks = Checks()
    Task { @MainActor in
        do { try await checks.run(); exit(0) } catch { fputs("FAIL \(error)\n", stderr); exit(1) }
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 60) { fputs("FAIL timeout\n", stderr); exit(1) }
app.run()
