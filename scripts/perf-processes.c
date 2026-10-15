// macOS-only diagnostic helper; never linked into Browser.
// Responsibility SPI identifies XPC helpers whose parent is launchd, not Browser.
#include <libproc.h>
#include <sys/resource.h>
#include <stdio.h>
#include <stdlib.h>
#include <dlfcn.h>
#include <mach/mach_time.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int root = atoi(argv[1]);
    // rusage CPU totals are Mach absolute-time ticks, not nanoseconds on ARM.
    mach_timebase_info_data_t timebase;
    mach_timebase_info(&timebase);
    int (*responsible)(int) = dlsym(RTLD_DEFAULT, "responsibility_get_pid_responsible_for_pid");
    if (root <= 0 || !responsible || responsible(root) != root) {
        fprintf(stderr, "Launch the bundle with open; responsibility root must equal app PID.\n");
        return 3;
    }
    int count = proc_listallpids(NULL, 0) + 256;
    int *pids = calloc(count, sizeof(int));
    if (!pids) return 5;
    count = proc_listallpids(pids, count * sizeof(int));
    printf("[");
    int comma = 0;
    for (int i = 0; i < count; i++) {
        int pid = pids[i];
        if (pid != root && responsible(pid) != root) continue;
        struct rusage_info_v4 r = {0};
        char name[1024] = {0};
        if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&r)) {
            fprintf(stderr, "Cannot sample responsible PID %d\n", pid);
            free(pids); return 4;
        }
        proc_name(pid, name, sizeof(name));
        // OS process names in this controlled app contain no JSON metacharacters.
        printf("%s{\"pid\":%d,\"name\":\"%s\",\"cpu_ns\":%llu,\"footprint\":%llu,\"rss\":%llu,\"wakeups\":%llu}",
               comma++ ? "," : "", pid, name,
               (unsigned long long)((r.ri_user_time + r.ri_system_time) * (long double)timebase.numer / timebase.denom), r.ri_phys_footprint,
               r.ri_resident_size, r.ri_interrupt_wkups);
    }
    puts("]"); free(pids); return 0;
}
