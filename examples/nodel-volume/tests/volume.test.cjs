const {test} = require('node:test');
const assert = require('node:assert/strict');
const Panel = require('../content/volume.js');

test('real recipe snapshot survives the stock Nodel WebUI key encoding', () => {
  const {execFileSync} = require('node:child_process');
  const {join} = require('node:path');
  const json = execFileSync('python3', ['-c',
    'import json, struct; from volume import VolumeController, STATE_PREFIX; ' +
    'c = VolumeController(lambda p: None, lambda s: None, lambda: 1, 42, "test"); ' +
    'c.connected(); c.receive(STATE_PREFIX + struct.pack(">if", 42, -9)); ' +
    'print(json.dumps(c.snapshot()))'], {cwd: join(__dirname, '..'), encoding: 'utf8'});
  // Nodel's WebUI overrides JSON.parse and encodes punctuation in object keys.
  const parsed = JSON.parse(json, (key, value) => {
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return Object.fromEntries(Object.entries(value).map(([k, v]) =>
        [k.replace(/(^[0-9]|[^0-9a-zA-Z])/g, c => '__' + c.charCodeAt(0) + '__'), v]));
    }
    return value;
  });
  const panel = new Panel('panel');
  panel.update(parsed);
  assert.equal(panel.value(), -9);
});

function state(overrides = {}) {
  return {session: 'a', revision: 1, available: true, confirmedDb: -6,
    pendingDb: null, client: null, sequence: null, final: false, ...overrides};
}

test('rapid dragging coalesces previews and retains the final value', () => {
  const panel = new Panel('panel');
  panel.update(state());
  for (let i = 1; i <= 100; i++) panel.input(-i / 2, false);
  panel.input(-50, true);
  assert.equal(panel.queue.length, 1);
  assert.equal(panel.queue[0].db, -50);
  assert.equal(panel.queue[0].final, true);
  panel.input(-49, false); // A new drag does not discard the previous final.
  assert.equal(panel.queue.length, 2);
});

test('old feedback cannot pull the slider backwards while dragging or pending', () => {
  const panel = new Panel('panel');
  panel.update(state());
  panel.input(-20, false);
  panel.update(state({revision: 2, confirmedDb: -8}));
  assert.equal(panel.value(), -20);
  panel.input(-24, true);
  panel.update(state({revision: 3, confirmedDb: -20, client: 'panel', sequence: 1, final: false}));
  assert.equal(panel.value(), -24);
  panel.update(state({revision: 4, confirmedDb: -24, client: 'panel', sequence: 2, final: true}));
  assert.equal(panel.pending, null);
  panel.update(state({revision: 2, confirmedDb: -8}));
  assert.equal(panel.value(), -24);
});

test('GUI and show changes update idle state without generating commands', () => {
  const panel = new Panel('panel');
  panel.update(state());
  panel.update(state({revision: 2, confirmedDb: -18}));
  assert.equal(panel.value(), -18);
  assert.deepEqual(panel.queue, []);
  panel.update(state({revision: 3, confirmedDb: -96}));
  assert.equal(panel.value(), -96);
  assert.deepEqual(panel.queue, []);
});

test('missing feedback and Nodel restart discard old pending actions', () => {
  const panel = new Panel('panel');
  panel.update(state());
  panel.input(-30, true);
  panel.update(state({revision: 2, available: false}));
  assert.equal(panel.pending, null);
  assert.deepEqual(panel.queue, []);
  panel.update(state({session: 'b', confirmedDb: -3}));
  assert.equal(panel.value(), -3);
  panel.input(-9, true);
  assert.equal(panel.queue[0].session, 'b');
  panel.update(state({session: 'c', confirmedDb: -1}));
  assert.equal(panel.value(), -1);
  assert.deepEqual(panel.queue, []);
});

test('panel disconnect does not replay cached input after reconnect', () => {
  const panel = new Panel('panel');
  panel.update(state());
  panel.input(-32, true);
  panel.disconnected();
  assert.deepEqual(panel.queue, []);
  assert.equal(panel.pending, null);
  panel.update(state({revision: 2, confirmedDb: -7}));
  assert.equal(panel.value(), -7);
  assert.deepEqual(panel.queue, []);
});

test('Nodel serialisation may omit null fields from confirmation', () => {
  const panel = new Panel('panel');
  panel.update(state());
  panel.input(-24, true);
  panel.update({session: 'a', revision: 2, available: true, confirmedDb: -24,
    client: 'panel', sequence: 1, final: true});
  assert.equal(panel.pending, null);
  assert.equal(panel.value(), -24);
});
