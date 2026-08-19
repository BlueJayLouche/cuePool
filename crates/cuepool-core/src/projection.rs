//! Projection mapping configuration — canvas + output slices + edge blend.
//!
//! This is intentionally minimal and self-contained: one canvas, many outputs,
//! each sampling a rectangle from the canvas and rendering to its own window.

use serde::{Deserialize, Serialize};

/// How a video frame is fit into the projection canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CanvasFit {
    /// Scale to fill the entire canvas (may distort aspect ratio).
    Stretch,
    /// Scale to fit while preserving aspect ratio, letterboxing/pillarboxing.
    #[default]
    Fit,
    /// Scale to cover the entire canvas preserving aspect ratio, center-cropping
    /// the overflow. Content spans the whole canvas (all projectors) with no bars.
    Fill,
}

/// Top-level projection state stored in a show file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionConfig {
    #[serde(default = "default_canvas_width")]
    pub canvas_width: u32,
    #[serde(default = "default_canvas_height")]
    pub canvas_height: u32,
    #[serde(default)]
    pub fit: CanvasFit,
    #[serde(default)]
    pub outputs: Vec<ProjectorOutput>,
}

impl ProjectionConfig {
    /// Default single-output config matching CuePool's historical 1920×1080 video window.
    pub fn default_single() -> Self {
        Self {
            canvas_width: 1920,
            canvas_height: 1080,
            fit: CanvasFit::Fit,
            outputs: vec![ProjectorOutput::default_single()],
        }
    }

    /// 3×1 edge-blend preset matching `testFiles/example_3x1_EdgeBlend.png`.
    pub fn preset_3x1_edgeblend() -> Self {
        Self {
            canvas_width: 5400,
            canvas_height: 1080,
            fit: CanvasFit::Fit,
            outputs: vec![
                ProjectorOutput {
                    name: "Projector 1".into(),
                    source_x: 0,
                    source_y: 0,
                    source_width: 1920,
                    source_height: 1080,
                    output_width: 1920,
                    output_height: 1080,
                    fullscreen_monitor: None,
                    monitor_id: None,
                    edge_blend: EdgeBlend {
                        right: EdgeBlendEdge {
                            enabled: true,
                            width: 180,
                            gamma: 2.0,
                        },
                        ..Default::default()
                    },
                    black_uplift: 0.0,
                    test_pattern: TestPattern::Off,
                    blend_enabled: true,
                    uplift_enabled: true,
                    warp: WarpCorners::default(),
                    gamma: OutputGamma::default(),
                },
                ProjectorOutput {
                    name: "Projector 2".into(),
                    source_x: 1740,
                    source_y: 0,
                    source_width: 1920,
                    source_height: 1080,
                    output_width: 1920,
                    output_height: 1080,
                    fullscreen_monitor: None,
                    monitor_id: None,
                    edge_blend: EdgeBlend {
                        left: EdgeBlendEdge {
                            enabled: true,
                            width: 180,
                            gamma: 2.0,
                        },
                        right: EdgeBlendEdge {
                            enabled: true,
                            width: 180,
                            gamma: 2.0,
                        },
                        ..Default::default()
                    },
                    black_uplift: 0.0,
                    test_pattern: TestPattern::Off,
                    blend_enabled: true,
                    uplift_enabled: true,
                    warp: WarpCorners::default(),
                    gamma: OutputGamma::default(),
                },
                ProjectorOutput {
                    name: "Projector 3".into(),
                    source_x: 3480,
                    source_y: 0,
                    source_width: 1920,
                    source_height: 1080,
                    output_width: 1920,
                    output_height: 1080,
                    fullscreen_monitor: None,
                    monitor_id: None,
                    edge_blend: EdgeBlend {
                        left: EdgeBlendEdge {
                            enabled: true,
                            width: 180,
                            gamma: 2.0,
                        },
                        ..Default::default()
                    },
                    black_uplift: 0.0,
                    test_pattern: TestPattern::Off,
                    blend_enabled: true,
                    uplift_enabled: true,
                    warp: WarpCorners::default(),
                    gamma: OutputGamma::default(),
                },
            ],
        }
    }
}

impl Default for ProjectionConfig {
    fn default() -> Self {
        Self::default_single()
    }
}

fn default_canvas_width() -> u32 {
    1920
}

fn default_canvas_height() -> u32 {
    1080
}

/// One projector output window sampling a rectangle from the canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectorOutput {
    #[serde(default = "default_name")]
    pub name: String,
    /// Source rectangle on the canvas, in pixels.
    #[serde(default)]
    pub source_x: u32,
    #[serde(default)]
    pub source_y: u32,
    #[serde(default = "default_1920")]
    pub source_width: u32,
    #[serde(default = "default_1080")]
    pub source_height: u32,
    /// Window/output resolution in pixels.
    #[serde(default = "default_1920")]
    pub output_width: u32,
    #[serde(default = "default_1080")]
    pub output_height: u32,
    /// Legacy monitor *index* for borderless fullscreen. Fragile across reboots —
    /// kept only as a fallback. `monitor_id` (recall by position) takes precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fullscreen_monitor: Option<usize>,
    /// Saved identity of the monitor this output goes fullscreen on. Recalled by
    /// virtual-desktop position so a fixed multi-projector wall survives reboots and
    /// projector warm-up reorder (identical projectors share name/resolution but sit
    /// at fixed, distinct positions). `None` = windowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_id: Option<MonitorId>,
    #[serde(default)]
    pub edge_blend: EdgeBlend,
    /// Black-level uplift (linear light) applied uniformly across the whole
    /// output, de-interlocked from the edge-blend ramps.
    #[serde(default)]
    pub black_uplift: f32,
    /// Flat-field calibration pattern replacing the canvas content.
    #[serde(default)]
    pub test_pattern: TestPattern,
    /// Master switch for the edge-blend ramps. Turn off when blending is
    /// handled in-projector — the calibrated edge values are kept, not lost.
    #[serde(default = "default_enabled")]
    pub blend_enabled: bool,
    /// Master switch for black-level uplift. Turn off when black-level
    /// compensation is handled in-projector.
    #[serde(default = "default_enabled")]
    pub uplift_enabled: bool,
    /// Corner-pin warp (normalized output UV corners, TL/TR/BR/BL). Identity
    /// until the auto-blend calibration measures this output's projected quad.
    #[serde(default)]
    pub warp: WarpCorners,
    /// Per-channel output gamma from the white-pass photometric calibration.
    #[serde(default)]
    pub gamma: OutputGamma,
}

/// Normalized corner-pin warp for one output: TL, TR, BR, BL in output UV
/// space. Identity means the output fills its window unwarped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpCorners(pub [[f32; 2]; 4]);

impl WarpCorners {
    pub const IDENTITY: Self = Self([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);

    pub fn is_identity(&self) -> bool {
        self.0
            .iter()
            .zip(Self::IDENTITY.0.iter())
            .all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6)
    }
}

impl Default for WarpCorners {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Per-channel output gamma, fitted from the white-pass camera measurement.
///
/// ponytail: parametric gamma, not a measured LUT — three floats ride the
/// existing uniform instead of a LUT texture. If a real rig's response curves
/// defeat the parametric fit, upgrade to a 256-entry per-channel 1D LUT.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OutputGamma {
    #[serde(default = "default_gamma_one")]
    pub r: f32,
    #[serde(default = "default_gamma_one")]
    pub g: f32,
    #[serde(default = "default_gamma_one")]
    pub b: f32,
}

impl Default for OutputGamma {
    fn default() -> Self {
        Self {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_gamma_one() -> f32 {
    1.0
}

/// Calibration pattern for a projector output: Black for black-uplift
/// calibration, White for edge-blend width/gamma calibration, and a discrete
/// Color per output for the camera-measured auto-blend geometry pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TestPattern {
    #[default]
    Off,
    Black,
    White,
    Color {
        r: u8,
        g: u8,
        b: u8,
    },
}

impl ProjectorOutput {
    pub fn default_single() -> Self {
        Self {
            name: default_name(),
            source_x: 0,
            source_y: 0,
            source_width: 1920,
            source_height: 1080,
            output_width: 1920,
            output_height: 1080,
            fullscreen_monitor: None,
            monitor_id: None,
            edge_blend: EdgeBlend::default(),
            black_uplift: 0.0,
            test_pattern: TestPattern::Off,
            blend_enabled: true,
            uplift_enabled: true,
            warp: WarpCorners::default(),
            gamma: OutputGamma::default(),
        }
    }
}

/// Stable-ish identity for a physical monitor, used to recall which output drives
/// which projector across reboots. Position in the virtual desktop is the reliable
/// key for a fixed wall: identical projectors share name/resolution but sit at
/// fixed, distinct positions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MonitorId {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub pos_x: i32,
    #[serde(default)]
    pub pos_y: i32,
}

impl MonitorId {
    /// Human-readable label for the assignment dropdown.
    pub fn label(&self) -> String {
        let name = if self.name.is_empty() {
            "Display"
        } else {
            self.name.as_str()
        };
        format!(
            "{name} — {}×{} @ ({},{})",
            self.width, self.height, self.pos_x, self.pos_y
        )
    }

    /// Squared virtual-desktop position distance to another monitor.
    pub fn pos_distance_sq(&self, other: &MonitorId) -> i64 {
        let dx = (self.pos_x - other.pos_x) as i64;
        let dy = (self.pos_y - other.pos_y) as i64;
        dx * dx + dy * dy
    }
}

/// Greedily assign each output's saved monitor to the nearest unused *available*
/// monitor by position, within `max_dist_sq`. Returns, per output, the index into
/// `available` (or `None` = leave windowed). Position-keyed so identical projectors
/// recall correctly even if the OS enumerates them in a different order, and a
/// missing projector leaves only its output windowed rather than scrambling others.
pub fn resolve_monitor_assignment(
    wanted: &[Option<MonitorId>],
    available: &[MonitorId],
    max_dist_sq: i64,
) -> Vec<Option<usize>> {
    let mut used = vec![false; available.len()];
    wanted
        .iter()
        .map(|want| {
            let want = want.as_ref()?;
            let mut best: Option<(usize, i64)> = None;
            for (i, mon) in available.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let d = mon.pos_distance_sq(want);
                if d <= max_dist_sq && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
            let (i, _) = best?;
            used[i] = true;
            Some(i)
        })
        .collect()
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    fn pj(x: i32) -> MonitorId {
        MonitorId {
            name: "PJ".into(),
            width: 1920,
            height: 1080,
            pos_x: x,
            pos_y: 0,
        }
    }

    #[test]
    fn identical_projectors_recall_by_position_despite_reorder() {
        let wanted = vec![Some(pj(0)), Some(pj(1920)), Some(pj(3840))];
        // OS enumerates them in a different order after a reboot.
        let available = vec![pj(3840), pj(0), pj(1920)];
        let got = resolve_monitor_assignment(&wanted, &available, 100 * 100);
        assert_eq!(got, vec![Some(1), Some(2), Some(0)]); // each output → its position
    }

    #[test]
    fn missing_projector_only_windows_its_own_output() {
        let wanted = vec![Some(pj(0)), Some(pj(1920)), Some(pj(3840))];
        let available = vec![pj(0), pj(3840)]; // centre projector not detected yet
        let got = resolve_monitor_assignment(&wanted, &available, 100 * 100);
        assert_eq!(got, vec![Some(0), None, Some(1)]);
    }
}

fn default_name() -> String {
    "Output".into()
}

fn default_1920() -> u32 {
    1920
}

fn default_1080() -> u32 {
    1080
}

/// Per-edge soft-edge blend parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct EdgeBlend {
    #[serde(default)]
    pub left: EdgeBlendEdge,
    #[serde(default)]
    pub right: EdgeBlendEdge,
    #[serde(default)]
    pub top: EdgeBlendEdge,
    #[serde(default)]
    pub bottom: EdgeBlendEdge,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeBlendEdge {
    #[serde(default)]
    pub enabled: bool,
    /// Blend width in pixels.
    #[serde(default)]
    pub width: u32,
    /// Blend gamma (1.0 = linear, higher = steeper fade).
    #[serde(default = "default_gamma")]
    pub gamma: f32,
}

impl Default for EdgeBlendEdge {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 0,
            gamma: default_gamma(),
        }
    }
}

fn default_gamma() -> f32 {
    2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = ProjectionConfig::default();
        assert_eq!(cfg.canvas_width, 1920);
        assert_eq!(cfg.canvas_height, 1080);
        assert_eq!(cfg.outputs.len(), 1);
    }

    #[test]
    fn test_3x1_preset() {
        let cfg = ProjectionConfig::preset_3x1_edgeblend();
        assert_eq!(cfg.canvas_width, 5400);
        assert_eq!(cfg.outputs.len(), 3);
        assert!(cfg.outputs[0].edge_blend.right.enabled);
        assert!(cfg.outputs[1].edge_blend.left.enabled);
        assert!(cfg.outputs[1].edge_blend.right.enabled);
        assert!(cfg.outputs[2].edge_blend.left.enabled);
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = ProjectionConfig::preset_3x1_edgeblend();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let de: ProjectionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, de);
    }

    /// A v10 file predates `warp`/`gamma`: both must land on their
    /// pass-through defaults so an uncalibrated show renders unchanged.
    #[test]
    fn test_v10_output_without_warp_or_gamma_loads_uncalibrated() {
        let legacy = r#"{"name":"Output","source_x":0,"source_y":0}"#;
        let out: ProjectorOutput = serde_json::from_str(legacy).unwrap();
        assert!(out.warp.is_identity());
        assert_eq!(out.gamma, OutputGamma::default());
        assert!(out.blend_enabled);
        assert!(out.uplift_enabled);
    }

    #[test]
    fn test_test_pattern_color_roundtrip() {
        let out = ProjectorOutput {
            test_pattern: TestPattern::Color {
                r: 255,
                g: 0,
                b: 128,
            },
            ..ProjectorOutput::default_single()
        };
        let json = serde_json::to_string(&out).unwrap();
        let de: ProjectorOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(out, de);
    }

    #[test]
    fn test_warp_identity_detection() {
        assert!(WarpCorners::default().is_identity());
        let warped = WarpCorners([[0.1, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        assert!(!warped.is_identity());
    }
}
