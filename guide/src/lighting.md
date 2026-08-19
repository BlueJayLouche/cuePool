# Lighting & Pixel Mapping

CuePool sends DMX over **sACN** (E1.31) or **Art-Net**, built on the same
[`rustjay-lighting`](https://bluejaylouche.github.io/rustjay-engine/lighting.html) crate as the rest of the engine.
Everything lives in *Window → Lighting* and is saved in the project file.

## DMX output

At the top of the Lighting panel: enable output, pick the protocol, and set
an optional unicast destination IP (leave empty for sACN multicast /
Art-Net broadcast). The refresh rate defaults to 44 fps.

## Patching fixtures

*Patch* lists your rig: each fixture is a **profile** at a universe +
1-based DMX address (the channel footprint is shown next to it). Each
fixture can also set its own unicast **IP** — useful when several Art-Net
or sACN nodes each expect their universes on their own address; leave it
empty to use the panel-level destination. Overlap warnings are checked per
destination, so the same universe on different nodes doesn't collide.
Built-in profiles:

| Profile | Channels |
|---|---|
| RGB / GRB / BGR | 3-channel colour in the named order |
| RGBW | Colour + white |
| RGB + Dimmer / Dimmer + RGB | Colour with a dimmer channel after/before |
| Dimmer | Single channel |
| Moving Head (16-bit) | Pan/tilt (16-bit) + dimmer + colour + beam |

*+ New Profile* builds a custom profile from channel roles (colour, dimmer,
pan/tilt coarse+fine, zoom, strobe, gobo, static values for fixed channels,
…). User profiles override a built-in with the same id.

## Profiles

Profiles define a fixture's channel layout. In the profile editor, channel
chips **drag to reorder** (the order *is* the DMX layout), **right-click to
delete**, and hovering names the channel type. **Import GDTF…** loads a
manufacturer `.gdtf` fixture file: pick a DMX mode and the channels map to
CuePool roles automatically (dimmer, RGB+W/A/UV, 16-bit pan/tilt, zoom,
strobe, gobo); anything unrecognized becomes a constant channel holding its
GDTF default value, so footprints and mode bytes stay correct.

**Export sheet…** (next to the patch) saves a CSV patch sheet: one summary
row per fixture with per-channel detail rows underneath, plus pixel-map
segment spans (one row per universe crossed), sorted by universe/address —
with a `Notes` column flagging any address overlaps.

## Lighting cues

A **Lighting** cue stores a *look* — dimmer, colour, white, pan/tilt, and
beam (zoom / strobe / gobo) values — for any subset of the patch, and
crossfades the rig to it over *Fade (s)* with a selectable
[curve](cues.md#fade-curves).

Fixtures **not** included in a cue keep whatever state the previous cue left
them in (LTP tracking): build your show as a sequence of partial looks, and
only the fixtures you touch change.

### Live programming

Tick **🔴 Live** at the top of a Lighting cue's inspector to stream every
look edit straight to the fixtures while you program — what you see on
stage is what the cue will play. The toggle is session-only (not saved) and
follows LTP: fixtures not included in the cue hold their current levels.

## DMX Show cues

A **DMX Show** cue plays a recorded DMX stream (a `.dmxrec` file) straight
to the lighting output — raw universe data captured from a console or
recorder, bypassing the fixture patch.

Recordings merge with Lighting-cue looks sACN-style: every source has a
**priority** (0–255, default 100); on each channel, the highest-priority
source *with data there* wins. A recording only owns the channels it
actually touches; the look engine owns patched fixtures' channels (its
priority is a project-level setting, default 100 — at equal priority the
recording wins). *Fade In/Out* crossfade the recording's channels against
whatever is underneath, so pan/tilt fade as movement, not a dip to zero.

Loop follows the cue's Loop mode: **OneShot** releases its channels at the
end (with the tail fade), **HoldLast** holds the final frame until stopped,
**Looped/Looped ∞** wrap hard from last frame to first. A Stop cue targeting
the DMX Show fades it out and releases its channels.

## DMX Recorder

**Window → DMX Recorder** captures incoming sACN (`:5568`) and Art-Net
(`:6454`) — unicast or broadcast — into a `.dmxrec` take: program on your
console (or stageLX), record the wire, play it back as a DMX Show cue.

Pick or create a take file, hit **⏺ Record**, perform the pass, **⏹ Stop &
Keep**. Recording *over an existing take* is a punch-in pass: each channel
overwrites the old take only from the moment its incoming value first
deviates, and stays live until the pass ends — channels you don't touch
keep their old data, and after the pass end the old take resumes. **Discard**
throws the pass away; **Revert** swaps back to the previous take (one level,
kept as `<take>.prev`).

**Monitor** (default on) streams the merged result — old take playing plus
your live punches — to the lighting output while you record, above playback
and looks in the merge. The raw pass also streams to `<take>.pass` as it
happens, so a crash loses seconds, not the take. **▶ Preview** plays the
take through the lighting output without needing a cue.

### Take Editor

**✏ Edit…** in the recorder panel opens the take as per-channel curves —
value over time, one channel at a time (universe/channel picker). On the
canvas: **drag** a point to move it (time stays between its neighbours),
**double-click** empty space to insert one, **right-click** a point to
delete it, and **drag on empty canvas** to scrub the playhead ("Output
scrub" sends the frame at the playhead to the rig). Zoom is logarithmic
with a scroll slider for long takes.

Channel ops: **Delete channel** (hands its addresses back to other
sources) and **Shift (ms)** (time-shift one channel, clamped at zero).
Range ops: **Trim take** (keeps only the window — values held at the
window start become the new baseline) and **Flatten** (holds the selected
channel at a constant over the range; the old curve resumes after it).

Edits live in memory until **Save**, which keeps the previous version as
`.prev` — same single-level revert as recording. One level of **Undo**
covers the last operation. Bigger fixes are usually faster as a punch-in
re-record than as mouse surgery.

### Live input (OSC / MIDI)

`/dmx/{universe}/{channel} <0–1>` sets channels directly — from touchOSC or
anything speaking OSC — and MIDI CC does the same on a configurable universe
(CC# = channel). Live input is a *bridge*: it outputs even when not
recording, at high merge priority (150) so a hand on a fader overrides
playback; while a pass runs it is recorded like wire input. Values are held
until **Clear**. Transport verbs (`/recorder/record` …) are listed in
[Show Control](show-control.md#dmx-recorder).

> Close other sACN listeners (e.g. sACNView) while recording *unicast* —
> port sharing can route packets to the other app. Broadcast is unaffected.

## Pixel-map segments

*Segments* stream video onto LED fixtures, vjarda-style. Each segment
samples a rectangle of a source texture, downsamples it to a `cols × rows`
grid, and writes one fixture-profile-worth of channels per cell starting at
a universe/address, walking the grid in the chosen scan order (row/column
order, serpentine, …).

| Property | Meaning |
|---|---|
| Source | **PixelMap** (default — a dedicated texture fed by PixelMap cues, LED content independent of the projector picture) or **Canvas** (mirror what the projectors show). Projects saved before the default changed keep their explicit Canvas — flip the dropdown if you want PixelMap cues to drive those segments. |
| Region | Normalized rectangle of the source to sample. |
| Grid | `cols × rows` cells — one fixture per cell. |
| Profile / U / Ch | Fixture profile, universe, and 1-based start address of the first cell. |
| Scan | Cell-to-address walking order. |
| Brightness / Gamma / White | Colour pipeline: output gamma (default 2.2) maps the display-referred canvas to LED-linear intensity; the white mode controls RGBW derivation (use **Off** for plain RGB). |
| Derive A/UV | Off by default: profiles with Amber/UV channels emit 0 on them. Tick to approximate Amber ((R+G)/2) and UV (0.8·B) from the sampled colour. |

While a segment has content it streams continuously, and its channels
**override lighting-cue looks** on the same addresses.

## PixelMap cues

A [PixelMap cue](cues.md#pixelmap) plays a video or still into the dedicated
pixel-map texture. Point segments at the **PixelMap** source to drive LEDs
with it; a OneShot cue blanks the texture to black when it ends.
