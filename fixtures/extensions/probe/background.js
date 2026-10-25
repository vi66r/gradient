chrome.runtime.onMessage.addListener((message, sender, reply) => {
  reply({ pong: message.ping, tab: sender.tab ? sender.tab.id : null });
});
