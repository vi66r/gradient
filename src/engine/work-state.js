// Runs only in Browser's isolated WKContentWorld. No IPC handler, network,
// timers, or field values leave this realm. Native code asks for a small report.
(() => {
  const token = globalThis.crypto?.randomUUID?.() || (Date.now().toString(36) + Math.random().toString(36));
  let revision = 0;
  const editableBaseline = new WeakMap();
  const touchedEditors = new Set();
  const observed = new WeakMap();
  let unknownEdit = false;
  const editor = target => target instanceof Element
    ? (target.isContentEditable ? target.closest('[contenteditable]') : null) : null;
  addEventListener('beforeinput', event => {
    const node = editor(event.composedPath()[0]);
    if (node && !editableBaseline.has(node)) {
      if (node.innerHTML.length > 65536 || touchedEditors.size >= 256) unknownEdit = true;
      else editableBaseline.set(node, node.innerHTML);
    }
  }, true);
  addEventListener('input', event => {
    revision++;
    const target = event.composedPath()[0];
    const node = editor(target);
    if (node) {
      if (touchedEditors.size < 256) touchedEditors.add(node);
      else unknownEdit = true;
      if (!editableBaseline.has(node)) unknownEdit = true;
    } else if (!(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
                 || target instanceof HTMLSelectElement)) unknownEdit = true;
  }, true);
  addEventListener('change', () => revision++, true);
  addEventListener('reset', () => revision++, true);
  // A page can deliberately change DOM defaults after saving. Standard controls
  // are compared with their current defaults, so reset/revert/save can clear risk.
  function local() {
    let risk = unknownEdit ? 3 : 0;
    for (const node of touchedEditors) {
      if (!node.isConnected) risk = Math.max(risk, 3);
      else if (node.innerHTML !== editableBaseline.get(node)) risk = Math.max(risk, 1);
    }
    let visited = 0;
    function scan(root) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (++visited > 10000) { risk = 3; return; }
        // Detect programmatic edits between probes too. Values stay in the
        // isolated realm; native code receives only a changing revision number.
        let value;
        if (node instanceof HTMLInputElement) value = node.type === 'file'
          ? [...node.files].map(file => file.name + ':' + file.size).join('|')
          : node.value + ':' + node.checked;
        else if (node instanceof HTMLTextAreaElement) value = node.value;
        else if (node instanceof HTMLSelectElement) value = [...node.options].map(option => option.selected).join(',');
        else if (touchedEditors.has(node)) value = node.innerHTML;
        if (value !== undefined) {
          if (value.length > 65536) risk = 3;
          else {
            if (observed.has(node) && observed.get(node) !== value) revision++;
            observed.set(node, value);
          }
        }
        if (node instanceof HTMLTextAreaElement && node.value !== node.defaultValue) risk = Math.max(risk, 1);
        if (node instanceof HTMLInputElement) {
          if (['checkbox', 'radio'].includes(node.type)) {
            if (node.checked !== node.defaultChecked) risk = Math.max(risk, 1);
          } else if (node.type === 'file') {
            if (node.files.length) risk = Math.max(risk, 1);
          } else if (!['hidden', 'submit', 'button', 'reset', 'image'].includes(node.type)) {
            const baseline = node.cloneNode(false);
            baseline.value = node.defaultValue;
            if (node.value !== baseline.value) risk = Math.max(risk, 1);
          }
        }
        if (node instanceof HTMLSelectElement) {
          const options = [...node.options];
          const defaultIndex = options.reduce((last, option, index) => option.defaultSelected ? index : last, -1);
          const fallbackIndex = options.findIndex(option => !option.disabled && !option.closest('optgroup')?.disabled);
          if (options.some((option, index) => option.selected !== (node.multiple
            ? option.defaultSelected : index === (defaultIndex < 0 ? fallbackIndex : defaultIndex)))) risk = Math.max(risk, 1);
        }
        if (node instanceof HTMLMediaElement && !node.paused && !node.ended) risk = Math.max(risk, 2);
        if (['CANVAS', 'OBJECT', 'EMBED'].includes(node.tagName)) risk = 3;
        if (node.shadowRoot) scan(node.shadowRoot);
      }
    }
    scan(document);
    return {risk, stamp: token + ':' + revision};
  }
  Object.defineProperty(globalThis, '__browserWorkState', {value: (depth = 0) => {
    const result = local();
    if (depth >= 8 || frames.length > 32) return {...result, risk: 3};
    for (let i = 0; i < frames.length; i++) {
      try {
        const child = frames[i].__browserWorkState(depth + 1);
        result.risk = Math.max(result.risk, child.risk);
        result.stamp += '|' + child.stamp;
        if (result.stamp.length > 15000) return {risk: 3, stamp: token + ':' + revision};
      } catch { result.risk = 3; }
    }
    return result;
  }});
})();
