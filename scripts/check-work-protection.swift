// Native WebKit regression tests for the SAME isolated script used by Browser.
// Run from the repo root: swift scripts/check-work-protection.swift
import AppKit
import WebKit

let source = try String(contentsOfFile: "src/engine/work-state.js", encoding: .utf8)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)

@MainActor final class Checks: NSObject, WKNavigationDelegate {
    let world = WKContentWorld.world(name: "Browser Work Protection")
    var view: WKWebView!
    var count = 0
    func start() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.mediaTypesRequiringUserActionForPlayback = []
        configuration.userContentController.addUserScript(WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: false, in: world))
        view = WKWebView(frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
        view.navigationDelegate = self
        view.loadHTMLString("""
        <form id="form"><textarea id="draft"></textarea><input id="text" value="saved">
        <input id="check" type="checkbox"><input type="range"><input type="color">
        <input id="password" type="password"><select id="select"><option>A</option><option>B</option></select>
        <select><option selected>A</option><option selected>B</option></select></form>
        <div id="editor" contenteditable="true">Original</div><div id="shadow"></div>
        """, baseURL: URL(string: "https://fixture.invalid/"))
    }
    func page(_ script: String) async throws {
        _ = try await view.evaluateJavaScript("(() => { \(script); return true; })()")
    }
    func state() async throws -> [String: Any] {
        let value: Any = try await withCheckedThrowingContinuation { continuation in
            view.evaluateJavaScript("JSON.stringify(globalThis.__browserWorkState())", in: nil, in: world) { result in
                continuation.resume(with: result)
            }
        }
        guard let json = value as? String, !json.contains("DISPOSABLE_PASSWORD") else { fatalError("Field value leaked in report") }
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
                try await page("draft.value = 'changed'")
                try await expect(1, "programmatic and autofill-style edits are protected")
                let before = try await state()["stamp"] as! String
                try await page("draft.value = 'changed while deciding'")
                let after = try await state()["stamp"] as! String
                guard before != after else { fatalError("Programmatic edit did not invalidate consent") }
                try await page("form.reset()")
                try await expect(0, "reset clears ordinary form risk")
                try await page("text.value = 'saved again'; text.defaultValue = text.value")
                try await expect(0, "site updating defaults after save clears risk")
                try await page("check.checked = true")
                try await expect(1, "checkbox change")
                try await page("form.reset(); select.selectedIndex = 1")
                try await expect(1, "selection change")
                try await page("form.reset(); password.value = 'DISPOSABLE_PASSWORD'")
                try await expect(1, "password edit protected without returning its contents")
                try await page("form.reset(); editor.dispatchEvent(new InputEvent('beforeinput', {bubbles:true})); editor.innerHTML = '<b>Original</b>'; editor.dispatchEvent(new InputEvent('input', {bubbles:true}))")
                try await expect(1, "rich text formatting edit")
                try await page("editor.innerHTML = 'Original'")
                try await expect(0, "editor reverted to original")
                try await page("shadow.attachShadow({mode:'open'}).innerHTML = '<textarea id=inside></textarea>'; shadow.shadowRoot.querySelector('textarea').value = 'shadow draft'")
                try await expect(1, "open shadow-root edit")
                try await page("shadow.shadowRoot.querySelector('textarea').value = ''")
                try await expect(0, "shadow-root revert")
                try await page("globalThis.child = document.createElement('iframe'); child.srcdoc = '<textarea></textarea>'; document.body.append(child)")
                try await Task.sleep(nanoseconds: 200_000_000)
                try await expect(0, "clean same-origin child frame")
                try await page("child.contentDocument.querySelector('textarea').value = 'frame draft'")
                try await expect(1, "edited same-origin child frame")
                try await page("child.remove(); globalThis.video = document.createElement('video'); video.muted = true; video.srcObject = document.createElement('canvas').captureStream(1); document.body.append(video); video.play()")
                try await Task.sleep(nanoseconds: 200_000_000)
                try await expect(2, "active video is protected")
                try await page("video.pause(); video.srcObject.getTracks().forEach(track => track.stop()); video.remove()")
                try await expect(0, "stopped media is clear")
                try await page("child = document.createElement('iframe'); child.src = 'data:text/html,opaque-frame'; document.body.append(child)")
                try await Task.sleep(nanoseconds: 200_000_000)
                try await expect(3, "opaque child frame is conservative")
                try await page("child.remove()")
                try await page("document.body.append(document.createElement('canvas'))")
                try await expect(3, "canvas application falls back conservatively")
                print("PASS \(count) WebKit state cases and page-world isolation")
                exit(0)
            } catch { fputs("FAIL \(error)\n", stderr); exit(1) }
        }
    }
}
let checks = MainActor.assumeIsolated { let checks = Checks(); checks.start(); return checks }
DispatchQueue.main.asyncAfter(deadline: .now() + 30) { fputs("FAIL timeout\n", stderr); exit(1) }
app.run()
