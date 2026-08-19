# DMX Recorder

DMX show recorder for cuepool: capture Art-Net/sACN (and OSC-driven channels)
off the wire, store as `.dmxrec` sidecar files, play back as lighting cues.
Design agreed 2026-07-08.

## Context

There is no open interchange standard for *recorded* DMX — hardware recorders
(Swisson XRC, ELC showStore, DecaBox) all use proprietary formats. The
standard part is the wire protocol: we record standard sACN (ANSI E1.31) and
Art-Net input and store it in our own format.

Primary workflow: program lighting in a console (stageLX or anything else),
record its DMX output over the network, play back in cuepool at show time.
Secondary: touchOSC → recorder's OSC→DMX bridge → record/perform live.

## Architecture

- **Core** (capture, format, playback, merge) lives in `rustjay-lighting` —
  shared with stageLX or a future standalone recorder if ever needed.
- **UI** (recorder panel, cue type, editor) lives in cuepool.
- Standalone recorder app: deferred. Cuepool records stageLX's output off the
  wire already (loopback or network); build the app only when recording must
  happen on a machine without cuepool.

## Capture

- Art-Net + sACN receive (`DmxReceiver`, mirror of `DmxSender`), reusing the
  existing `parse_sacn` / `parse_artdmx` / `rx_socket`.
- **Sparse universes**: up to 512 universes supported, allocated only when
  first seen on the wire. Recording silence costs nothing.
- sACN multicast groups are joined per-universe on demand (config list);
  unicast and broadcast work with no configuration.
- **OSC as DMX source** (level 1, literal): `/dmx/{universe}/{channel} <float>`
  → channel value. MIDI CC → channel via a base-universe setting. No mapping
  table — fixture-level intelligence stays upstream in the console.
- **Live bridge**: the OSC→DMX path also works outside recording as a
  show-time touchOSC bridge, with its own merge priority.
- **Monitor output** toggle (default ON): the recorder's merged result goes
  out the same per-universe destinations playback will use, so punch-in
  monitoring shows the merged take.

## Recording semantics

- **Punch-in per channel**: during a pass over an existing take, a channel
  punches in the moment its incoming value first differs from the recorded
  take, and stays live until end of pass. No thresholds, no arming.
- **Single-level revert**: before each pass the current take is kept as
  `<name>.dmxrec.prev`; one button / `/recorder/revert` swaps back. The next
  pass discards it.

## Playback (cuepool)

- New cue type referencing a `.dmxrec` path (sidecar media, like audio/video).
- Free-runs from cue trigger — no SMPTE/MTC chase. Audio alignment = trigger
  both cues together.
- Fade in/out over the cue's fade time via the existing crossfade engine;
  channels released (LTP-correct) after fade-out.
- Per-cue **loop** option like other cue types; wrap is a hard jump
  (last frame → first frame). Crossfade-on-wrap is a possible later add.
- Holds last frame at end (non-looping) until stopped or superseded.

## Merge model (sACN-style LTP)

Every DMX source has a priority 0–255; per channel, the highest-priority
source **with data** wins; equal priorities → latest change wins (LTP).

| Source | Priority | Where set |
|---|---|---|
| Each recorded-show cue | default 100 | cue inspector field |
| FixtureLook engine | default 100 | one global setting |
| Live OSC/MIDI input | default 150 | recorder panel |

A source owns only channels where it has data: recordings own their sparse
channel set, looks own patched fixtures' channels. Nothing fancier: no
per-channel priorities, no HTP zones, no priority automation.

## Editor

Per-channel value-over-time curves (universe/channel picker), scrub playhead.
Edits: drag/insert/delete points, flatten selection to constant, trim
recording start/end, delete a channel's data entirely, per-channel time-shift.
Skipped: copy-between-channels, generators, multi-channel lasso — punch-in
re-recording is the fix for anything bigger than point surgery.

## Control (OSC/MIDI transport)

OSC verbs on cuepool's existing server, `/qplayer/*`-style:

- `/recorder/record` — start pass on selected recording (new or punch-in);
  again = stop and keep
- `/recorder/stop` — stop pass or preview
- `/recorder/play` — preview selected recording from the panel
- `/recorder/select <name>` — choose target recording
- `/recorder/discard` — stop and throw away the in-flight pass
- `/recorder/revert` — swap back to previous take

MIDI maps to the same verbs through the existing cue MIDI mapping. Status
feedback (touchOSC LEDs etc.) is stubbed — reply hook exists, unimplemented.

## File format: `.dmxrec`

Sidecar file next to the project; the cue stores the path in `.qproj`.
Gzip (flate2) over:

```
header:  b"DMXREC" + u16 version (LE)          — 8 bytes
events:  repeated 9-byte records (LE):
         u32 t_ms | u16 universe | u16 channel | u8 value
```

- Timestamps are ms from recording start (`u32` → 49-day max).
- Flat append-only event log: the recorder streams to disk during the pass
  (a crash loses seconds, not the take). No fixed tick grid — sources send at
  whatever rate they send; the editor/playback index per-channel on load.
- Duration = max `t_ms` (computed on load, not stored — gzip streams can't
  seek back to patch a header).
- Revert file: `<name>.dmxrec.prev`.

## Phases

1. **DONE (2026-07-08)** — format (`rec.rs`) + RX capture (`rx.rs`) in
   `rustjay-lighting`, loopback-tested against existing TX.
2. **DONE (2026-07-08)** — `DmxShowCue` + LTP merge + playback:
   `play.rs` (`ShowPlayer`, `MaskedFrame`, `composite`) in `rustjay-lighting`;
   cuepool `LightingEngine` composites looks + shows per destination
   (recordings play to the project-level destination); Stop cues,
   AfterLast chaining, all four loop modes, tail fade mirroring SoundCue;
   `look_priority` on `LightingConfig`; minimal inspector + add-cue UI.
   Equal-priority tie-break is source paint order (looks first, then shows
   by start), not per-channel LTP timestamps — upgrade if it ever matters.
3. **DONE (2026-07-08)** — recorder panel + punch-in + monitor + revert:
   `punch.rs` (`PunchRecorder`, clock-free, streams pass to `<take>.pass`)
   in `rustjay-lighting`; cuepool `recorder.rs` (RX lifecycle, stop-and-keep
   with `.prev` rotation, revert swap) + `recorder_panel.rs` + monitor as a
   `LightingEngine` overlay layer at priority 150. sACN multicast joins not
   exposed in the panel yet (unicast/broadcast only). Real-world gotcha
   baked into a test: another sACN app on :5568 (reuseport) steals unicast —
   tests bind ephemeral ports.
4. **DONE (2026-07-08)** — OSC DMX source / live bridge + transport verbs:
   `/dmx/{u}/{ch}` (1-based channel on the wire, float 0–1 or int 0–255) +
   MIDI CC → configurable universe; `Recorder.live` MaskedFrame is the
   bridge (held values, Clear button), output as the overlay while idle and
   fed to punch-in during a pass. `/recorder/record|stop|play|select|
   discard|revert` wired through the existing OscManager; panel gained
   Preview (sentinel show qid `-1`, excluded from AfterLast). OSC status
   feedback still stubbed. Overlay priority still the const 150.
5. **DONE (2026-07-08)** — Take Editor (`cuepool-gui/src/take_editor.rs`,
   GUI-local, loads/saves `.dmxrec` directly): step-curve canvas with
   drag/insert(double-click)/delete(right-click) points, scrub playhead
   with optional output (held via `Recorder::set_scrub`, wins over the
   live bridge), log zoom + scroll, channel picker, delete-channel,
   per-channel time-shift, trim (held values materialise as t=0
   baseline), flatten-range with resume value, single-level undo, Save
   rotates `.prev`. Editing disabled while a pass records. Trim/flatten/
   shift/state_at covered by unit tests.

## Post-v1 backlog

- Hands-on GUI run against a real console/touchOSC (all verification so
  far is headless through real sockets).
- OSC status feedback (`/recorder/*` replies, touchOSC LEDs).
- sACN multicast universe joins in the recorder panel.
- Configurable live-input/monitor priority (const 150 today).
- Per-channel LTP tie-break timestamps, crossfade-on-loop-wrap — only if
  practice demands them.
