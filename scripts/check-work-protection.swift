// Native WebKit regression tests for the SAME isolated script used by Gradient.
// Run from the repo root: swift scripts/check-work-protection.swift
//
// Only user-produced work counts, so edits here are real AppKit key and mouse
// events delivered to an offscreen window (trusted DOM events), while
// page-script edits check that settled pages stay clear.
import AppKit
import WebKit

let source = try String(contentsOfFile: "src/engine/work-state.js", encoding: .utf8)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)

@MainActor final class Checks: NSObject, WKNavigationDelegate {
    let world = WKContentWorld.world(name: "Gradient Work Protection")
    var view: WKWebView!
    var window: NSWindow!
    var count = 0
    func start() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.mediaTypesRequiringUserActionForPlayback = []
        configuration.userContentController.addUserScript(WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: false, in: world))
        view = WKWebView(frame: CGRect(x: 0, y: 0, width: 800, height: 900), configuration: configuration)
        view.navigationDelegate = self
        // Offscreen but ordered in, so WebKit treats input as real user input.
        window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 800, height: 900),
                          styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        app.activate(ignoringOtherApps: true)
        window.makeFirstResponder(view)
        view.loadHTMLString("""
        <style>*{display:block;margin:4px} canvas{width:80px;height:40px} iframe{width:200px;height:60px}</style>
        <form id="form"><textarea id="draft"></textarea><input id="text" value="saved">
        <input id="check" type="checkbox"><input type="range"><input type="color">
        <input id="password" type="password"><input id="query" type="search">
        <select><option selected>A</option><option selected>B</option></select></form>
        <div id="editor" contenteditable="true">Original</div><div id="shadow"></div>
        """, baseURL: URL(string: "https://fixture.invalid/"))
    }
    func page(_ script: String) async throws {
        _ = try await view.evaluateJavaScript("(() => { \(script); return true; })()")
    }
    func settle() async throws { try await Task.sleep(nanoseconds: 250_000_000) }
    /// Types into whatever the page has focused, as a person would.
    func type(_ text: String) async throws {
        for character in text {
            for kind in [NSEvent.EventType.keyDown, .keyUp] {
                let event = NSEvent.keyEvent(with: kind, location: .zero, modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                    context: nil, characters: String(character), charactersIgnoringModifiers: String(character),
                    isARepeat: false, keyCode: 0)!
                window.sendEvent(event)
            }
        }
        try await settle()
    }
    /// Clicks the centre of the element an expression evaluates to.
    func click(_ element: String) async throws {
        let point = try await view.evaluateJavaScript(
            "(() => { const r = (\(element)).getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()") as! [Double]
        let location = NSPoint(x: point[0], y: view.bounds.height - point[1])
        for kind in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = NSEvent.mouseEvent(with: kind, location: location, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
            window.sendEvent(event)
        }
        try await settle()
    }
    func state() async throws -> [String: Any] {
        let value: Any = try await withCheckedThrowingContinuation { continuation in
            view.evaluateJavaScript("JSON.stringify(globalThis.__browserWorkState())", in: nil, in: world) { result in
                continuation.resume(with: result)
            }
        }
        guard let json = value as? String, !json.contains("DISPOSABLE") else { fatalError("Field value leaked in report") }
        return try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
    }
    func expect(_ risk: Int, _ name: String) async throws {
        let actual = try await state()
        guard actual["risk"] as? Int == risk else { fatalError("FAIL \(name): expected \(risk), got \(actual["risk"]!)") }
        count += 1
        print("PASS \(name)")
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor in
            do {
                try await expect(0, "untouched form including normalized controls is clear")
                try await page("if (typeof globalThis.__browserWorkState !== 'undefined') throw Error('Isolated function exposed')")
                // Pages fill their own fields (a search box holding the query).
                try await page("draft.value = 'page wrote this'; text.value = 'page'; check.checked = true")
                try await page("document.body.append(document.createElement('canvas'))")
                try await page("globalThis.opaqueFrame = document.createElement('iframe'); opaqueFrame.src = 'data:text/html,<textarea></textarea>'; document.body.append(opaqueFrame)")
                try await settle()
                try await expect(0, "settled page: page-set values, canvas and opaque frame are clear")
                try await page("form.reset()")

                try await page("draft.focus()")
                try await type("hi")
                try await expect(1, "typed draft is protected")
                let before = try await state()["stamp"] as! String
                try await page("draft.value = 'changed while deciding'")
                let after = try await state()["stamp"] as! String
                guard before != after else { fatalError("Scripted edit did not invalidate consent") }
                try await page("form.reset()")
                try await expect(0, "reset clears typed form risk")

                try await page("text.focus(); text.select()")
                try await type("x")
                try await expect(1, "typed text input is protected")
                try await page("text.defaultValue = text.value")
                try await expect(0, "site updating defaults after save clears risk")

                try await click("check")
                try await expect(1, "clicked checkbox is protected")
                try await page("form.reset()")
                try await expect(0, "checkbox reset")

                try await page("password.focus()")
                try await type("DISPOSABLE")
                try await expect(1, "password edit protected without returning its contents")
                try await page("form.reset()")

                try await page("query.focus()")
                try await type("cats")
                try await expect(0, "typed search query is not work")

                try await page("editor.focus(); getSelection().selectAllChildren(editor); getSelection().collapseToEnd()")
                try await type("!")
                try await expect(1, "rich text edit")
                try await page("editor.innerHTML = 'Original'")
                try await expect(0, "editor reverted to original")

                try await page("shadow.attachShadow({mode:'open'}).innerHTML = '<textarea id=inside></textarea>'; shadow.shadowRoot.querySelector('textarea').focus()")
                try await type("s")
                try await expect(1, "open shadow-root edit")
                try await page("shadow.shadowRoot.querySelector('textarea').value = ''")
                try await expect(0, "shadow-root revert")

                try await page("globalThis.child = document.createElement('iframe'); child.srcdoc = '<textarea></textarea>'; document.body.append(child)")
                try await settle()
                try await expect(0, "clean same-origin child frame")
                try await page("child.contentDocument.querySelector('textarea').focus()")
                try await type("f")
                try await expect(1, "typed same-origin child frame")
                try await page("child.remove(); draft.focus()")

                try await page("globalThis.video = document.createElement('video'); video.muted = true; video.srcObject = document.createElement('canvas').captureStream(1); document.body.append(video); video.play()")
                try await settle()
                try await expect(0, "playing media alone does not block closing")
                try await page("video.pause(); video.srcObject.getTracks().forEach(track => track.stop()); video.remove()")

                try await click("opaqueFrame")
                try await expect(3, "opaque frame the user worked in is conservative")
                try await page("opaqueFrame.remove(); draft.focus()")
                try await click("document.querySelector('canvas')")
                try await expect(3, "canvas application the user worked in is conservative")
                print("PASS \(count) WebKit state cases and page-world isolation")
                exit(0)
            } catch { fputs("FAIL \(error)\n", stderr); exit(1) }
        }
    }
}
let checks = MainActor.assumeIsolated { let checks = Checks(); checks.start(); return checks }
DispatchQueue.main.asyncAfter(deadline: .now() + 60) { fputs("FAIL timeout\n", stderr); exit(1) }
app.run()
