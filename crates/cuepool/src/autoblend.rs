//! OSC-driven camera auto-blend calibration for projection outputs.
//!
//! A network camera stream (RTSP/HLS) watches the projection surface while
//! CuePool runs pattern passes on the projector outputs: AprilTag markers
//! (identity) and discrete solid colors (geometry + overlap). The measured
//! camera-space quads feed a per-output corner-pin warp and edge-blend widths
//! between canvas-adjacent outputs, written into the live `ProjectionConfig`
//! on `apply` — the winit tick's output diff pushes them to the render
//! threads, so no window rebuild is needed.
//!
//! Status is reported through the log; the OSC events carry no source address,
//! so there is no direct reply path (unlike `/qplayer/remote/ping`).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use cuepool_core::{
    CanvasFit, EdgeBlendEdge, ProjectionConfig, ProjectorOutput, TestPattern, WarpCorners,
};
use cuepool_video::VideoFrame;
use cuepool_video::calibration::capture::StreamCapture;
use cuepool_video::calibration::detect::{
    AprilTagDetector, OverlapRegion, PALETTE, average_color, find_color_quad, generate_marker,
    measure_overlap,
};
use cuepool_video::homography::{apply_3x3, compute_forward_homography, invert_3x3, mul_3x3};

use crate::video_pipeline::CanvasCommand;

/// Pattern → capture settle time: the camera needs a few frames of the new
/// pattern before measurement.
const SETTLE: Duration = Duration::from_millis(750);
/// Re-poll interval while waiting for the first stream frame.
const FRAME_RETRY: Duration = Duration::from_millis(250);
/// Give up on a pass after this many frame retries (~4 s of stream startup).
const MAX_FRAME_RETRIES: u32 = 16;
/// Per-channel tolerance for the color-region and overlap matchers.
const COLOR_TOLERANCE: u8 = 48;
/// Tag side as a fraction of the smaller source-rect dimension.
const TAG_FILL: f32 = 0.5;

const UNIT_SQUARE: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// Camera-space quad, corners TL/TR/BR/BL.
type Quad = [[f32; 2]; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Streaming,
    Markers,
    Colors,
    White,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassKind {
    Markers,
    Colors,
    /// White → gray → black photometric series; `PendingPass.step` tracks
    /// which level is on screen (0=white, 1=gray, 2=black).
    White,
}

/// A pattern pass waiting for its settle timer before the camera capture.
struct PendingPass {
    kind: PassKind,
    targets: Vec<usize>,
    not_before: Instant,
    frame_retries: u32,
    /// Photometric sub-step (White pass only): 0=white, 1=gray, 2=black.
    step: usize,
}

/// The three measured camera-space levels of one output's solo region.
#[derive(Default, Clone, Copy)]
struct Levels {
    white: Option<[f32; 3]>,
    gray: Option<[f32; 3]>,
    black: Option<[f32; 3]>,
}

/// Steps the `run` macro walks through once the stream is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStep {
    Markers,
    Colors,
    White,
    Apply,
}

/// Overlap measurement for one canvas-adjacent pair. `vertical`: the pair
/// shares a vertical canvas edge (`before` left of `after`).
struct AdjacentOverlap {
    vertical: bool,
    /// `None` = the pair was measured but no overlap was found.
    region: Option<OverlapRegion>,
}

/// The auto-blend calibration state machine. Owned by `App`; driven by OSC
/// events and the per-tick `tick`.
pub(crate) struct AutoBlend {
    phase: Phase,
    capture: Option<StreamCapture>,
    pending: Option<PendingPass>,
    /// Measured camera-space quads per output index, kept across passes so a
    /// per-output walk builds the full picture.
    quads: HashMap<usize, Quad>,
    /// Tag centers (camera space) per output index, from the markers pass.
    tag_centers: HashMap<usize, [f32; 2]>,
    /// Overlap AABBs per canvas-adjacent pair, keyed `(before, after)`.
    overlaps: HashMap<(usize, usize), AdjacentOverlap>,
    /// Per-output photometric levels from the white pass (camera space).
    levels: HashMap<usize, Levels>,
    /// Remaining steps when driven by the `run` macro; empty = stepped mode.
    run_queue: VecDeque<RunStep>,
    /// Output filter carried through a `run` (None = all outputs).
    run_output: Option<usize>,
    /// Outputs whose measurement failed; `apply` leaves their config alone.
    failed: Vec<usize>,
}

impl AutoBlend {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Idle,
            capture: None,
            pending: None,
            quads: HashMap::new(),
            tag_centers: HashMap::new(),
            overlaps: HashMap::new(),
            levels: HashMap::new(),
            run_queue: VecDeque::new(),
            run_output: None,
            failed: Vec::new(),
        }
    }

    /// Whether a pass is waiting on its settle timer (cheap gate for the
    /// per-tick call, so idle ticks never touch the shared state lock).
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// `/qplayer/projection/autoblend/stream <url>` — (re)start the camera.
    pub(crate) fn stream(&mut self, url: &str) {
        self.capture = None; // Drop stops the old decode thread.
        self.pending = None;
        match StreamCapture::start(url) {
            Ok(capture) => {
                self.capture = Some(capture);
                self.phase = Phase::Streaming;
                log::info!("[autoblend] streaming {url}");
            }
            Err(error) => {
                self.phase = Phase::Idle;
                log::warn!("[autoblend] stream {url} failed: {error:#}");
            }
        }
    }

    /// `/qplayer/projection/autoblend/markers [index]` — show AprilTag `i` on
    /// output `i` (all outputs, or one for a per-output walk) and measure tag
    /// centers after the settle timer.
    pub(crate) fn markers(
        &mut self,
        output: Option<usize>,
        projection: &ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        let Some(targets) = self.targets(output, projection) else {
            return;
        };
        let Some(frame) = marker_frame(projection, &targets) else {
            return;
        };
        let _ = canvas_cmd_tx.send(CanvasCommand::BlankCanvas);
        let _ = canvas_cmd_tx.send(CanvasCommand::Overlay(Some((frame, CanvasFit::Stretch))));
        self.begin_pass(PassKind::Markers, targets);
    }

    /// `/qplayer/projection/autoblend/colors [index]` — show `PALETTE[i]` on
    /// each target output via its live `test_pattern` and measure the
    /// projected quads (+ overlaps) after the settle timer.
    pub(crate) fn colors(
        &mut self,
        output: Option<usize>,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        let Some(targets) = self.targets(output, projection) else {
            return;
        };
        for &i in &targets {
            let [r, g, b] = PALETTE[i % PALETTE.len()];
            projection.outputs[i].test_pattern = TestPattern::Color { r, g, b };
        }
        // A leftover marker overlay would corrupt the color regions.
        let _ = canvas_cmd_tx.send(CanvasCommand::Overlay(None));
        self.begin_pass(PassKind::Colors, targets);
    }

    /// `/qplayer/projection/autoblend/white [index]` — photometric pass:
    /// white → gray → black on each target output, sampling each output's
    /// solo region in camera space. Feeds the gamma fit + black uplift on
    /// `apply`. Needs measured quads (run the colors pass first).
    pub(crate) fn white(&mut self, output: Option<usize>, projection: &mut ProjectionConfig) {
        let Some(targets) = self.targets(output, projection) else {
            return;
        };
        let targets: Vec<usize> = targets
            .into_iter()
            .filter(|i| {
                let measured = self.quads.contains_key(i);
                if !measured {
                    log::warn!("[autoblend] output {i}: no measured quad — run colors first");
                }
                measured
            })
            .collect();
        if targets.is_empty() {
            log::warn!("[autoblend] white pass: nothing measured yet");
            return;
        }
        set_level_pattern(projection, &targets, 0);
        self.begin_pass(PassKind::White, targets);
    }

    /// `/qplayer/projection/autoblend/run <url> [index]` — the full sequence:
    /// stream, markers, colors, white, apply. Each step starts when the
    /// previous one lands (see `tick`).
    pub(crate) fn run(
        &mut self,
        url: &str,
        output: Option<usize>,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        self.stream(url);
        if self.capture.is_none() {
            return; // stream() already logged the failure
        }
        self.run_queue = VecDeque::from([
            RunStep::Markers,
            RunStep::Colors,
            RunStep::White,
            RunStep::Apply,
        ]);
        self.run_output = output;
        self.advance_run(projection, canvas_cmd_tx);
    }

    /// Start the next queued `run` step, if any.
    fn advance_run(
        &mut self,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        let Some(step) = self.run_queue.pop_front() else {
            return;
        };
        let output = self.run_output;
        log::info!("[autoblend] run: {step:?}");
        match step {
            RunStep::Markers => self.markers(output, projection, canvas_cmd_tx),
            RunStep::Colors => self.colors(output, projection, canvas_cmd_tx),
            RunStep::White => self.white(output, projection),
            RunStep::Apply => self.apply(projection, canvas_cmd_tx),
        }
    }

    /// `/qplayer/projection/autoblend/apply` — compute warp + edge blends +
    /// photometry from the measurements, write them into the live config, and
    /// clean up.
    pub(crate) fn apply(
        &mut self,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        self.pending = None;
        if self.quads.is_empty() {
            log::warn!("[autoblend] apply with no measured outputs — nothing written");
        } else {
            let targets = target_quads(projection, &self.quads);
            for (&i, measured) in &self.quads {
                let (Some(target), Some(output)) = (targets.get(&i), projection.outputs.get_mut(i))
                else {
                    continue;
                };
                output.warp = compute_warp(measured, target);
                log::info!("[autoblend] output {i} warp: {:?}", output.warp.0);
            }
            for (&(i, j), overlap) in &self.overlaps {
                let (Some(mi), Some(mj)) = (self.quads.get(&i), self.quads.get(&j)) else {
                    continue;
                };
                let Some((before, after)) = blend_pair(&mut projection.outputs, i, j) else {
                    continue;
                };
                match overlap.region {
                    Some(region) if overlap.vertical => {
                        before.edge_blend.right = blend_edge(mi, region, true, before.output_width);
                        after.edge_blend.left = blend_edge(mj, region, true, after.output_width);
                    }
                    Some(region) => {
                        before.edge_blend.bottom =
                            blend_edge(mi, region, false, before.output_height);
                        after.edge_blend.top = blend_edge(mj, region, false, after.output_height);
                    }
                    None => {
                        let (a, b) = facing_edges_mut(before, after, overlap.vertical);
                        *a = EdgeBlendEdge::default();
                        *b = EdgeBlendEdge::default();
                    }
                }
            }
            // Photometry: gamma equalization + black uplift from the
            // white/gray/black levels.
            let gammas = fit_gammas(&self.levels);
            for &i in self.levels.keys() {
                let Some(output) = projection.outputs.get_mut(i) else {
                    continue;
                };
                if let Some(gamma) = gammas.get(&i) {
                    output.gamma = *gamma;
                    log::info!(
                        "[autoblend] output {i} gamma: ({:.2}, {:.2}, {:.2})",
                        gamma.r,
                        gamma.g,
                        gamma.b
                    );
                }
                if let Some(uplift) = black_uplift_for(&self.levels, &self.overlaps, i) {
                    output.black_uplift = uplift;
                    log::info!("[autoblend] output {i} black uplift: {uplift:.4}");
                }
            }
            log::info!(
                "[autoblend] applied: {} warped, {} overlap pairs, {} photometered, {} failed",
                self.quads.len(),
                self.overlaps.len(),
                self.levels.len(),
                self.failed.len()
            );
        }
        self.finish(projection, canvas_cmd_tx);
    }

    /// `/qplayer/projection/autoblend/abort` — discard everything and clean up.
    pub(crate) fn abort(
        &mut self,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        log::info!("[autoblend] aborted (was {:?})", self.phase);
        self.finish(projection, canvas_cmd_tx);
    }

    /// Advance a pending pass whose settle timer elapsed. Called from the App
    /// tick (gated by `has_pending`); takes the live projection config mutably
    /// so the colors/white passes can re-pattern the outputs, and the canvas
    /// command sender so a `run` can chain into the next step.
    pub(crate) fn tick(
        &mut self,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        let now = Instant::now();
        let Some(pending) = &self.pending else {
            return;
        };
        if now < pending.not_before {
            return;
        }
        let Some(capture) = &self.capture else {
            log::warn!("[autoblend] camera stream gone mid-pass");
            self.fail_pending();
            return;
        };
        let Some(frame) = capture.latest_frame() else {
            let pending = self.pending.as_mut().expect("pending checked above");
            pending.frame_retries += 1;
            if pending.frame_retries > MAX_FRAME_RETRIES {
                log::warn!("[autoblend] no camera frame after {MAX_FRAME_RETRIES} retries");
                self.fail_pending();
            } else {
                pending.not_before = now + FRAME_RETRY;
            }
            return;
        };
        let pending = self.pending.take().expect("pending checked above");
        match pending.kind {
            PassKind::Markers => {
                self.detect_markers(frame, &pending.targets);
                self.phase = Phase::Ready;
            }
            PassKind::Colors => {
                self.detect_colors(&frame, &pending.targets, projection);
                self.phase = Phase::Ready;
            }
            PassKind::White => {
                self.detect_levels(&frame, &pending.targets, pending.step);
                if pending.step < 2 {
                    // Next level of the white → gray → black series.
                    set_level_pattern(projection, &pending.targets, pending.step + 1);
                    self.pending = Some(PendingPass {
                        not_before: Instant::now() + SETTLE,
                        step: pending.step + 1,
                        ..pending
                    });
                    return;
                }
                self.phase = Phase::Ready;
            }
        }
        // A completed pass kicks the next step of a `run`.
        self.advance_run(projection, canvas_cmd_tx);
    }

    /// One photometric capture: sample each target's solo region (the central
    /// 40% of its measured quad — away from the blend zones) and store the
    /// level for this step (0=white, 1=gray, 2=black).
    ///
    /// ponytail: the central region assumes the blend zones stay near the
    /// edges; an overlap covering the middle of an output contaminates the
    /// sample. Upgrade path: subtract the measured overlap AABBs.
    fn detect_levels(&mut self, frame: &image::RgbaImage, targets: &[usize], step: usize) {
        for &i in targets {
            let Some(quad) = self.quads.get(&i) else {
                continue;
            };
            let (min, max) = quad_aabb(quad);
            let cx = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
            let half = [(max[0] - min[0]) * 0.2, (max[1] - min[1]) * 0.2];
            let color = average_color(
                frame,
                [cx[0] - half[0], cx[1] - half[1]],
                [cx[0] + half[0], cx[1] + half[1]],
            );
            let levels = self.levels.entry(i).or_default();
            match step {
                0 => levels.white = Some(color),
                1 => levels.gray = Some(color),
                _ => levels.black = Some(color),
            }
        }
    }

    /// Resolve the target output indices; requires an active capture.
    fn targets(
        &mut self,
        output: Option<usize>,
        projection: &ProjectionConfig,
    ) -> Option<Vec<usize>> {
        if self.capture.is_none() {
            log::warn!("[autoblend] no camera stream — send autoblend/stream first");
            return None;
        }
        let targets = match output {
            Some(i) if i < projection.outputs.len() => vec![i],
            Some(i) => {
                log::warn!(
                    "[autoblend] output index {i} out of range ({} outputs)",
                    projection.outputs.len()
                );
                return None;
            }
            None => (0..projection.outputs.len()).collect(),
        };
        if targets.is_empty() {
            log::warn!("[autoblend] no projection outputs configured");
            return None;
        }
        Some(targets)
    }

    fn begin_pass(&mut self, kind: PassKind, targets: Vec<usize>) {
        self.phase = match kind {
            PassKind::Markers => Phase::Markers,
            PassKind::Colors => Phase::Colors,
            PassKind::White => Phase::White,
        };
        log::info!("[autoblend] {kind:?} pass on outputs {targets:?}");
        self.pending = Some(PendingPass {
            kind,
            targets,
            not_before: Instant::now() + SETTLE,
            frame_retries: 0,
            step: 0,
        });
    }

    /// Mark the pending pass's targets failed and drop the pass. A failed
    /// pass also stops a `run`: chaining on bad measurements is worse than
    /// stopping early.
    fn fail_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            for i in pending.targets {
                if !self.failed.contains(&i) {
                    self.failed.push(i);
                }
            }
        }
        self.run_queue.clear();
        self.phase = Phase::Ready;
    }

    /// Markers pass capture: tag id = output index → camera-space center.
    fn detect_markers(&mut self, frame: image::RgbaImage, targets: &[usize]) {
        let gray = image::DynamicImage::ImageRgba8(frame).into_luma8();
        let mut detector = AprilTagDetector::default();
        let detections = detector.detect(&gray);
        for &i in targets {
            match detections.iter().find(|d| d.id == i as u32) {
                Some(d) => {
                    self.tag_centers.insert(i, d.center);
                }
                None => {
                    if !self.failed.contains(&i) {
                        self.failed.push(i);
                    }
                    log::warn!("[autoblend] output {i}: tag {i} not found in camera frame");
                }
            }
        }
    }

    /// Colors pass capture: palette color → measured quad per target, plus
    /// overlap AABBs for canvas-adjacent measured pairs.
    fn detect_colors(
        &mut self,
        frame: &image::RgbaImage,
        targets: &[usize],
        projection: &ProjectionConfig,
    ) {
        for &i in targets {
            match find_color_quad(frame, PALETTE[i % PALETTE.len()], COLOR_TOLERANCE) {
                Some(quad) => {
                    if let Some(center) = self.tag_centers.get(&i)
                        && !point_in_quad(*center, &quad)
                    {
                        log::warn!(
                            "[autoblend] output {i}: tag center {center:?} lies outside the \
                             measured color quad — geometry still trusted"
                        );
                    }
                    self.quads.insert(i, quad);
                }
                None => {
                    if !self.failed.contains(&i) {
                        self.failed.push(i);
                    }
                    log::warn!("[autoblend] output {i}: color region not found in camera frame");
                }
            }
        }
        for (i, j, vertical) in adjacent_pairs(&projection.outputs) {
            if !(targets.contains(&i) && targets.contains(&j)) {
                continue;
            }
            if !(self.quads.contains_key(&i) && self.quads.contains_key(&j)) {
                continue;
            }
            // ponytail: the additive mix of two palette colors can itself be a
            // palette color (red + green = yellow), so a third output showing
            // that color pollutes the overlap AABB. Ceiling noted in detect.rs.
            let region = measure_overlap(
                frame,
                PALETTE[i % PALETTE.len()],
                PALETTE[j % PALETTE.len()],
                COLOR_TOLERANCE,
            );
            self.overlaps
                .insert((i, j), AdjacentOverlap { vertical, region });
        }
    }

    /// Shared cleanup for apply/abort: patterns off, overlay cleared, capture
    /// stopped, measurements discarded, back to Idle.
    fn finish(
        &mut self,
        projection: &mut ProjectionConfig,
        canvas_cmd_tx: &std::sync::mpsc::Sender<CanvasCommand>,
    ) {
        for output in &mut projection.outputs {
            output.test_pattern = TestPattern::Off;
        }
        let _ = canvas_cmd_tx.send(CanvasCommand::Overlay(None));
        self.capture = None;
        self.pending = None;
        self.phase = Phase::Idle;
        self.quads.clear();
        self.tag_centers.clear();
        self.overlaps.clear();
        self.levels.clear();
        self.run_queue.clear();
        self.run_output = None;
        self.failed.clear();
    }
}

/// Set the flat pattern for one photometric level on the target outputs:
/// 0 = white, 1 = 50% gray, 2 = black.
fn set_level_pattern(projection: &mut ProjectionConfig, targets: &[usize], step: usize) {
    let pattern = match step {
        0 => TestPattern::White,
        1 => TestPattern::Color {
            r: 128,
            g: 128,
            b: 128,
        },
        _ => TestPattern::Black,
    };
    for &i in targets {
        projection.outputs[i].test_pattern = pattern;
    }
}

/// Axis-aligned bounding box of a camera-space quad.
fn quad_aabb(quad: &Quad) -> ([f32; 2], [f32; 2]) {
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for p in quad {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    (min, max)
}

/// Native response exponent from the three levels: e = ln((G−B)/(W−B))/ln(0.5).
/// None when the measurement can't support a fit (crushed black, no contrast).
fn native_exponent(black: f32, gray: f32, white: f32) -> Option<f32> {
    let range = white - black;
    let mid = gray - black;
    if range < 1e-3 || mid <= 0.0 {
        return None;
    }
    let ratio = mid / range;
    if !(0.0..1.0).contains(&ratio) {
        return None;
    }
    Some(ratio.ln() / 0.5f32.ln())
}

/// Per-output gamma equalization: each output's gamma scales its native
/// exponent onto the median exponent of the rig, per channel. Outputs with an
/// unmeasurable channel keep gamma 1.0 there.
fn fit_gammas(levels: &HashMap<usize, Levels>) -> HashMap<usize, cuepool_core::OutputGamma> {
    // Median native exponent per channel across the outputs that measured.
    let mut exponents: [Vec<f32>; 3] = Default::default();
    for l in levels.values() {
        let (Some(w), Some(g), Some(b)) = (l.white, l.gray, l.black) else {
            continue;
        };
        for ch in 0..3 {
            if let Some(e) = native_exponent(b[ch], g[ch], w[ch]) {
                exponents[ch].push(e);
            }
        }
    }
    let mut target = [1.0f32; 3];
    for ch in 0..3 {
        let v = &mut exponents[ch];
        if !v.is_empty() {
            v.sort_by(f32::total_cmp);
            target[ch] = v[v.len() / 2];
        }
    }
    levels
        .iter()
        .filter_map(|(&i, l)| {
            let (Some(w), Some(g), Some(b)) = (l.white, l.gray, l.black) else {
                return None;
            };
            let mut gamma = [1.0f32; 3];
            for ch in 0..3 {
                if let Some(e) = native_exponent(b[ch], g[ch], w[ch]) {
                    // Clamp to a sane correction band: beyond this the
                    // measurement is noise, not response.
                    gamma[ch] = (target[ch] / e).clamp(0.5, 3.0);
                }
            }
            Some((
                i,
                cuepool_core::OutputGamma {
                    r: gamma[0],
                    g: gamma[1],
                    b: gamma[2],
                },
            ))
        })
        .collect()
}

/// Black uplift for output `i`: the brightest neighbor black floor across its
/// measured overlap pairs, minus its own, normalized by its white range.
/// Matches the solo-region black to the overlap zone's additive leakage.
fn black_uplift_for(
    levels: &HashMap<usize, Levels>,
    overlaps: &HashMap<(usize, usize), AdjacentOverlap>,
    i: usize,
) -> Option<f32> {
    let own = levels.get(&i)?;
    let (Some(own_black), Some(own_white)) = (own.black, own.white) else {
        return None;
    };
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let range = luma(own_white) - luma(own_black);
    if range < 1e-3 {
        return None;
    }
    let neighbor_black = overlaps
        .iter()
        .filter(|((a, b), o)| o.region.is_some() && (*a == i || *b == i))
        .filter_map(|(&(a, b), _)| levels.get(if a == i { &b } else { &a }))
        .filter_map(|l| l.black)
        .map(luma)
        .fold(0.0f32, f32::max);
    if neighbor_black <= 0.0 {
        return None;
    }
    Some(((neighbor_black - luma(own_black)).max(0.0) / range).clamp(0.0, 0.2))
}

/// Mutable access to outputs `i` and `j` at once (`i != j`).
fn blend_pair(
    outputs: &mut [ProjectorOutput],
    i: usize,
    j: usize,
) -> Option<(&mut ProjectorOutput, &mut ProjectorOutput)> {
    if i == j || i >= outputs.len() || j >= outputs.len() {
        return None;
    }
    let (lo, hi, swapped) = if i < j { (i, j, false) } else { (j, i, true) };
    let (a, b) = outputs.split_at_mut(hi);
    let (first, second) = (&mut a[lo], &mut b[0]);
    Some(if swapped {
        (second, first)
    } else {
        (first, second)
    })
}

/// The facing edges of a canvas-adjacent pair (`before` left of/above `after`).
fn facing_edges_mut<'a>(
    before: &'a mut ProjectorOutput,
    after: &'a mut ProjectorOutput,
    vertical: bool,
) -> (&'a mut EdgeBlendEdge, &'a mut EdgeBlendEdge) {
    if vertical {
        (&mut before.edge_blend.right, &mut after.edge_blend.left)
    } else {
        (&mut before.edge_blend.bottom, &mut after.edge_blend.top)
    }
}

/// Canvas-adjacent output pairs: source rects sharing an edge with overlapping
/// spans. Returns `(before, after, vertical)` — for a vertical shared edge
/// `before` is left of `after`; for a horizontal one `before` is above.
fn adjacent_pairs(outputs: &[ProjectorOutput]) -> Vec<(usize, usize, bool)> {
    let mut pairs = Vec::new();
    for i in 0..outputs.len() {
        for j in (i + 1)..outputs.len() {
            let (a, b) = (&outputs[i], &outputs[j]);
            let spans_overlap =
                |a0: u32, a_len: u32, b0: u32, b_len: u32| a0 < b0 + b_len && b0 < a0 + a_len;
            if spans_overlap(a.source_y, a.source_height, b.source_y, b.source_height) {
                if a.source_x + a.source_width == b.source_x {
                    pairs.push((i, j, true));
                } else if b.source_x + b.source_width == a.source_x {
                    pairs.push((j, i, true));
                }
            }
            if spans_overlap(a.source_x, a.source_width, b.source_x, b.source_width) {
                if a.source_y + a.source_height == b.source_y {
                    pairs.push((i, j, false));
                } else if b.source_y + b.source_height == a.source_y {
                    pairs.push((j, i, false));
                }
            }
        }
    }
    pairs
}

/// Fit the full canvas rect into the union bbox of the measured quads (uniform
/// scale, centered), then map each measured output's canvas source rect
/// through that transform — its axis-aligned target quad in camera space.
fn target_quads(
    projection: &ProjectionConfig,
    quads: &HashMap<usize, Quad>,
) -> HashMap<usize, Quad> {
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for quad in quads.values() {
        for p in quad {
            min = [min[0].min(p[0]), min[1].min(p[1])];
            max = [max[0].max(p[0]), max[1].max(p[1])];
        }
    }
    let bbox = [max[0] - min[0], max[1] - min[1]];
    let canvas = [
        projection.canvas_width as f32,
        projection.canvas_height as f32,
    ];
    if bbox[0] <= 0.0 || bbox[1] <= 0.0 || canvas[0] <= 0.0 || canvas[1] <= 0.0 {
        return HashMap::new();
    }
    let s = (bbox[0] / canvas[0]).min(bbox[1] / canvas[1]);
    let origin = [
        min[0] + (bbox[0] - canvas[0] * s) / 2.0,
        min[1] + (bbox[1] - canvas[1] * s) / 2.0,
    ];
    quads
        .keys()
        .filter_map(|&i| {
            let o = projection.outputs.get(i)?;
            let x0 = origin[0] + o.source_x as f32 * s;
            let y0 = origin[1] + o.source_y as f32 * s;
            let x1 = x0 + o.source_width as f32 * s;
            let y1 = y0 + o.source_height as f32 * s;
            Some((i, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]))
        })
        .collect()
}

/// Corner-pin warp for one output. The shader's warp corners are the forward
/// map unit→corners (see `warp_matrix_rows`): content point u is displayed at
/// output UV F(u) and lands at camera point H_p(F(u)). Requiring the landing
/// to equal the target T(u) gives F = H_p⁻¹·H_t; the stored corners are F
/// evaluated at the unit corners.
fn compute_warp(measured: &Quad, target: &Quad) -> WarpCorners {
    let h_p = compute_forward_homography(&UNIT_SQUARE, measured);
    let h_t = compute_forward_homography(&UNIT_SQUARE, target);
    let f = mul_3x3(invert_3x3(h_p), h_t);
    WarpCorners(UNIT_SQUARE.map(|u| apply_3x3(f, u)))
}

/// Overlap width as an `EdgeBlendEdge`: the overlap AABB (camera space) mapped
/// through the camera→output-UV homography, as a fraction of the output.
///
/// ponytail: sampled at the AABB midline — on a keystoned rig the true overlap
/// width varies along the edge. Upgrade path: sample both ends, take the max.
fn blend_edge(
    measured: &Quad,
    region: OverlapRegion,
    vertical: bool,
    output_size: u32,
) -> EdgeBlendEdge {
    let cam_to_uv = compute_forward_homography(measured, &UNIT_SQUARE);
    let (axis, cross) = if vertical { (0, 1) } else { (1, 0) };
    let mid = (region.min[cross] + region.max[cross]) / 2.0;
    let point = |a: f32| {
        let mut p = [0.0; 2];
        p[axis] = a;
        p[cross] = mid;
        apply_3x3(cam_to_uv, p)[axis]
    };
    let frac = (point(region.max[axis]) - point(region.min[axis])).abs();
    let width = (frac * output_size as f32).round() as u32;
    EdgeBlendEdge {
        enabled: width > 0,
        width,
        gamma: 2.0,
    }
}

/// Whether `p` lies inside the convex quad (TL/TR/BR/BL): the cross-product
/// sign against each edge must be consistent.
fn point_in_quad(p: [f32; 2], quad: &Quad) -> bool {
    let mut sign = 0.0f32;
    for k in 0..4 {
        let a = quad[k];
        let b = quad[(k + 1) % 4];
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Canvas-sized transparent RGBA frame with one white-backed AprilTag centered
/// in each target output's canvas source rect (tag id = output index). The
/// projection shader alpha-composites it over the blanked canvas.
fn marker_frame(projection: &ProjectionConfig, targets: &[usize]) -> Option<VideoFrame> {
    let (w, h) = (projection.canvas_width, projection.canvas_height);
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for &i in targets {
        let o = &projection.outputs[i];
        let side = (o.source_width.min(o.source_height) as f32 * TAG_FILL) as u32;
        if side < 16 {
            log::warn!("[autoblend] output {i}: source rect too small for a marker");
            continue;
        }
        let marker = match generate_marker(Default::default(), i as u32) {
            Ok(marker) => marker,
            Err(error) => {
                log::warn!("[autoblend] output {i}: marker generation failed: {error:#}");
                continue;
            }
        };
        // White quiet zone around the tag: a tenth of the side per edge.
        let quiet = (side / 10).max(1);
        let inner = side - 2 * quiet;
        let tag =
            image::imageops::resize(&marker, inner, inner, image::imageops::FilterType::Nearest);
        let x0 = (o.source_x + o.source_width / 2).saturating_sub(side / 2);
        let y0 = (o.source_y + o.source_height / 2).saturating_sub(side / 2);
        let put = |x: u32, y: u32, px: [u8; 4], rgba: &mut [u8]| {
            if x < w && y < h {
                let at = ((y * w + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&px);
            }
        };
        // Opaque white backing square (canvas shows through elsewhere).
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                put(x, y, [255, 255, 255, 255], &mut rgba);
            }
        }
        // Tag cells on top of the backing (black and white both opaque).
        for (x, y, p) in tag.enumerate_pixels() {
            let v = p[0];
            put(x0 + quiet + x, y0 + quiet + y, [v, v, v, 255], &mut rgba);
        }
    }
    Some(VideoFrame::new(w, h, rgba, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(x: u32, y: u32, w: u32, h: u32) -> ProjectorOutput {
        ProjectorOutput {
            source_x: x,
            source_y: y,
            source_width: w,
            source_height: h,
            output_width: w,
            output_height: h,
            ..ProjectorOutput::default_single()
        }
    }

    /// Two 1920×1080 outputs side by side on a 3840×1080 canvas.
    fn two_output_config() -> ProjectionConfig {
        ProjectionConfig {
            canvas_width: 3840,
            canvas_height: 1080,
            fit: CanvasFit::Fit,
            outputs: vec![output(0, 0, 1920, 1080), output(1920, 0, 1920, 1080)],
        }
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Quad {
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    fn autoblend_with(quads: &[(usize, Quad)]) -> AutoBlend {
        let mut ab = AutoBlend::new();
        ab.quads = quads.iter().cloned().collect();
        ab
    }

    fn dummy_tx() -> (
        std::sync::mpsc::Sender<CanvasCommand>,
        std::sync::mpsc::Receiver<CanvasCommand>,
    ) {
        std::sync::mpsc::channel()
    }

    /// Unwarped content corner u must project (through the measured quad's
    /// output-UV → camera homography) onto the target rect corner.
    fn assert_corners_project_to_target(measured: &Quad, target: &Quad, warp: &WarpCorners) {
        let h_p = compute_forward_homography(&UNIT_SQUARE, measured);
        let f = compute_forward_homography(&UNIT_SQUARE, &warp.0);
        for (u, t) in UNIT_SQUARE.iter().zip(target.iter()) {
            let got = apply_3x3(h_p, apply_3x3(f, *u));
            assert!(
                (got[0] - t[0]).abs() < 0.5 && (got[1] - t[1]).abs() < 0.5,
                "corner {u:?} projected to {got:?}, expected {t:?}"
            );
        }
    }

    #[test]
    fn aligned_wall_warps_identity_and_edges_disabled_without_overlap() {
        // Camera sees the wall at 0.1 px scale, outputs abutting exactly.
        let mut config = two_output_config();
        let m0 = rect(100.0, 50.0, 292.0, 158.0);
        let m1 = rect(292.0, 50.0, 484.0, 158.0);
        let mut ab = autoblend_with(&[(0, m0), (1, m1)]);
        ab.overlaps.insert(
            (0, 1),
            AdjacentOverlap {
                vertical: true,
                region: None,
            },
        );

        let (tx, _rx) = dummy_tx();
        ab.apply(&mut config, &tx);

        assert!(
            config.outputs[0].warp.is_identity(),
            "warp {:?}",
            config.outputs[0].warp.0
        );
        assert!(config.outputs[1].warp.is_identity());
        assert!(!config.outputs[0].edge_blend.right.enabled);
        assert!(!config.outputs[1].edge_blend.left.enabled);
        assert_eq!(config.outputs[0].edge_blend.right.width, 0);
    }

    #[test]
    fn measured_overlap_sets_facing_edge_widths() {
        // Same wall, but the projectors overlap 20 camera px (of 192 per output).
        let mut config = two_output_config();
        let m0 = rect(100.0, 50.0, 292.0, 158.0);
        let m1 = rect(272.0, 50.0, 464.0, 158.0);
        let mut ab = autoblend_with(&[(0, m0), (1, m1)]);
        ab.overlaps.insert(
            (0, 1),
            AdjacentOverlap {
                vertical: true,
                region: Some(OverlapRegion {
                    min: [272.0, 50.0],
                    max: [292.0, 158.0],
                    pixel_count: 20 * 108,
                }),
            },
        );

        let (tx, _rx) = dummy_tx();
        ab.apply(&mut config, &tx);

        // 20/192 of the 1920-px output width.
        assert_eq!(config.outputs[0].edge_blend.right.width, 200);
        assert!(config.outputs[0].edge_blend.right.enabled);
        assert_eq!(config.outputs[0].edge_blend.right.gamma, 2.0);
        assert_eq!(config.outputs[1].edge_blend.left.width, 200);
        assert!(config.outputs[1].edge_blend.left.enabled);
        // Non-facing edges untouched.
        assert!(!config.outputs[0].edge_blend.left.enabled);
        assert!(!config.outputs[1].edge_blend.right.enabled);
    }

    #[test]
    fn keystoned_quad_warps_onto_target_rect() {
        let mut config = two_output_config();
        // Output 0 keystoned (trapezoid), output 1 aligned.
        let m0 = [[110.0, 60.0], [292.0, 50.0], [292.0, 158.0], [100.0, 148.0]];
        let m1 = rect(292.0, 50.0, 484.0, 158.0);
        let mut ab = autoblend_with(&[(0, m0), (1, m1)]);

        let (tx, _rx) = dummy_tx();
        ab.apply(&mut config, &tx);

        assert!(!config.outputs[0].warp.is_identity());
        // `apply` consumed the measurements; recompute targets from the copies.
        let quads: HashMap<usize, Quad> = [(0, m0), (1, m1)].into_iter().collect();
        let targets = target_quads(&config, &quads);
        assert_corners_project_to_target(&m0, &targets[&0], &config.outputs[0].warp);
        assert_corners_project_to_target(&m1, &targets[&1], &config.outputs[1].warp);
    }

    #[test]
    fn failed_output_keeps_previous_config() {
        let mut config = two_output_config();
        config.outputs[1].warp = WarpCorners([[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]]);
        config.outputs[1].edge_blend.left = EdgeBlendEdge {
            enabled: true,
            width: 42,
            gamma: 2.2,
        };
        config.outputs[0].test_pattern = TestPattern::Color { r: 255, g: 0, b: 0 };
        // Only output 0 measured; output 1 failed its pass.
        let m0 = rect(100.0, 50.0, 292.0, 158.0);
        let mut ab = autoblend_with(&[(0, m0)]);

        let (tx, rx) = dummy_tx();
        ab.apply(&mut config, &tx);

        assert!(
            !config.outputs[0].warp.is_identity(),
            "one of two outputs measured: canvas-fit target is scaled, not identity"
        );
        let quads: HashMap<usize, Quad> = [(0, m0)].into_iter().collect();
        let targets = target_quads(&config, &quads);
        assert_corners_project_to_target(&m0, &targets[&0], &config.outputs[0].warp);
        assert_eq!(
            config.outputs[1].warp.0,
            [[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]],
            "failed output's warp untouched"
        );
        assert_eq!(config.outputs[1].edge_blend.left.width, 42);
        // Cleanup: all patterns off, overlay cleared, back to Idle.
        assert_eq!(config.outputs[0].test_pattern, TestPattern::Off);
        assert!(matches!(rx.try_recv(), Ok(CanvasCommand::Overlay(None))));
        assert_eq!(ab.phase, Phase::Idle);
        assert!(ab.quads.is_empty());
    }

    #[test]
    fn adjacent_pairs_finds_shared_edges() {
        let side_by_side = two_output_config();
        assert_eq!(adjacent_pairs(&side_by_side.outputs), vec![(0, 1, true)]);

        let mut stacked = two_output_config();
        stacked.outputs[1].source_x = 0;
        stacked.outputs[1].source_y = 1080;
        stacked.canvas_height = 2160;
        assert_eq!(adjacent_pairs(&stacked.outputs), vec![(0, 1, false)]);

        // Gap between the outputs: no shared edge.
        let mut gapped = two_output_config();
        gapped.outputs[1].source_x = 2000;
        assert!(adjacent_pairs(&gapped.outputs).is_empty());
    }

    #[test]
    fn point_in_quad_handles_trapezoid() {
        let quad = [[110.0, 60.0], [292.0, 50.0], [292.0, 158.0], [100.0, 148.0]];
        assert!(point_in_quad([200.0, 100.0], &quad));
        assert!(!point_in_quad([105.0, 55.0], &quad), "outside the TL bevel");
        assert!(!point_in_quad([500.0, 100.0], &quad));
    }

    #[test]
    fn blend_edge_disabled_when_overlap_is_slim_to_zero() {
        let measured = rect(100.0, 50.0, 292.0, 158.0);
        let zero = OverlapRegion {
            min: [200.0, 50.0],
            max: [200.0, 158.0],
            pixel_count: 0,
        };
        let edge = blend_edge(&measured, zero, true, 1920);
        assert!(!edge.enabled);
        assert_eq!(edge.width, 0);
    }

    fn levels(white: f32, gray: f32, black: f32) -> Levels {
        Levels {
            white: Some([white; 3]),
            gray: Some([gray; 3]),
            black: Some([black; 3]),
        }
    }

    #[test]
    fn native_exponent_recovers_known_gamma() {
        // Ideal 2.0-response projector: gray (0.5 in) reads 0.25 of the range.
        let e = native_exponent(0.0, 0.25, 1.0).unwrap();
        assert!((e - 2.0).abs() < 1e-4, "e = {e}");
        // Crushed measurements don't fit.
        assert!(native_exponent(0.5, 0.5, 1.0).is_none());
        assert!(native_exponent(0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn fit_gammas_pulls_outputs_to_the_median_exponent() {
        let mut map = HashMap::new();
        // Output 0: exponent 2.0 (gray 0.25). Output 1: exponent 1.0 (gray 0.5).
        map.insert(0, levels(1.0, 0.25, 0.0));
        map.insert(1, levels(1.0, 0.5, 0.0));
        let gammas = fit_gammas(&map);
        let g0 = gammas[&0];
        let g1 = gammas[&1];
        // Median of {2.0, 1.0} (sorted, upper middle) is 2.0.
        assert!(
            (g0.r - 1.0).abs() < 1e-4,
            "output 0 already on target: {g0:?}"
        );
        assert!((g1.r - 2.0).abs() < 1e-4, "output 1 corrected 2x: {g1:?}");
    }

    #[test]
    fn fit_gammas_skips_unmeasurable_outputs() {
        let mut map = HashMap::new();
        map.insert(0, levels(1.0, 0.25, 0.0));
        map.insert(1, Levels::default()); // never measured
        let gammas = fit_gammas(&map);
        assert!(gammas.contains_key(&0));
        assert!(!gammas.contains_key(&1));
    }

    #[test]
    fn black_uplift_matches_the_brightest_neighbor_floor() {
        let mut map = HashMap::new();
        map.insert(0, levels(1.0, 0.25, 0.01));
        map.insert(1, levels(1.0, 0.25, 0.04));
        let mut overlaps = HashMap::new();
        overlaps.insert(
            (0, 1),
            AdjacentOverlap {
                vertical: true,
                region: Some(OverlapRegion {
                    min: [0.0, 0.0],
                    max: [10.0, 10.0],
                    pixel_count: 100,
                }),
            },
        );
        // Output 0 lifts toward output 1's brighter black floor.
        let up0 = black_uplift_for(&map, &overlaps, 0).unwrap();
        // Luma of pure gray = the gray value; range 0.99, delta 0.03.
        assert!((up0 - 0.03 / 0.99).abs() < 1e-3, "uplift {up0}");
        // Output 1 already has the brighter floor: no lift.
        assert_eq!(black_uplift_for(&map, &overlaps, 1), Some(0.0));
        // No measured overlap → no uplift.
        assert!(black_uplift_for(&map, &HashMap::new(), 0).is_none());
    }

    #[test]
    fn set_level_pattern_cycles_white_gray_black() {
        let mut config = two_output_config();
        set_level_pattern(&mut config, &[0, 1], 0);
        assert_eq!(config.outputs[0].test_pattern, TestPattern::White);
        set_level_pattern(&mut config, &[0, 1], 1);
        assert_eq!(
            config.outputs[1].test_pattern,
            TestPattern::Color {
                r: 128,
                g: 128,
                b: 128
            }
        );
        set_level_pattern(&mut config, &[0], 2);
        assert_eq!(config.outputs[0].test_pattern, TestPattern::Black);
        // Untargeted output untouched by the last call.
        assert_eq!(
            config.outputs[1].test_pattern,
            TestPattern::Color {
                r: 128,
                g: 128,
                b: 128
            }
        );
    }

    #[test]
    fn run_with_a_dead_stream_queues_nothing() {
        let mut config = two_output_config();
        let (tx, _rx) = dummy_tx();
        let mut ab = AutoBlend::new();
        ab.run("not a url", None, &mut config, &tx);
        assert!(
            ab.run_queue.is_empty(),
            "failed stream must not queue steps"
        );
        assert_eq!(ab.phase, Phase::Idle);
    }
}
