const out = document.getElementById('out');
chrome.tabs.query({ active: true, currentWindow: true }).then(([tab]) => {
  document.getElementById('tab').textContent = tab ? `Tab ${tab.id}: ${tab.title} (${tab.url})` : 'No active tab';
});
document.getElementById('open').onclick = () => chrome.tabs.create({ url: 'http://127.0.0.1:8773/?opened-by-probe' }).then(
  (tab) => { out.textContent = 'Opened tab ' + tab.id; },
  (error) => { out.textContent = 'error: ' + error.message; });
document.getElementById('ask').onclick = () => chrome.permissions.request({ permissions: ['clipboardWrite'] }).then(
  (granted) => { out.textContent = 'granted: ' + granted; },
  (error) => { out.textContent = 'error: ' + error.message; });
