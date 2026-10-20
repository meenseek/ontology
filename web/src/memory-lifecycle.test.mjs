import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { JSDOM, VirtualConsole } from 'jsdom';

// Reuse the browser scenario, including strict synthetic API boundaries, so CI
// checks the same transitions. jsdom supplies DOM behavior, not layout or WebGL.
test('App keeps correction state and newly created groups across the panel lifecycle', { timeout: 30000 }, async () => {
  const html = await readFile(new URL('../memory-grouping-check.html', import.meta.url), 'utf8');
  const scripts = [...html.matchAll(/<script type="module">([\s\S]*?)<\/script>/g)];
  assert.equal(scripts.length, 1, 'one explicit local fixture module');
  const runtimeErrors = [];
  const virtualConsole = new VirtualConsole();
  virtualConsole.on('jsdomError', error => runtimeErrors.push(error.message));
  const dom = new JSDOM(html.replace(scripts[0][0], ''), { virtualConsole, url: 'http://localhost/memory-grouping-check.html?scope=personal', pretendToBeVisual: true });
  const { window } = dom;
  const saved = new Map();
  function bind(name, value) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  }
  for (const name of ['window', 'document', 'navigator', 'location', 'history', 'HTMLElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLSelectElement', 'Event', 'MessageEvent', 'DOMParser', 'Node', 'MutationObserver', 'getComputedStyle']) {
    bind(name, typeof window[name] === 'function' && name === 'getComputedStyle' ? window[name].bind(window) : window[name]);
  }
  bind('innerWidth', 1440); bind('innerHeight', 900);
  bind('requestAnimationFrame', window.requestAnimationFrame.bind(window));
  bind('cancelAnimationFrame', window.cancelAnimationFrame.bind(window));
  Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
  Object.defineProperty(window, 'innerHeight', { value: 900, configurable: true });
  window.matchMedia = query => ({ media: query, matches: false, addEventListener() {}, removeEventListener() {} });
  window.HTMLElement.prototype.scrollIntoView = () => {};
  window.HTMLElement.prototype.scrollTo = () => {};
  window.fetch = async input => {
    assert.equal(new URL(String(input), window.location.href).pathname, '/index.html', 'only bootstrap entry may bypass the synthetic API');
    return new Response('<script type="module" src="/assets/source-app.js"></script>', { headers: { 'content-type': 'text/html' } });
  };
  // The fixture replaces window.fetch/EventSource before mounting. Forward the
  // global names used by source modules to those exact same synthetic adapters.
  for (const name of ['fetch', 'EventSource']) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, get: () => window[name] });
  }
  let root;
  try {
    const { createElement } = await import('react');
    const { createRoot } = await import('react-dom/client');
    const { default: App } = await import('./App.tsx');
    window.ontologyFixtureMount = async () => {
      root = createRoot(window.document.querySelector('#root'));
      root.render(createElement(App));
    };
    // Execute only this repository-owned module; jsdom never enables script
    // tags, resource loading, or a live HTTP API.
    const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
    await new AsyncFunction(scripts[0][1])();
    const result = window.document.querySelector('#result');
    assert.match(result.textContent, /^PASS \d+ checks\n/, result.textContent);
    for (const proof of ['reselect created group in same panel', 'partial creation keeps correction', 'retried created group remains selectable', 'reselect suggested group in same panel', 'form-created group remains in options', 'suggested partial creation keeps correction', 'external content conflict', 'explicit discard guard']) {
      assert.ok(result.textContent.includes(proof), `missing transition: ${proof}`);
    }
    window.document.querySelector('#finish').click();
    const deadline = Date.now() + 5000;
    while (!result.textContent.includes('CLEAN:')) {
      assert.ok(Date.now() < deadline, 'fixture cleanup completed');
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    assert.equal(window.document.querySelector('.management-panel'), null);
    assert.deepEqual(runtimeErrors, [], 'no uncaught DOM runtime errors');
    console.log(result.textContent.split('\n')[0]);
  } finally {
    root?.unmount();
    await new Promise(resolve => setTimeout(resolve, 30));
    window.close();
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name];
    }
  }
});
