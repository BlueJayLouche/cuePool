# Show Control

CuePool can be driven by OSC, MIDI, MIDI Show Control, wall-clock time, and
its own show clock — and can drive other machines running CuePool as remote
nodes.

## OSC

*Project Settings → OSC / Remote* sets the network interface and ports
(defaults: receive 9000, transmit 8000).

Incoming commands (the address namespace is kept as `/qplayer` for
compatibility with the original QPlayer remote protocol and existing
controller rigs):

| Address | Arguments | Action |
|---|---|---|
| `/qplayer/go` | optional cue # | GO — or fire a specific cue |
| `/qplayer/stop` | optional cue # | Stop all — or one cue |
| `/qplayer/pause` | optional cue # | Pause all — or one cue |
| `/qplayer/unpause` | optional cue # | Resume all — or one cue |
| `/qplayer/preload` | cue #, time | Decode a cue and hold it Ready |
| `/qplayer/select` | cue # | Move the selection |
| `/qplayer/save` | — | Save the project |
| `/qplayer/volume` | dB (float), optional request ID (int32) | Set master gain, −96 (silence) to +12 dB; saved with the show. With an ID, reply after applying the setting |
| `/qplayer/volume/get` | request ID (int32) | Read the current master-volume setting through the application command queue |

The cue number can also ride in the address instead of the argument list —
`/qplayer/go/1.1` is the same command as `/qplayer/go` with `"1.1"` as its
argument (likewise `stop`, `pause`, `unpause`, `preload`, `select`). Handy for
controllers that template the address rather than the arguments.

`/qplayer/up` and `/qplayer/down` from the original QPlayer protocol are not
supported — move the selection with `/qplayer/select` and a cue number.

An incoming message whose address matches nothing CuePool subscribes to is
discarded and reported in *Window → Log*, once per address so a fader on a
wrong address does not fill it. A controller that appears to do nothing is
worth checking there before the network.

Outbound messages are sent by [OSC cues](cues.md#osc) — command format
`/address,arg1,arg2,…`.

### Master-volume feedback

Send `/qplayer/volume/get` with one OSC int32 argument, for example `41`.
CuePool replies with `/qplayer/volume/state 41 -12.0`: the same int32 ID and
the current gain as an OSC float32 in dB. The reply goes directly to the
requesting **IP address and source UDP port**, from CuePool's receive socket;
the configured OSC transmit host/port are not used. Send and listen on the
same controller socket. Remote-control client mode swaps the configured
receive/transmit ports; use the actual listening port shown in CuePool's log.

To confirm an adjustment, send `/qplayer/volume -18.0 42` (type tags `,fi`).
Its reply is `/qplayer/volume/state 42 -18.0` (`,if`). Setters and queries are
processed in arrival order through the application's command queue, so the
confirmation follows the applied change. Even an unchanged value receives a
confirmation. The original single-argument setter remains valid and sends no
reply. Queries do not change the gain or dirty the show.

Request IDs are opaque signed 32-bit integers, including zero and negative
values. Missing, non-int32 or extra query arguments are ignored. An invalid
optional setter ID or extra setter arguments cause the whole request to be
ignored. Volume retains the existing numeric conversion and clamping:
−96 dB or below is silence, +12 dB is the ceiling, and NaN becomes 0 dB.
Always use finite float32 dB values in controllers, not percentages or strings.

Feedback reads the clamped `ShowSettings.master_volume_db` at processing time,
so polling observes Project Settings edits, show loads and audio-device
rebuilds. The value is **saved with the show**, not per machine; OSC edits mark
the show dirty just like the GUI. Readback reports the master setting, not
measured audio output or proof of a functioning audio device. The gain remains
ahead of the limiter.

For a Nodel slider, poll once per second and give each outstanding request a
fresh ID. Keep pending user input separate from confirmed state, reject late
replies, throttle previews and always confirm the final adjustment. After
repeated timeouts, mark state unavailable and refresh by reading on reconnect.
Do not send another setter in response to feedback. A complete recipe, slider,
integration contract and automated tests are in
[`examples/nodel-volume`](https://github.com/BlueJayLouche/cuePool/tree/main/examples/nodel-volume).

OSC uses UDP here: IDs correlate replies but do not provide delivery guarantees,
server-side deduplication or global ordering between different senders. Clients
must not treat an old response as confirmation of a newer adjustment.

### DMX recorder

The [DMX Recorder](lighting.md#dmx-recorder) listens on the same OSC port:

| Address | Arguments | Action |
|---|---|---|
| `/dmx/{universe}/{channel}` | float 0–1 (or int 0–255) | Set a DMX channel (1-based) — live bridge, recorded while a pass runs |
| `/recorder/record` | — | Start a pass on the selected take; again = stop & keep |
| `/recorder/stop` | — | Stop the pass (keep) — or stop preview when idle |
| `/recorder/play` | — | Preview the selected take on the lighting output |
| `/recorder/select` | take name/path | Choose the target take (`.dmxrec` appended if missing) |
| `/recorder/discard` | — | Throw the in-flight pass away |
| `/recorder/revert` | — | Swap the take with its previous version |

Build a touchOSC layout of faders addressed `/dmx/1/1`, `/dmx/1/2`, … and
you have a hand-held DMX console; values are held (latest wins) until
**Clear** in the recorder panel. MIDI CC works the same way: enable
**MIDI CC → universe** in the panel and CC# = channel. Status feedback
(recording LEDs etc.) is not implemented yet.

A ready-made layout ships at
`assets/CuePool-DMX-Recorder.touchosc` — 16 faders
(universe 1, ch 1–16, two pages) plus REC / STOP / PLAY / DISCARD / REVERT.
It's the classic `.touchosc` format: open it directly in TouchOSC Mk1, or
**File → Import** in current TouchOSC. Point the connection's *send* host
at the CuePool machine on the OSC RX port from Settings. Buttons fire on
press only — the release value of 0 is ignored.

## Show clock & timecode

The transport bar shows the **show clock** — the clock
[timecode triggers](#per-cue-triggers) fire against — as `HH:MM:SS.ff`
(green = running, yellow = paused, `--:--:--.--` before the first GO).
The frame part is display-only; set its rate under **Settings → Timecode**
(triggers are stored in seconds). Next to it, the next armed trigger is
shown as `next: Q12 @ 00:03:15.00`.

**Pause freezes the show clock** and no timecode triggers fire while
paused. While paused, **⏭ / ⏮ frame-step** move the current video one frame
forward or back with the clock following in lockstep — creep up on the exact
moment, then either hit **Capture** on a cue's timecode trigger or just add
a new cue: **cues created while the clock is live are pre-filled with a
timecode trigger at the current time.** Triggers stepped past while paused
fire on resume. (Stepping back briefly re-seeks the decoder, so the first
back-step can take a moment on long-GOP files.)

## Per-cue triggers

Every cue has an optional *Triggers* section in the Inspector, in addition
to the GO chain:

- **Hotkey** — a key that fires the cue directly.
- **MIDI** — Note On / Note Off / CC on a channel, with a minimum velocity.
- **Wall clock** — a time of day (12/24-hour), once or daily. Useful for
  house music, pre-show loops, and installations.
- **Timecode** — a time on the show clock, which is started by a
  [TimeCode cue](cues.md#timecode). Entered as `HH:MM:SS.FF` (matching the
  show clock display; a bare number is plain seconds). A capture button
  stamps the trigger with the current show time.

## MIDI Show Control (MSC)

*Project Settings → MSC* enables MSC over the network (default ports 6004,
receive device `0x70`, transmit device `0x71`, optional executor/page
filters), so a lighting desk can GO CuePool — or vice versa.

## Remote nodes

*Project Settings → OSC / Remote* also enables **remote control**: multiple
CuePool machines discover each other over OSC by node name. Each machine takes
its hostname as its node name, so two of them are already distinguishable
before you configure anything. One node is the host; the others are clients.
With *sync show file on save* enabled, saving on the host pushes the project to
the clients. Set a cue's *Remote Node* field to make it fire on that named
machine instead of locally — e.g. a video machine at front of house triggered
from the sound desk.

The field takes free text, because a node that has not broadcast yet still has
to be nameable, but the ⏷ picker beside it lists the machines actually
detected. The Inspector says so when a cue would not play where you meant it to:

- **The named node has never been detected.** The cue is still sent — a network
  that filters broadcast can hide a node that is really there — but nothing
  answers if the name is simply wrong, and the cue plays nowhere at all.
- **The node was detected but has gone quiet.** Usually the machine is off or
  off the network. This clears within a few seconds of the node answering, so a
  warning that persists after loading a show file is a real one.
- **Remote control is off.** The cue falls back to this machine.
- **The name is this machine's own node name.** The cue plays here.

The last two are deliberate fallbacks rather than errors, but neither is silent:
each raises an operator alert if it happens during a show, and is logged to
*Window → Log*.
