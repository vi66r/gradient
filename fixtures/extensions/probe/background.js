// The badge counts this window's tabs, so tab events show on the toolbar.
const count = async () => {
  const tabs = await chrome.tabs.query({ currentWindow: true });
  await chrome.action.setBadgeText({ text: String(tabs.length) });
};
chrome.tabs.onCreated.addListener(count);
chrome.tabs.onRemoved.addListener(count);
chrome.tabs.onActivated.addListener(count);
count();
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  chrome.tabs.query({ active: true, currentWindow: true }).then(([active]) => {
    reply({
      pong: message.ping,
      tab: sender.tab ? sender.tab.id : null,
      url: sender.tab ? sender.tab.url : null,
      active: active ? active.id : null,
    });
  });
  return true;
});
