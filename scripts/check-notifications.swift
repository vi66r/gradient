// Native WebKit checks for web notifications.
// Run from the repo root: swift scripts/check-notifications.swift
//
// Gradient shows a page's notifications through WebKit's notification-provider
// C SPI, resolved at runtime (src/engine/notifications.rs). This drives the
// same symbols against the fixture (fixtures/notify.html):
// - a persistent store asks the UI delegate's SPI and honors its answer;
// - the provider receives each notification with its title, body, origin and
//   page, and the page's onshow and onclick fire when told;
// - close() reaches the provider's cancel, and closing a page clears its
//   notifications;
// - a pool seeded with allowed sites starts "granted", and removing a site's
//   policy takes it back to "default" in an open page;
// - an ephemeral (private) store denies without asking.
// Notification Center is not involved: an unbundled script can't use it.
import AppKit
import WebKit

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
setvbuf(stdout, nil, _IOLBF, 0)

let server = Process()
server.executableURL = URL(fileURLWithPath: "/usr/bin/env")
server.arguments = ["python3", "scripts/test-server.py", "--port", "0"]
let output = Pipe()
server.standardOutput = output
server.standardError = FileHandle.nullDevice
try server.run()
let banner = String(data: output.fileHandleForReading.availableData, encoding: .utf8) ?? ""
guard let address = banner.split(separator: " ").last.map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) }) else {
    fatalError("FAIL fixture server: \(banner)")
}
atexit { server.terminate() }
func fail(_ message: String) -> Never {
    fputs("FAIL \(message)\n", stderr)
    exit(1)
}

typealias Ref = UnsafeRawPointer
func function<T>(_ name: String, _: T.Type) -> T {
    guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), name) else { fail("WebKit lacks \(name)") }
    return unsafeBitCast(symbol, to: T.self)
}
let setProvider = function("WKNotificationManagerSetProvider", (@convention(c) (Ref, UnsafeRawPointer) -> Void).self)
let didShow = function("WKNotificationManagerProviderDidShowNotification", (@convention(c) (Ref, UInt64) -> Void).self)
let didClick = function("WKNotificationManagerProviderDidClickNotification", (@convention(c) (Ref, UInt64) -> Void).self)
let didRemove = function("WKNotificationManagerProviderDidRemoveNotificationPolicies", (@convention(c) (Ref, Ref) -> Void).self)
let pageContext = function("WKPageGetContext", (@convention(c) (Ref) -> Ref).self)
let contextManager = function("WKContextGetNotificationManager", (@convention(c) (Ref) -> Ref).self)
let copyTitle = function("WKNotificationCopyTitle", (@convention(c) (Ref) -> Ref?).self)
let copyBody = function("WKNotificationCopyBody", (@convention(c) (Ref) -> Ref?).self)
let notificationID = function("WKNotificationGetID", (@convention(c) (Ref) -> UInt64).self)
let notificationOrigin = function("WKNotificationGetSecurityOrigin", (@convention(c) (Ref) -> Ref?).self)
let originString = function("WKSecurityOriginCopyToString", (@convention(c) (Ref) -> Ref?).self)
let originFromString = function("WKSecurityOriginCreateFromString", (@convention(c) (Ref) -> Ref).self)
let copyCFString = function("WKStringCopyCFString", (@convention(c) (CFAllocator?, Ref) -> Unmanaged<CFString>?).self)
let createString = function("WKStringCreateWithUTF8CString", (@convention(c) (UnsafePointer<CChar>) -> Ref).self)
let createDictionary = function("WKMutableDictionaryCreate", (@convention(c) () -> Ref).self)
let setItem = function("WKDictionarySetItem", (@convention(c) (Ref, Ref, Ref) -> Bool).self)
let createBoolean = function("WKBooleanCreate", (@convention(c) (Bool) -> Ref).self)
let createArray = function("WKArrayCreate", (@convention(c) (UnsafePointer<Ref>, Int) -> Ref).self)
let arraySize = function("WKArrayGetSize", (@convention(c) (Ref) -> Int).self)
let arrayItem = function("WKArrayGetItemAtIndex", (@convention(c) (Ref, Int) -> Ref).self)
let uint64Value = function("WKUInt64GetValue", (@convention(c) (Ref) -> UInt64).self)
let release = function("WKRelease", (@convention(c) (Ref) -> Void).self)
func text(_ value: Ref?) -> String {
    guard let value else { return "" }
    defer { release(value) }
    return (copyCFString(nil, value)?.takeRetainedValue() as String?) ?? ""
}

struct Shown { let id: UInt64; let title: String; let body: String; let origin: String; let page: Ref? }
nonisolated(unsafe) var shown: [Shown] = []
nonisolated(unsafe) var cancelled: [UInt64] = []
nonisolated(unsafe) var cleared: [UInt64] = []
nonisolated(unsafe) var allowed: [String] = []

typealias Show = @convention(c) (Ref?, Ref?, UnsafeRawPointer?) -> Void
typealias One = @convention(c) (Ref?, UnsafeRawPointer?) -> Void
typealias Permissions = @convention(c) (UnsafeRawPointer?) -> Ref?
struct Provider { var version: Int32; var info: UnsafeRawPointer?; var show: Show; var cancel: One; var destroy: One; var add: One; var remove: One; var permissions: Permissions; var clear: One }
let provider = UnsafeMutablePointer<Provider>.allocate(capacity: 1)
provider.initialize(to: Provider(version: 0, info: nil,
    show: { page, notification, _ in
        let notification = notification!
        shown.append(Shown(id: notificationID(notification), title: text(copyTitle(notification)), body: text(copyBody(notification)),
                           origin: notificationOrigin(notification).map { text(originString($0)) } ?? "", page: page))
    },
    cancel: { notification, _ in cancelled.append(notificationID(notification!)) },
    destroy: { _, _ in }, add: { _, _ in }, remove: { _, _ in },
    permissions: { _ in
        let dictionary = createDictionary()
        let yes = createBoolean(true)
        for site in allowed { let key = createString(site); _ = setItem(dictionary, key, yes); release(key) }
        release(yes)
        return dictionary
    },
    clear: { ids, _ in for index in 0..<arraySize(ids!) { cleared.append(uint64Value(arrayItem(ids!, index))) } }))

@MainActor final class Checks: NSObject, WKNavigationDelegate, WKUIDelegate {
    let base: String
    init(base: String) { self.base = base }
    let identifier = UUID()
    var asked: [String] = []
    var answer = true
    var loaded: Set<ObjectIdentifier> = []
    var windows: [NSWindow] = []
    var count = 0

    func pass(_ name: String) { count += 1; print("PASS \(name)") }

    func pageRef(_ view: WKWebView) -> Ref {
        typealias Getter = @convention(c) (AnyObject, Selector) -> Ref?
        let selector = NSSelectorFromString("_pageRefForTransitionToWKWebView")
        guard view.responds(to: selector), let page = unsafeBitCast(view.method(for: selector), to: Getter.self)(view, selector) else {
            fail("no _pageRefForTransitionToWKWebView")
        }
        return page
    }
    func manager(_ view: WKWebView) -> Ref { contextManager(pageContext(pageRef(view))) }

    /// A page in its own configuration (and so its own pool), as Gradient makes them.
    func open(_ url: String, store: WKWebsiteDataStore) async -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = store
        let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 600, height: 400), configuration: configuration)
        view.navigationDelegate = self
        view.uiDelegate = self
        setProvider(manager(view), UnsafeRawPointer(provider))
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.orderFront(nil)
        windows.append(window)
        view.load(URLRequest(url: URL(string: url)!))
        await wait(until: { self.loaded.contains(ObjectIdentifier(view)) }, "load \(url)")
        return view
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { loaded.insert(ObjectIdentifier(webView)) }

    func wait(until condition: () async -> Bool, _ what: String) async {
        for _ in 0..<60 {
            if await condition() { return }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        fail("timed out: \(what)")
    }

    func state(_ view: WKWebView) async -> (permission: String, events: [String]) {
        guard let json = try? await view.evaluateJavaScript("state()") as? String,
              let value = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { fail("no state") }
        return (value["permission"] as? String ?? "", value["events"] as? [String] ?? [])
    }

    func request(_ view: WKWebView) async -> String {
        (try? await view.callAsyncJavaScript("return await Notification.requestPermission()", contentWorld: .page) as? String) ?? "error"
    }

    func run() async {
        let origin = base
        let other = base.replacingOccurrences(of: "127.0.0.1", with: "localhost")
        let store = WKWebsiteDataStore(forIdentifier: identifier)

        let page = await open(origin + "/notify", store: store)
        guard await state(page).permission == "default" else { fail("fresh permission \(await state(page))") }
        let granted = await request(page)
        guard granted == "granted", asked == [origin], await state(page).permission == "granted" else {
            fail("request: \(granted), asked \(asked)")
        }
        pass("a persistent store asks the UI delegate with the page's origin; Allow grants")

        _ = try? await page.evaluateJavaScript("notify('Hello'), 1")
        await wait(until: { !shown.isEmpty }, "provider show")
        guard let first = shown.first, first.title == "Hello", first.body.hasPrefix("From the fixture"), first.origin == origin,
              first.page == pageRef(page) else { fail("shown \(shown)") }
        pass("the provider gets the title, body, origin and the page that notified")

        didShow(manager(page), first.id)
        didClick(manager(page), first.id)
        await wait(until: { await self.state(page).events == ["shown Hello", "clicked Hello"] }, "onshow/onclick")
        pass("DidShow and DidClick fire the page's onshow and onclick")

        _ = try? await page.evaluateJavaScript("last.close(), 1")
        await wait(until: { cancelled == [first.id] }, "cancel")
        pass("close() reaches the provider's cancel")

        _ = try? await page.evaluateJavaScript("notify('Left open'), 1")
        await wait(until: { shown.count == 2 }, "second show")
        let leftOpen = shown[1].id
        // What a WKWebView's dealloc does when Gradient drops a tab's page.
        let close = NSSelectorFromString("_close")
        guard page.responds(to: close) else { fail("no _close") }
        page.perform(close)
        await wait(until: { cleared.contains(leftOpen) }, "clear on page close")
        pass("closing a page clears the notifications it left open")

        allowed = [other]
        let seeded = await open(other + "/notify", store: store)
        guard await state(seeded).permission == "granted" else { fail("seeded \(await state(seeded))") }
        pass("a new pool seeded with the allowed site starts granted, without asking")
        var origins = [originFromString(createString(other))]
        let array = createArray(&origins, 1)
        didRemove(manager(seeded), array)
        await wait(until: { await self.state(seeded).permission == "default" }, "policy removal")
        pass("removing the site's policy takes an open page back to default (Reset)")

        let before = asked.count
        let ephemeral = await open(origin + "/notify", store: .nonPersistent())
        let denied = await request(ephemeral)
        guard denied == "denied", asked.count == before else { fail("ephemeral: \(denied), asked \(asked)") }
        pass("an ephemeral (private) store denies without asking")

        try? await WKWebsiteDataStore.remove(forIdentifier: identifier)
        print("PASS \(count) notification cases")
        exit(0)
    }

    @objc(_webView:requestNotificationPermissionForSecurityOrigin:decisionHandler:)
    func notifications(_ webView: WKWebView, origin: WKSecurityOrigin, decisionHandler: @escaping (Bool) -> Void) {
        asked.append("\(origin.protocol)://\(origin.host)" + (origin.port == 0 ? "" : ":\(origin.port)"))
        decisionHandler(answer)
    }
}
MainActor.assumeIsolated {
    let checks = Checks(base: address)
    objc_setAssociatedObject(app, "checks", checks, .OBJC_ASSOCIATION_RETAIN)
    Task { await checks.run() }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 60) { fail("timeout") }
app.run()
