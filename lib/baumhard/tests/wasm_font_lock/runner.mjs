// Runs the scenarios exported by probe.rs inside node's WebAssembly
// engine. Usage: node runner.mjs <path-to-probe.wasm>
//
// Each scenario gets a fresh instance, so the font system's lazy
// statics and the lock's state start clean every time. A scenario
// passes only when the *reason* matches, not merely the outcome:
// a trap is what a deliberate re-entrancy panic produces and also
// what `std::time::Instant::now()` produces on wasm32, so the panic
// message is what tells them apart. The probe's panic hook leaves it
// in a static buffer; this runner reads it straight out of linear
// memory, at addresses it asks the module for *before* the scenario
// runs, so nothing is ever called on an instance that has trapped.

import fs from 'node:fs';

const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error('usage: node runner.mjs <probe.wasm>');
  process.exit(2);
}

// [export name, expected outcome]. `ok` expects a return of 1; a
// `panic` entry expects a trap whose captured message contains the
// given text.
const SCENARIOS = [
  ['uncontended_write', { ok: true }],
  ['uncontended_try_write', { ok: true }],
  ['uncontended_read', { ok: true }],
  ['reentrant_write_write', { panic: 'FONT_SYSTEM re-entrant acquire' }],
  ['reentrant_write_try_write', { panic: 'FONT_SYSTEM re-entrant acquire' }],
  ['reentrant_write_read', { panic: 'FONT_SYSTEM re-entrant acquire' }],
  ['reentrant_read_write', { panic: 'while holding a read guard' }],
];

const module = await WebAssembly.compile(fs.readFileSync(wasmPath));

let failures = 0;
for (const [name, expected] of SCENARIOS) {
  const imports = {};
  for (const imp of WebAssembly.Module.imports(module)) {
    (imports[imp.module] ??= {})[imp.name] = () => 0;
  }
  const instance = await WebAssembly.instantiate(module, imports);
  const { memory } = instance.exports;
  // Fetched now, while the instance is healthy.
  const messageAddr = instance.exports.panic_message_addr();
  const lenAddr = instance.exports.panic_message_len_addr();

  let returned;
  let trapped = false;
  try {
    returned = instance.exports[name]();
  } catch (e) {
    trapped = true;
  }
  // `memory.buffer` is read after the run: the module may have grown it.
  const len = new Uint32Array(memory.buffer, lenAddr, 1)[0];
  const message = new TextDecoder().decode(new Uint8Array(memory.buffer, messageAddr, len));
  const panics = message ? [message] : [];
  const why = message ? ` (panic: ${message})` : '';

  let problem = null;
  if (expected.ok) {
    if (trapped) problem = `trapped, expected to return 1${why}`;
    else if (returned !== 1) problem = `returned ${returned}, expected 1`;
  } else if (!trapped) {
    problem = `returned ${returned}, expected a panic containing "${expected.panic}"`;
  } else if (!panics.some((p) => p.includes(expected.panic))) {
    problem = `trapped without the expected panic "${expected.panic}"${why || ' (no panic message at all)'}`;
  }

  if (problem) {
    failures++;
    console.log(`FAIL ${name}: ${problem}`);
  } else {
    console.log(`ok   ${name}`);
  }
}

process.exit(failures ? 1 : 0);
