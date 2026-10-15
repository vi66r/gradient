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

The SPI is confined to a developer diagnostic executable and is not shipped or linked into Browser. Its availability and attribution are checked on each invocation. GPU page reports track hidden-page transitions and surface WebGPU errors. Static reports check visibility when the report is sent; blank windows have no page visibility guard. See the anomalous blank run below. The runner aborts on missing reports or inaccessible process samples.

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

Native resize, Inspector layout, tab state, and copy feedback need the separate UI checks recorded in VALIDATION.md. A native view's geometry must stay synchronized with the shell even when the page's animation frame rate remains stable.

Implementation references: Apple's [resource accounting definitions](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/resource.h) and [task resource collection](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c). The helper uses the SDK's struct definitions rather than guessing the rusage layout.

## Live-tab lifecycle check

The revised release was also exercised with one static page containing an unsaved test draft and four live WebGPU pages. A 15-second observation with a GPU page selected used 21.21% total CPU and a median 282.6 MiB footprint. Selecting the static page, waiting five seconds, and observing for another 15 seconds reduced total CPU to 1.23%; the hidden GPU pages and GPU helper had approximately zero CPU during that interval. Footprint remained 278.1 MiB because pages were kept alive.

Closing the four GPU tabs, then waiting five seconds and sampling for 15 seconds, reduced footprint to 193.4 MiB and left one WebContent process. Compiler, Inspector-related and GPU caches remained warm, so this did not return to a fresh static-session baseline. The draft survived throughout. This is one sequential lifecycle observation of the revised app, not evidence that this patch introduced background throttling or a long-session memory guarantee.

A separate five-second idle `sample` trace showed an active CVDisplayLink thread. The pinned GPUI macOS backend starts this source while the window is visible. Together with the measured idle interrupt wakeups, that identifies a concrete next profiling target; it does not attribute every wakeup to that source or quantify battery impact.
