document.documentElement.dataset.probe = 'content-ran';
chrome.runtime.sendMessage({ ping: 'hello' }).then(
  (answer) => { document.documentElement.dataset.background = JSON.stringify(answer); },
  (error) => { document.documentElement.dataset.background = 'error: ' + error.message; });
