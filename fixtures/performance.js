// Loaded only by the local performance fixture server. Opt in via ?run=<id>.
(() => {
  const query = new URLSearchParams(location.search), run = query.get('run');
  if (!run) return;
  fetch('/performance-report', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({run:run+'-ready',loaded:performance.now()})});
  if (!location.pathname.startsWith('/gpu') && !query.has('frames')) {
    setTimeout(() => fetch('/performance-report', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({run,hidden:document.hidden?1:0,viewport:[innerWidth,innerHeight,devicePixelRatio]})}), 25000);
    return;
  }
  const warmup = Math.min(60000, Math.max(0, Number(query.get('warmup') ?? 5000)));
  const duration = Math.min(60000, Math.max(1000, Number(query.get('duration') ?? 20000))), intervals = [], sizes = [];
  let previous, start, resizes = 0, hidden = 0;
  addEventListener('resize', () => { resizes++; if(sizes.length < 1000) sizes.push([performance.now(),innerWidth,innerHeight]); });
  document.addEventListener('visibilitychange', () => { if (document.hidden) hidden++; });
  const ready = () => !location.pathname.startsWith('/gpu') || window.probe?.frames > 5 || window.probe?.error;
  function frame(time) {
    if (!start && ready()) start = time;
    if (start && time - start >= warmup && previous !== undefined) intervals.push(time - previous);
    previous = time;
    if (start && time - start >= warmup + duration) {
      const sorted = [...intervals].sort((a,b) => a-b);
      const percentile = q => sorted[Math.min(sorted.length-1, Math.floor(sorted.length*q))];
      const report = {run, path:location.pathname, userAgent:navigator.userAgent,
        viewport:[innerWidth,innerHeight,devicePixelRatio], samples:intervals.length,
        fps:1000/(intervals.reduce((a,b)=>a+b,0)/intervals.length),
        p50_ms:percentile(.5),p95_ms:percentile(.95),p99_ms:percentile(.99),
        over25ms:intervals.filter(x=>x>25).length,over50ms:intervals.filter(x=>x>50).length,
        resizes,sizes,hidden,probe:window.probe??null};
      fetch('/performance-report', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(report)});
      window.performanceReport=report;
    } else requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})();
