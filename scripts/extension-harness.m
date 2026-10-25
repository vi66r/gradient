// Loads web extensions (unpacked folders or Safari .appex bundles) into a
// WKWebExtensionController and reports what they did to a fixture page.
// Build: clang -fobjc-arc -framework AppKit -framework WebKit scripts/extension-harness.m -o /tmp/extension-harness
// Serve:  python3 -m http.server 8773 --bind 127.0.0.1 --directory fixtures/extensions/site
// Run:    /tmp/extension-harness http://127.0.0.1:8773/ 5 fixtures/extensions/probe
// Env: HARNESS_WARMUP=s (wait before loading the page), HARNESS_ACTION=1 (open
// the toolbar popup), HARNESS_SHOT=path (snapshot it), HARNESS_NOTABS=1 (no
// tab/window model), HARNESS_STORE=1 (page on its own data store),
// HARNESS_REGRANT=1, HARNESS_BYPASS=1 (permission experiments).
// See docs/extensions-spike.md.
#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>

@class Window;
@interface Tab : NSObject <WKWebExtensionTab>
@property (strong) WKWebView *view;
@property (weak) Window *window;
@end
@interface Window : NSObject <WKWebExtensionWindow>
@property (strong) NSMutableArray<Tab *> *tabs;
@property (strong) NSWindow *native;
@end
@implementation Tab
- (id<WKWebExtensionWindow>)windowForWebExtensionContext:(WKWebExtensionContext *)c { return self.window; }
- (NSUInteger)indexInWindowForWebExtensionContext:(WKWebExtensionContext *)c { return [self.window.tabs indexOfObject:self]; }
- (WKWebView *)webViewForWebExtensionContext:(WKWebExtensionContext *)c { return self.view; }
- (NSString *)titleForWebExtensionContext:(WKWebExtensionContext *)c { return self.view.title; }
- (NSURL *)urlForWebExtensionContext:(WKWebExtensionContext *)c { return self.view.URL; }
- (BOOL)isLoadingCompleteForWebExtensionContext:(WKWebExtensionContext *)c { return !self.view.loading; }
- (BOOL)isSelectedForWebExtensionContext:(WKWebExtensionContext *)c { return YES; }
- (BOOL)shouldGrantPermissionsOnUserGestureForWebExtensionContext:(WKWebExtensionContext *)c { return YES; }
- (BOOL)shouldBypassPermissionsForWebExtensionContext:(WKWebExtensionContext *)c { return getenv("HARNESS_BYPASS") != NULL; }
@end
@implementation Window
- (NSArray<id<WKWebExtensionTab>> *)tabsForWebExtensionContext:(WKWebExtensionContext *)c { return self.tabs; }
- (id<WKWebExtensionTab>)activeTabForWebExtensionContext:(WKWebExtensionContext *)c { return self.tabs.firstObject; }
- (WKWebExtensionWindowType)windowTypeForWebExtensionContext:(WKWebExtensionContext *)c { return WKWebExtensionWindowTypeNormal; }
- (WKWebExtensionWindowState)windowStateForWebExtensionContext:(WKWebExtensionContext *)c { return WKWebExtensionWindowStateNormal; }
- (BOOL)isPrivateForWebExtensionContext:(WKWebExtensionContext *)c { return NO; }
- (CGRect)frameForWebExtensionContext:(WKWebExtensionContext *)c { return self.native.frame; }
- (CGRect)screenFrameForWebExtensionContext:(WKWebExtensionContext *)c { return self.native.screen.frame; }
@end

@interface Delegate : NSObject <WKWebExtensionControllerDelegate>
@property (strong) Window *window;
@property (strong) NSPopover *popover;
@property (strong) WKWebView *popup;
@end
@implementation Delegate
- (NSArray<id<WKWebExtensionWindow>> *)webExtensionController:(WKWebExtensionController *)c openWindowsForExtensionContext:(WKWebExtensionContext *)x { return self.window ? @[self.window] : @[]; }
- (id<WKWebExtensionWindow>)webExtensionController:(WKWebExtensionController *)c focusedWindowForExtensionContext:(WKWebExtensionContext *)x { return self.window; }
- (void)webExtensionController:(WKWebExtensionController *)c promptForPermissions:(NSSet<WKWebExtensionPermission> *)permissions inTab:(id<WKWebExtensionTab>)tab forExtensionContext:(WKWebExtensionContext *)x completionHandler:(void (^)(NSSet<WKWebExtensionPermission> *, NSDate *))done {
    printf("  prompt for permissions: %s\n", permissions.description.UTF8String); done(permissions, nil);
}
- (void)webExtensionController:(WKWebExtensionController *)c promptForPermissionMatchPatterns:(NSSet<WKWebExtensionMatchPattern *> *)patterns inTab:(id<WKWebExtensionTab>)tab forExtensionContext:(WKWebExtensionContext *)x completionHandler:(void (^)(NSSet<WKWebExtensionMatchPattern *> *, NSDate *))done {
    printf("  prompt for patterns: %lu\n", (unsigned long)patterns.count); done(patterns, nil);
}
- (void)webExtensionController:(WKWebExtensionController *)c presentPopupForAction:(WKWebExtensionAction *)action forExtensionContext:(WKWebExtensionContext *)x completionHandler:(void (^)(NSError *))done {
    printf("  present popup: %s\n", action.popupWebView.URL.absoluteString.UTF8String ?: "(no url yet)");
    self.popup = action.popupWebView;
    NSViewController *controller = [NSViewController new];
    controller.view = action.popupWebView;
    action.popupWebView.frame = NSMakeRect(0, 0, 380, 560);
    self.popover = [NSPopover new];
    self.popover.contentViewController = controller;
    self.popover.contentSize = NSMakeSize(380, 560);
    [self.popover showRelativeToRect:NSMakeRect(860, 660, 10, 10) ofView:self.window.native.contentView preferredEdge:NSRectEdgeMinY];
    done(nil);
}
- (void)webExtensionController:(WKWebExtensionController *)c sendMessage:(id)message toApplicationWithIdentifier:(NSString *)app forExtensionContext:(WKWebExtensionContext *)x replyHandler:(void (^)(id, NSError *))reply {
    printf("  native message to %s: %s\n", app.UTF8String ?: "(default)", [message description].UTF8String); reply(nil, [NSError errorWithDomain:@"harness" code:1 userInfo:@{NSLocalizedDescriptionKey: @"no native host"}]);
}
- (void)webExtensionController:(WKWebExtensionController *)c connectUsingMessagePort:(WKWebExtensionMessagePort *)port forExtensionContext:(WKWebExtensionContext *)x completionHandler:(void (^)(NSError *))done {
    printf("  native port to %s\n", port.applicationIdentifier.UTF8String ?: "(default)"); done([NSError errorWithDomain:@"harness" code:1 userInfo:@{NSLocalizedDescriptionKey: @"no native host"}]);
}
@end

static void grantAll(WKWebExtensionContext *context) {
    for (WKWebExtensionPermission permission in context.webExtension.requestedPermissions)
        [context setPermissionStatus:WKWebExtensionContextPermissionStatusGrantedExplicitly forPermission:permission];
    for (WKWebExtensionMatchPattern *pattern in context.webExtension.requestedPermissionMatchPatterns)
        [context setPermissionStatus:WKWebExtensionContextPermissionStatusGrantedExplicitly forMatchPattern:pattern];
    [context setPermissionStatus:WKWebExtensionContextPermissionStatusGrantedExplicitly forMatchPattern:[WKWebExtensionMatchPattern allURLsMatchPattern]];
}

int main(int argc, const char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    @autoreleasepool {
        if (argc < 4) { fprintf(stderr, "usage: harness URL SECONDS EXTDIR...\n"); return 2; }
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        NSURL *page = [NSURL URLWithString:@(argv[1])];
        double seconds = atof(argv[2]);
        // Extension pages and workers identify as Safari, like Browser's pages.
        NSString *safari = @"Version/26.6 Safari/605.1.15";
        WKWebExtensionControllerConfiguration *controllerConfiguration = [WKWebExtensionControllerConfiguration nonPersistentConfiguration];
        WKWebViewConfiguration *extensionPages = [WKWebViewConfiguration new];
        extensionPages.applicationNameForUserAgent = safari;
        controllerConfiguration.webViewConfiguration = extensionPages;
        WKWebExtensionController *controller = [[WKWebExtensionController alloc] initWithConfiguration:controllerConfiguration];
        Delegate *delegate = [Delegate new];
        controller.delegate = delegate;
        BOOL action = getenv("HARNESS_ACTION") != NULL;
        NSMutableArray<WKWebExtensionContext *> *contexts = [NSMutableArray array];
        __block int pending = argc - 3;
        for (int i = 3; i < argc; i++) {
            NSURL *dir = [NSURL fileURLWithPath:@(argv[i]) isDirectory:YES];
            void (^loaded)(WKWebExtension *, NSError *) = ^(WKWebExtension *extension, NSError *error) {
                if (!extension) { printf("LOAD FAILED %s: %s\n", argv[i], error.localizedDescription.UTF8String); }
                else {
                    printf("extension: %s %s (MV%g)\n", extension.displayName.UTF8String, extension.version.UTF8String, extension.manifestVersion);
                    for (NSError *e in extension.errors) printf("  manifest issue: %s\n", e.localizedDescription.UTF8String);
                    WKWebExtensionContext *context = [WKWebExtensionContext contextForExtension:extension];
                    context.inspectable = YES;
                    grantAll(context);
                    NSError *loadError = nil;
                    if (![controller loadExtensionContext:context error:&loadError])
                        printf("  context load failed: %s\n", loadError.localizedDescription.UTF8String);
                    if (getenv("HARNESS_REGRANT")) grantAll(context);
                    [contexts addObject:context];
                }
                pending--;
            };
            if ([dir.pathExtension isEqualToString:@"appex"])
                [WKWebExtension extensionWithAppExtensionBundle:[NSBundle bundleWithURL:dir] completionHandler:loaded];
            else
                [WKWebExtension extensionWithResourceBaseURL:dir completionHandler:loaded];
        }
        while (pending > 0) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
        configuration.webExtensionController = controller;
        configuration.applicationNameForUserAgent = safari;
        if (getenv("HARNESS_STORE")) configuration.websiteDataStore = [WKWebsiteDataStore dataStoreForIdentifier:[NSUUID UUID]];
        NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(200, 200, 900, 700) styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
        WKWebView *view = [[WKWebView alloc] initWithFrame:window.contentView.bounds configuration:configuration];
        view.inspectable = YES;
        window.contentView = view;
        Window *model = [Window new];
        model.native = window;
        Tab *tab = [Tab new];
        tab.view = view;
        tab.window = model;
        model.tabs = [NSMutableArray arrayWithObject:tab];
        if (!getenv("HARNESS_NOTABS")) {
            delegate.window = model;
            [controller didOpenWindow:model];
            [controller didFocusWindow:model];
            [controller didOpenTab:tab];
            [controller didActivateTab:tab previousActiveTab:nil];
        }
        [window makeKeyAndOrderFront:nil];
        [app activateIgnoringOtherApps:YES];
        const char *warm = getenv("HARNESS_WARMUP");
        if (warm) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:atof(warm)]];
        NSDate *start = [NSDate date];
        [view loadRequest:[NSURLRequest requestWithURL:page]];
        while (view.loading) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
        printf("page load: %.0f ms\n", -start.timeIntervalSinceNow * 1000);
        [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:seconds]];
        if (action) {
            for (WKWebExtensionContext *context in contexts) {
                WKWebExtensionAction *a = [context actionForTab:tab];
                printf("action %s: label=%s popup=%d enabled=%d\n", context.webExtension.displayName.UTF8String, a.label.UTF8String ?: "", a.presentsPopup, a.enabled);
                if (a.presentsPopup) {
                    [context performActionForTab:tab];
                    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:seconds]];
                    WKWebView *pop = a.popupWebView;
                    printf("popup view: %s url=%s loading=%d title=%s\n", pop ? "yes" : "no", pop.URL.absoluteString.UTF8String ?: "", pop.loading, pop.title.UTF8String ?: "");
                    if (pop && !delegate.popup) delegate.popup = pop;
                    break;
                }
            }
            if (delegate.popup) {
                __block BOOL popDone = NO;
                [delegate.popup evaluateJavaScript:@"JSON.stringify({url: location.href, title: document.title, text: document.body ? document.body.innerText.slice(0, 300) : ''})" completionHandler:^(id result, NSError *error) {
                    printf("popup: %s\n", error ? error.localizedDescription.UTF8String : [result description].UTF8String); popDone = YES; }];
                while (!popDone) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
                const char *shot = getenv("HARNESS_SHOT");
                if (shot) {
                    __block BOOL snapDone = NO;
                    [delegate.popup takeSnapshotWithConfiguration:nil completionHandler:^(NSImage *image, NSError *error) {
                        if (image) { NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:image.TIFFRepresentation]; [[rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:@(shot) atomically:YES]; }
                        snapDone = YES; }];
                    while (!snapDone) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
                }
            }
        }
        __block BOOL done = NO;
        NSString *probe = @"JSON.stringify({probe: document.documentElement.dataset.probe || null, background: document.documentElement.dataset.background || null, ad: (document.getElementById('status')||{dataset:{}}).dataset.ad || null, adLoaded: !!window.adLoaded, title: document.title, bodyText: document.body ? document.body.innerText.slice(0, 200) : ''})";
        [view evaluateJavaScript:probe completionHandler:^(id result, NSError *error) {
            printf("page: %s\n", error ? error.localizedDescription.UTF8String : [result description].UTF8String);
            done = YES;
        }];
        while (!done) [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        for (WKWebExtensionContext *context in contexts) {
            NSURL *ad = [NSURL URLWithString:@"https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js"];
            printf("access: page=%d (tab %d, status %ld) ad=%d (tab %d, status %ld) patterns=%lu requestedOptionalAll=%d\n",
                [context hasAccessToURL:page], [context hasAccessToURL:page inTab:tab], (long)[context permissionStatusForURL:page inTab:tab],
                [context hasAccessToURL:ad], [context hasAccessToURL:ad inTab:tab], (long)[context permissionStatusForURL:ad inTab:tab],
                (unsigned long)context.currentPermissionMatchPatterns.count, context.hasRequestedOptionalAccessToAllHosts);
            printf("context %s: loaded=%d allURLs=%d injected=%d rules=%d\n", context.webExtension.displayName.UTF8String, context.loaded, context.hasAccessToAllURLs, context.hasInjectedContent, context.hasContentModificationRules);
            for (NSError *e in context.errors) printf("  error: %s %s\n", e.localizedDescription.UTF8String, e.userInfo.description.UTF8String);
        }
    }
    return 0;
}
