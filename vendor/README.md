# Vendored crates

## gpui-pre-macos 0.3.9

GPUI's macOS window backend, from crates.io (Zed `gpui_macos` at `93d7456`, Apache-2.0). It is wired in with `[patch.crates-io]` in the root `Cargo.toml`. Commit `079078e` holds the unmodified copy, so `git diff 079078e -- vendor/` is the whole patch.

**Why:** upstream keeps the window's CVDisplayLink running whenever the window is visible. GPUI then throws away every clean frame, so an idle browser window woke the process on each vsync. On a 120 Hz ProMotion panel that was about 145 interrupt wakeups/sec (PERFORMANCE.md).

**Change:** the link now runs on demand. GPUI's core already supports platforms that stop requesting frames: it calls `PlatformWindow::frame_waker` when a window becomes dirty, schedules next-frame callbacks, or defers a throttled frame. Only the macOS backend lacked a waker.

- `step` parks the link after 100 ms with no presented frame and no frame request.
- `frame_waker` restarts a parked link on a fresh main-queue turn and requests one frame immediately.
- Presenting counts as activity. That includes GPUI's 1 s of sustained presentation after high-rate input, so animation and ProMotion input pacing still get every vsync.
- Occlusion handling is unchanged. "Parked" (idle) and "stopped" (occluded) stay distinct.
- `#![allow(deprecated)]` at the crate root restores the lint capping that crates.io builds get.

**Dropping it:** remove the `[patch.crates-io]` entry and this directory once upstream implements a macOS frame waker. This patch is a candidate to offer upstream.
