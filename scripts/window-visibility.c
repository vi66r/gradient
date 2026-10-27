// macOS-only diagnostic helper; never linked into Gradient.
// Prints the fraction of PID's largest on-screen window that is not covered by
// windows above it. GPUI parks its display link for occluded windows, so idle
// measurements are only meaningful while the window is actually visible.
// Window bounds and ownership need no Screen Recording permission.
#include <CoreFoundation/CoreFoundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <stdio.h>
#include <stdlib.h>

static int number(CFDictionaryRef d, CFStringRef key, double *out) {
    CFNumberRef n = CFDictionaryGetValue(d, key);
    return n && CFNumberGetValue(n, kCFNumberDoubleType, out);
}

static int bounds(CFDictionaryRef d, CGRect *out) {
    CFDictionaryRef b = CFDictionaryGetValue(d, kCGWindowBounds);
    return b && CGRectMakeWithDictionaryRepresentation(b, out);
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    double pid = atoi(argv[1]);
    // Front-to-back order of windows on the current Space.
    CFArrayRef list = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
    if (!list) return 3;
    CFIndex count = CFArrayGetCount(list), target = -1;
    CGRect window = CGRectNull;
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef d = CFArrayGetValueAtIndex(list, i);
        double owner, layer;
        CGRect r;
        if (number(d, kCGWindowOwnerPID, &owner) && owner == pid &&
            number(d, kCGWindowLayer, &layer) && layer == 0 && bounds(d, &r) &&
            (target < 0 || r.size.width * r.size.height > window.size.width * window.size.height)) {
            target = i;
            window = r;
        }
    }
    int visible = 0, samples = 0;
    if (target >= 0) {
        const int grid = 48;
        for (int y = 0; y < grid; y++) {
            for (int x = 0; x < grid; x++) {
                CGPoint p = CGPointMake(window.origin.x + (x + 0.5) * window.size.width / grid,
                                        window.origin.y + (y + 0.5) * window.size.height / grid);
                int covered = 0;
                for (CFIndex i = 0; i < target && !covered; i++) {
                    CFDictionaryRef d = CFArrayGetValueAtIndex(list, i);
                    double alpha = 1, layer = 0;
                    CGRect r;
                    number(d, kCGWindowAlpha, &alpha);
                    number(d, kCGWindowLayer, &layer);
                    // App and floating windows only: the Dock, menu bar and
                    // system overlays sit at higher levels and span the screen.
                    covered = layer >= 0 && layer < kCGDockWindowLevel && alpha > 0 &&
                              bounds(d, &r) && CGRectContainsPoint(r, p);
                }
                visible += !covered;
                samples++;
            }
        }
    }
    printf("{\"onscreen\":%s,\"visible_fraction\":%.3f}\n", target >= 0 ? "true" : "false",
           samples ? (double)visible / samples : 0.0);
    CFRelease(list);
    return 0;
}
