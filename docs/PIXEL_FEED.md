# Pixel feed

The pixel feed mirrors pixel-map samples over WebSocket for external visualisers.
It is disabled unless `CUEPOOL_PIXELS_BIND` is set:

```sh
CUEPOOL_PIXELS_BIND=127.0.0.1:7134 cuepool
```

Connect to `ws://<bind>/v1/pixels`. The optional query parameters are `fps`
(1–60, default 30) and `segments`, a comma-separated list of segment IDs.
Omitting the filter includes every active segment. IDs that cannot be parsed
are ignored; if none can be parsed, the filter streams nothing.

The pixel feed runs on its own port and thread, separately from the
[automation API](AUTOMATION.md). It exposes pixel samples; project details and
logs remain on the API's `/v1/project` and `/v1/logs` endpoints.

The automation API accepts only loopback addresses. The pixel listener accepts
any bind address and, like sACN and Art-Net output, is unauthenticated and
unencrypted. Prefer a specific interface (`192.168.10.5:7134`) over `0.0.0.0`
so a machine connected to two networks does not serve the feed on both.
Up to eight clients can connect at once; further connections get `503`.
Idle connections are pinged every 30 seconds to reclaim disconnected clients' slots.

Any web page can attempt to connect through a visitor's browser. To restrict
which browser `Origin`s may connect, set `CUEPOOL_PIXELS_ORIGINS` to a
comma-separated list, such as `https://vis.example` or `null` for a `file://`
page. Unlisted origins get `403`. Requests without an `Origin` header, such as
those from non-browser clients, always pass this check. When the variable is
unset, any origin is accepted.

On connect, and whenever the patch changes, the server sends a JSON text frame
describing the segments:

```json
{"type": "segments", "segments": [
  {"id": 1, "name": "Upstage truss", "cols": 32, "rows": 8,
   "region": [0.0, 0.0, 1.0, 0.25], "source": "PixelMap",
   "universe": 1, "address": 1, "gamma": 2.2,
   "order": {"start_corner": "TopLeft", "serpentine": true, "primary": "Horizontal"}}]}
```

Pixel data then arrives as binary frames, one per segment, only when that
segment's pixels change. Each frame is a 12-byte little-endian header followed
by tightly packed RGBA, row-major from the top-left:

| Offset | Type | Meaning |
|---|---|---|
| 0 | `u32` | Segment id |
| 4 | `u16` | Columns |
| 6 | `u16` | Rows |
| 8 | `u32` | Milliseconds since the stream opened (wraps every ~49.7 days) |
| 12 | … | `cols * rows * 4` bytes RGBA |

**Trust the frame header over the metadata for dimensions.** The sampler can lag
a grid resize by a frame, so the two disagree briefly after an edit.

These are raw samples, taken before `demux_tile` applies the segment's wiring and
before the gamma, white-mode and colour correction that shape the actual DMX
output. A visualiser showing source imagery can ignore `order`; one mirroring
fixture wiring should apply it.

```js
const socket = new WebSocket("ws://127.0.0.1:7134/v1/pixels?fps=30");
socket.binaryType = "arraybuffer";
const ctx = document.querySelector("canvas").getContext("2d");

socket.onmessage = (event) => {
  if (typeof event.data === "string") {
    console.log("segments", JSON.parse(event.data).segments);
    return;
  }
  const view = new DataView(event.data);
  const cols = view.getUint16(4, true);
  const rows = view.getUint16(6, true);
  const pixels = new Uint8ClampedArray(event.data, 12);
  ctx.putImageData(new ImageData(pixels, cols, rows), 0, 0);
};
```

See also: [Lighting & Pixel Mapping](../guide/src/lighting.md) · [CuePool](../README.md)
