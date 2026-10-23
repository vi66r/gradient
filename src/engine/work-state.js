// Runs only in Browser's isolated WKContentWorld. No IPC handler, network,
// timers, or field values leave this realm. Native code asks for a small report.
//
// Only work the user produced counts. Pages routinely set field values,
// draw canvases and embed cross-origin frames on their own; treating those as
// unsaved work made settled pages (a search results page) prompt on close.
// A control counts once it receives a trusted input/change event; opaque
// content (canvas, plugins, cross-origin frames) counts once the user clicks
// or types into it. Search fields never count.
(() => {
  const token = globalThis.crypto?.randomUUID?.() || (Date.now().toString(36) + Math.random().toString(36));
  let revision = 0;
  const editableBaseline = new WeakMap();
  const touchedEditors = new Set();
  const engaged = new WeakSet();
  const engagedFrames = new WeakSet();
  const observed = new WeakMap();
  let unknownEdit = false;
  let userEdited = false;
  let opaqueEngaged = false;
  const editor = target => target instanceof Element
    ? (target.isContentEditable ? target.closest('[contenteditable]') : null) : null;
  const opaque = node => node instanceof Element && ['CANVAS', 'OBJECT', 'EMBED'].includes(node.tagName);
  const search = node => node instanceof HTMLInputElement && (node.type === 'search'
    || node.getAttribute('role') === 'searchbox' || node.enterKeyHint === 'search'
    || !!node.closest('[role=search], search')
    || /^(q|query|search|search_query|s)$/i.test(node.name || node.id || ''));
  addEventListener('beforeinput', event => {
    if (!event.isTrusted) return;
    const node = editor(event.composedPath()[0]);
    if (node && !editableBaseline.has(node)) {
      if (node.innerHTML.length > 65536 || touchedEditors.size >= 256) unknownEdit = true;
      else editableBaseline.set(node, node.innerHTML);
    }
  }, true);
  addEventListener('input', event => {
    // Any edit, scripted or not, invalidates a pending consent.
    revision++;
    if (!event.isTrusted) return;
    userEdited = true;
    const target = event.composedPath()[0];
    const node = editor(target);
    if (node) {
      if (touchedEditors.size < 256) touchedEditors.add(node);
      else unknownEdit = true;
      if (!editableBaseline.has(node)) unknownEdit = true;
    } else if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
               || target instanceof HTMLSelectElement) engaged.add(target);
    else unknownEdit = true;
  }, true);
  addEventListener('change', event => {
    revision++;
    if (!event.isTrusted) return;
    userEdited = true;
    const target = event.composedPath()[0];
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
        || target instanceof HTMLSelectElement) engaged.add(target);
  }, true);
  addEventListener('reset', () => revision++, true);
  for (const type of ['pointerdown', 'keydown']) {
    addEventListener(type, event => {
      if (event.isTrusted && event.composedPath().some(opaque)) opaqueEngaged = true;
    }, true);
  }
  // Focus moving into a child frame blurs this window with the frame active.
  addEventListener('blur', event => {
    const active = document.activeElement;
    if (event.isTrusted && event.target === window && (active instanceof HTMLIFrameElement
        || active?.tagName === 'FRAME')) engagedFrames.add(active);
  }, true);
  const frameEngaged = frame => engagedFrames.has(frame) || document.activeElement === frame;
  // A page can deliberately change DOM defaults after saving. Standard controls
  // are compared with their current defaults, so reset/revert/save can clear risk.
  function local() {
    if (!userEdited && !opaqueEngaged) return {risk: 0, stamp: token + ':' + revision};
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
        if (engaged.has(node)) {
          if (node instanceof HTMLTextAreaElement && node.value !== node.defaultValue) risk = Math.max(risk, 1);
          if (node instanceof HTMLInputElement && !search(node)) {
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
        }
        if (opaqueEngaged && opaque(node)) risk = 3;
        if (node.shadowRoot) scan(node.shadowRoot);
      }
    }
    scan(document);
    return {risk, stamp: token + ':' + revision};
  }
  Object.defineProperty(globalThis, '__browserWorkState', {value: (depth = 0) => {
    const result = local();
    if (depth >= 8) return {...result, risk: 3};
    const elements = [...document.querySelectorAll('iframe, frame')];
    for (let i = 0; i < Math.min(frames.length, 32); i++) {
      try {
        const child = frames[i].__browserWorkState(depth + 1);
        result.risk = Math.max(result.risk, child.risk);
        result.stamp += '|' + child.stamp;
        if (result.stamp.length > 15000) return {risk: 3, stamp: token + ':' + revision};
      } catch {
        // Cross-origin or not yet loaded: unknowable, so only a frame the
        // user has worked in is treated as possibly holding unsaved work.
        const element = elements.find(element => element.contentWindow === frames[i]);
        if (element && frameEngaged(element)) result.risk = 3;
      }
    }
    return result;
  }});
})();
