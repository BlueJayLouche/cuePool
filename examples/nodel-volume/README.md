# Nodel master-volume slider

A standalone reference node for CuePool's correlated OSC volume API. It uses
Nodel's managed UDP socket and timer, the stock `range` renderer, and a small
frontend adapter. No additional Python or JavaScript packages are required.

The venue dashboard was not specified unambiguously during implementation.
These files are a runnable integration example, not a deployment to a live node.

## Install

1. Build/deploy CuePool with the volume-feedback change; unmodified 0.12.3 has
   the setter but does **not** answer the new queries.
2. Copy `script.py`, `volume.py` and `content/` into a new Nodel node directory.
   Keep `volume.py` alongside `script.py` so Jython can import it. For an existing
   node, integrate the recipe's parameters, actions, events and callbacks;
   do not replace its existing script or content wholesale.
3. Set `ipAddress` to CuePool's IPv4 address and `port` to its actual OSC receive
   port (normally 9000). These are the only required settings. The node sends
   and receives on one automatically allocated source port; replies do not use
   CuePool's configured OSC transmit destination. Permit UDP replies to that
   source port on the controller host.
4. Open the node's normal frontend. It starts unavailable, reads the current
   level and enables the slider on the first correlated response.

When adding this to an existing dashboard, bind its volume action to `Volume`
and forward the `VolumeState`, `ConfirmedVolume` and `VolumeAvailable` events.
Use the same frontend adapter or implement the action/event contract below.
Avoid a direct range-to-ConfirmedVolume binding: it would overwrite pending
user input. If another node forwards the action, forward its complete object.

## OSC contract

| Request | Type tags | Response |
|---|---|---|
| `/qplayer/volume/get <request_id>` | `,i` | `/qplayer/volume/state <request_id> <db>` (`,if`) |
| `/qplayer/volume <db> <request_id>` | `,fi` | Same response, after applying the setting |
| `/qplayer/volume <db>` | `,f` | Legacy setter: no reply |

IDs are opaque signed int32 values. Replies go to the requesting IP and source
port. The controller validates the sender, address, exact type tags and ID.
Each request gets a fresh ID; it keeps only one request in flight and ignores
duplicates, timed-out replies and IDs from older adjustments. A new node session
uses a random starting ID and a fresh socket. IDs wrap across the signed int32
range without using numeric ordering to decide freshness.

Levels are float32 dB: -96 is silence, 0 is unity and +12 is the ceiling. CuePool
clamps requests and reads its current show setting, not a separate OSC cache.
GUI edits and show loads are visible to the next poll. Settings survive audio
engine replacement and are saved **with the show**. Readback is not an audio
meter or an assertion that an audio device is working.

## Nodel action and events

`Volume` accepts an object. Through Nodel REST, POST to
`REST/actions/Volume/call` with this shape:

```json
{"arg":{"db":-18.0,"client":"panel-unique-id","sequence":42,"final":true,"session":"session-from-VolumeState"}}
```

Use a unique `client` per panel load and increase `sequence` for every input,
including the final change. `final:false` is a drag preview; `final:true` commits
the last value. `session` comes from the latest `VolumeState` event and prevents
HTTP requests from an old Nodel session applying after a restart.

- `ConfirmedVolume`: the last correlated CuePool level, in dB.
- `VolumeAvailable`: true while feedback is available; false initially and
  after three consecutive one-second request timeouts.
- `VolumeState`: `{session, revision, available, confirmedDb, pendingDb,
  client, sequence, final}`. `revision` increases within a session. The panel
  rejects old revisions and holds local pending input until its final sequence
  is confirmed (`pendingDb:null`). A new session resets pending input.

Nodel versions that omit null object fields report an absent `pendingDb`
instead; treat that as null. `confirmedDb` is likewise absent until first read.
Event object keys are alphanumeric because the stock WebUI JSON parser rewrites
punctuation in property names, including underscores.

`ConfirmedVolume` remains the last known value while unavailable. Always read
it alongside `VolumeAvailable`, or use `VolumeState` for an atomic snapshot.
Feedback updates the UI's value without dispatching an input/change event.

## Timing and recovery

The node reads on socket readiness and polls once per second when idle. Its
50 ms timer coalesces drag previews to at most 10 sends per second, waits for
each reply and always sends a final adjustment, even if the value matches the
last preview. The frontend also coalesces previews and serializes HTTP actions.
Its 250 ms event reads do not cause extra OSC polls.

An older in-flight reply may update the confirmed value, but cannot clear newer
pending intent or pull the slider backwards. A lost final request/response is
retried with a fresh ID until three requests time out. The node then abandons
pending input, marks feedback unavailable and polls for recovery. It reads the
current level on reconnection rather than restoring an old command. If the
panel loses Nodel or an action fails, it clears unsent input and refreshes state.

Multiple panels use last-received-wins control. A panel releases an unconfirmed
final intent after five seconds, covering a lost HTTP action or another panel
taking control. Its local pending display is never presented as confirmed.
As with the existing OSC setter, UDP does not guarantee delivery or ordering;
request IDs are not server-side transaction IDs or deduplication keys.

## Verification

From the CuePool repository root:

```sh
cargo test -p cuepool --test master_volume_osc --locked
cargo test -p cuepool --features test-harness --test master_volume_osc --locked
python3 -m unittest discover -s examples/nodel-volume/tests -p 'test_*.py'
node --test examples/nodel-volume/tests/volume.test.cjs
python3 examples/nodel-volume/tests/smoke_nodel.py --jar /path/to/nodelhost.jar
```

The Rust tests use real OSC sockets, the production router and application
volume handler. Python and JavaScript tests cover dragging, stale replies,
timeouts, restarts and feedback without another command. The smoke test starts
a disposable Nodel/Jython host and a loopback OSC simulator, exercises the real
recipe through Nodel REST, checks console errors and removes its temporary
files on exit. `--hold` keeps it available for browser inspection; Ctrl-C cleans
up. It uses no venue node, audio device or external controller.

Implementation verification on macOS passed all required Cargo checks,
including 528 workspace tests and six focused OSC/audio tests, plus nine Python
and seven JavaScript tests. The disposable Nodel 2.2.1 host passed the real
Jython recipe smoke test. Browser checks confirmed keyboard and pointer
adjustments at desktop and narrow-panel sizes, with no console errors. Those
checks used HTML transformed with the host's actual XSL: automated navigation
to the native XML frontend was blocked by the browser, so that path remains
to be checked on the venue panel.

Before venue cutover, check the rendered slider on the actual panel, confirm
the final value at CuePool, change the GUI level, load a show with another saved
level, rebuild the audio device and restart each application. Check audio and
the limiter on the attended rig. Local/simulated tests do not establish those
hardware results. Rolling back the Nodel integration leaves the legacy setter
and the show-file volume field intact.
