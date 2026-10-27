# Performance validation

This is a controlled local regression suite for the native browser, not a claim of Safari parity or a representative browsing benchmark.

## Reproduce

Build the release bundle first; do not compile while collecting samples. Keep the same display, window size, power mode, and other applications for both versions. The fixture server binds loopback only.

```sh
python3 scripts/test-server.py --port 8766 --reports artifacts/performance/reports
# In another terminal:
python3 scripts/benchmark-macos.py dist/Browser.app --label candidate
```

The runner launches fresh instances through Launch Services with disposable profiles and terminates only its own instances. Close the bundle being tested before starting. The normal browsing profile is not used. WebKit manages website data separately from the disposable session directory.

Each of three trials contains a blank window, an idle static interaction fixture, an animated WebGPU cube, and a restored 20-tab session whose selected tab is the same static fixture. Only the selected restored tab should load. The script waits for page readiness, warms up for seven seconds, then samples at one-second intervals for approximately 20 seconds. Static pages use one reporting timer; they do not run a synthetic animation loop. WebGPU frame intervals are collected after five seconds of warmup for 20 seconds.

`perf-processes.c` attributes the app plus its XPC/Metal helpers using macOS's responsibility SPI. It refuses launches that inherit another process's responsibility. `proc_pid_rusage` supplies per-process physical footprint, resident memory, CPU time and interrupt wakeups. CPU Mach absolute-time ticks are converted with `mach_timebase_info`; 100% CPU means one logical core. Reported memory is the sum of OS physical-footprint accounting, not shell RSS. Raw per-process records remain available for inspection. Helpers that exit between samples can have unobserved work; this is a steady-state suite, not a complete launch-energy accounting tool.

`window-visibility.c` samples the app's main window every second. Each run aborts unless at least 90% of the window stays uncovered on the current Space. An occluded window stops GPUI's display link, so a hidden window understates idle wakeups. Window bounds and ownership need no Screen Recording permission. The runner also reports the app process's own wakeups separately from its WebKit helpers.

The SPI is confined to a developer diagnostic executable and is not shipped or linked into Browser. Its availability and attribution are checked on each invocation. GPU page reports track hidden-page transitions and surface WebGPU errors. Static reports check visibility when the report is sent; blank windows have no page visibility guard. See the anomalous blank run below. The runner aborts on missing reports or inaccessible process samples.

## 2026-10-20 results: idle display-link parking

Apple M1 Max MacBook Pro on its built-in 120 Hz ProMotion display, DPR 2. Window 1240×820 points, page viewport 1016×757. Baseline is commit `6d70744` with upstream `gpui-pre-macos`. The candidate is the same app with the vendored backend from [vendor/README.md](vendor/README.md). Every sample of all 24 runs passed the visibility guard. Per-run data, per-process splits and raw-data hashes are in [benchmarks/2026-10-20.json](benchmarks/2026-10-20.json).

Three-trial medians, before → after:

| Workload | Total wakeups/s | App-process wakeups/s | Total CPU |
|---|---|---|---|
| Blank window | 121.1 → **1.1** | 121.0 → 1.0 | 1.11% → **0.04%** |
| Static page | 125.7 → **5.7** | 122.3 → 2.4 | 1.23% → **0.17%** |
| 20 restored tabs | 125.9 → **5.8** | 122.3 → 2.3 | 1.21% → **0.17%** |
| WebGPU cube | 299.6 → **178.1** | 272.7 → 151.7 | 22.32% → 23.16% |

All three trials of each workload agreed within a few tenths. The ~121 removed wakeups/sec match one GPUI display-link tick per 120 Hz vsync. The blank window's remaining 1/sec is the shell's one-second URL/session poll (`main.rs`). The static workloads' extra ~3/sec come from WebKit helpers and the fixture's reporting timer.

WebGPU frame pacing is unchanged: 60 FPS in all six runs, p95 19–20 ms → 19 ms, p99 22 → 21 ms, zero intervals over 50 ms. The cube page still paces at 60 FPS on this display. GPU-workload CPU did not fall, because the app process's share there (7.8–8.0% in both cohorts) is WebKit compositing inside the app. All three candidate GPU runs were 0.6–1.2 points higher in total. The difference is in WebKit's GPU process (7.2–7.4% → 7.8–8.0%), not Browser's own process. The 1-minute load average reached 14–16 during two candidate GPU runs. This difference is unexplained and is not counted as a gain or a regression.

Memory is unchanged. These footprints are higher than the 2026-10-15 cohort because that cohort ran on an external display at DPR 1. Do not compare memory across the two cohorts.

This also explains the 2026-10-15 `after-blank-3` anomaly, where shell wakeups fell to ~1/sec. That window was most likely occluded, which stops the display link. The visibility guard now rejects such runs.

A focused address bar keeps its caret blinking. In 10-second spot checks of the app process alone, the caret costs 30.3 wakeups/s and 1.01% CPU with the patch, against 123.8/s and 1.91% without. Each blink holds the link for one 100 ms park delay.

Not measured: battery or energy, input latency with the link parked and woken, and behavior on an external 60 Hz display.

## 2026-10-15 results

Apple M1 Max MacBook Pro, 64 GiB RAM, macOS 26.6.2; 1240×820-point window, 1016×756 page viewport at DPR 1. AC power was confirmed at the end. Baseline is commit `89ae83f`; candidate is the release built from this change. The benchmark copies use separate bundle identifiers for process attribution. All 24 primary runs completed, plus one blank-window confirmation. Compact per-run data, helper footprints, load ranges, resize traces and raw-data hashes are committed in [benchmarks/2026-10-15.json](benchmarks/2026-10-15.json). Full one-second process samples and logs remain locally in `artifacts/performance/`.

Three-trial median physical footprint, before → after:

- Blank: **49.1 → 49.4 MiB**; revised primary-trial range 49.0–49.5 MiB.
- Static fixture: **87.4 → 87.2 MiB**; revised range 86.4–90.1 MiB.
- WebGPU cube: **105.3 → 105.1 MiB**; revised range 103.5–106.8 MiB. The first baseline run reached a 130.8 MiB median while extra Metal helpers were present.
- Twenty restored tabs: **95.2 → 98.3 MiB**; revised range 98.2–101.9 MiB. All six runs had exactly one WebContent process. Per-process records show helper-footprint variation, including the audio service; this is not evidence of a per-tab memory increase.

The WebGPU workload stayed at **72 FPS** in all six runs. Frame-interval p95 was **15 ms**, p99 **16–17 ms**, with **zero intervals over 50 ms**. Total CPU median was 19.42% before and 19.44% after. Static-page CPU medians were 1.35% and 1.14%; restored-session medians were 1.34% and 1.17%. These sequential cohorts ran on a busy workstation, so the small CPU differences are not attributed to the patch. There is no measured rendering-throughput improvement.

Foreground idle interrupt wakeups remained about **145/sec** for a blank window and **150/sec** for the static page. In `after-blank-3`, shell wakeups dropped from roughly 145/sec to 1/sec three seconds into sampling; possible occlusion was not directly recorded. Its 0.30% CPU result is retained and is not treated as a gain. An additional blank-window run was explicitly raised and inspected with accessibility: wakeups returned to 145.8/sec. Inspection also introduced a helper and overhead (58.2 MiB, 1.47% CPU), making that confirmation unsuitable for a like-for-like memory comparison.

The practical improvement in this pass is native resize coordination and copy feedback with no viewport reflow. The page scheduling measurements stayed stable; they cannot quantify the former delay between shell geometry and native host geometry.

## Coverage and limits

The cube is deliberately stable and small: it tests the embedding and frame pacing, not maximum GPU throughput or complex real websites. rAF intervals measure page scheduling, not physical display presentation or end-to-end input latency. Memory snapshots are not a leak proof. Interrupt wakeups are diagnostic, not energy in joules or battery life. Existing applications and system load remain running, so small differences should be treated as noise. No Safari comparison, Linux measurement, long-session leak test, or battery measurement is implied.

Native resize, Inspector layout, tab state, and copy feedback need separate interactive UI checks. A native view's geometry must stay synchronized with the shell even when the page's animation frame rate remains stable.

Implementation references: Apple's [resource accounting definitions](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/resource.h) and [task resource collection](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c). The helper uses the SDK's struct definitions rather than guessing the rusage layout.

## Live-tab lifecycle check

The revised release was also exercised with one static page containing an unsaved test draft and four live WebGPU pages. A 15-second observation with a GPU page selected used 21.21% total CPU and a median 282.6 MiB footprint. Selecting the static page, waiting five seconds, and observing for another 15 seconds reduced total CPU to 1.23%; the hidden GPU pages and GPU helper had approximately zero CPU during that interval. Footprint remained 278.1 MiB because pages were kept alive.

Closing the four GPU tabs, then waiting five seconds and sampling for 15 seconds, reduced footprint to 193.4 MiB and left one WebContent process. Compiler, Inspector-related and GPU caches remained warm, so this did not return to a fresh static-session baseline. The draft survived throughout. This is one sequential lifecycle observation of the revised app, not evidence that this patch introduced background throttling or a long-session memory guarantee.

A separate five-second idle `sample` trace showed an active CVDisplayLink thread. The pinned GPUI macOS backend started this source while the window was visible. The 2026-10-20 results above confirm it as the source of nearly all idle wakeups and remove it.
