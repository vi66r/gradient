#!/usr/bin/env python3
"""Repeat isolated release-app workloads. Run the reporting fixture server first.
Uses macOS responsibility attribution (diagnostic SPI) + proc_pid_rusage. No UI automation.
"""
import argparse, json, os, signal, statistics, subprocess, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def processes(binary):
    rows = subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True).splitlines()
    return [int(row.strip().split(None, 1)[0]) for row in rows if row.strip().split(None, 1)[-1] == str(binary)]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle', type=Path)
    parser.add_argument('--label', required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'artifacts/performance')
    parser.add_argument('--port', type=int, default=8766)
    parser.add_argument('--trials', type=int, default=3)
    parser.add_argument('--workloads', nargs='+', choices=['blank','static','gpu','restore20'], default=['blank','static','gpu','restore20'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    bundle = args.bundle.resolve()
    binary = bundle / 'Contents/MacOS/gradient'
    helper = args.output / 'perf-processes'
    subprocess.run(['clang','-O2','-Wall','-Wextra',str(ROOT/'scripts/perf-processes.c'),'-o',str(helper)],check=True)
    visibility = args.output / 'window-visibility'
    subprocess.run(['clang','-O2','-Wall','-Wextra',str(ROOT/'scripts/window-visibility.c'),'-framework','CoreGraphics','-framework','CoreFoundation','-o',str(visibility)],check=True)
    if processes(binary): raise SystemExit('Close this benchmark bundle before running it.')
    results = []
    for trial in range(1,args.trials+1):
        for workload in args.workloads:
            run = f'{args.label}-{workload}-{trial}'
            for suffix in ['', '-ready']:
                (args.output/'reports'/f'{run}{suffix}.json').unlink(missing_ok=True)
            with tempfile.TemporaryDirectory(prefix='browser-perf-') as profile:
                url = f'http://127.0.0.1:{args.port}/' + ('gpu' if workload=='gpu' else '') + f'?run={run}'
                if workload == 'restore20':
                    Path(profile,'session.json').write_text(json.dumps({'version':1,'active':0,'tabs':[
                        {'title':f'Fixture {i}', 'url':url if i==0 else f'http://127.0.0.1:{args.port}/?tab={i}'} for i in range(20)]}))
                cmd = ['open','-n',str(bundle),'--env',f'GRADIENT_DATA_DIR={profile}', '--stdout',str(args.output/f'{run}.log'), '--stderr',str(args.output/f'{run}.log')]
                if workload not in ['blank','restore20']: cmd += ['--args',url]
                launched = time.monotonic()
                subprocess.run(cmd,check=True)
                pid = None
                try:
                    while time.monotonic()-launched < 10:
                        found = processes(binary)
                        if len(found)==1: pid=found[0]; break
                        time.sleep(.1)
                    if not pid: raise RuntimeError('App failed to launch')
                    if workload != 'blank':
                        ready=args.output/'reports'/f'{run}-ready.json'
                        while not ready.exists() and time.monotonic()-launched < 20:
                            time.sleep(.1)
                        if not ready.exists(): raise RuntimeError('Fixture never became ready')
                    time.sleep(7) # page/Metal warmup; no compilation during samples
                    samples=[]
                    for _ in range(21):
                        rows=json.loads(subprocess.check_output([str(helper),str(pid)],text=True))
                        if not rows: raise RuntimeError('No attributed processes')
                        window=json.loads(subprocess.check_output([str(visibility),str(pid)],text=True))
                        samples.append({'time':time.monotonic(),'processes':rows,'window':window,'loadavg':os.getloadavg()})
                        time.sleep(1)
                    bypid={}
                    cpu_ns=0; wakeups=0; app_wakeups=0
                    for sample in samples:
                        for row in sample['processes']:
                            old=bypid.get(row['pid'])
                            if old:
                                cpu_ns+=max(0,row['cpu_ns']-old['cpu_ns'])
                                wakeups+=max(0,row['wakeups']-old['wakeups'])
                                if row['pid']==pid: app_wakeups+=max(0,row['wakeups']-old['wakeups'])
                            bypid[row['pid']]=row
                    # Occluded windows stop GPUI's display link; such samples measure nothing useful.
                    visible=min(s['window']['visible_fraction'] for s in samples)
                    if visible < 0.9:
                        raise RuntimeError(f'Window was not fully visible while sampling (min visible fraction {visible:.2f}); keep it uncovered on the current Space.')
                    elapsed=samples[-1]['time']-samples[0]['time']
                    memory=[sum(p['footprint'] for p in s['processes'])/2**20 for s in samples]
                    report=args.output/'reports'/f'{run}.json'
                    deadline=time.monotonic()+10
                    while workload!='blank' and not report.exists() and time.monotonic()<deadline:
                        time.sleep(.1)
                    frames=json.loads(report.read_text()) if report.exists() else None
                    if workload!='blank' and (not frames or frames['hidden'] or (workload=='gpu' and frames['probe'].get('error'))):
                        raise RuntimeError(f'Missing/invalid/hidden frame report: {frames}')
                    result={'run':run,'workload':workload,'trial':trial,'pid':pid,
                            'cpu_percent':cpu_ns/1e9/elapsed*100,'wakeups_per_second':wakeups/elapsed,
                            'app_wakeups_per_second':app_wakeups/elapsed,'min_visible_fraction':visible,
                            'footprint_mib_median':statistics.median(memory),'footprint_mib_peak':max(memory),
                            'process_count':len(samples[-1]['processes']), 'frames':frames,'samples':samples}
                    results.append(result)
                    (args.output/f'{args.label}.json').write_text(json.dumps(results,indent=2))
                    print(json.dumps({k:v for k,v in result.items() if k not in ['samples','frames']}),flush=True)
                finally:
                    if pid:
                        try: os.kill(pid,signal.SIGTERM)
                        except ProcessLookupError: pass
                        for _ in range(50):
                            if not processes(binary): break
                            time.sleep(.1)
                    time.sleep(2)

if __name__=='__main__': main()
