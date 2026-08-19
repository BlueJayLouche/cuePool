//! Headless marker + color detection for camera-based calibration.
//!
//! Pure functions over `image` types — no camera, no GPU. Ported from
//! `rustjay-projection`'s `videowall.rs` so CuePool stays self-contained.

use image::{GrayImage, RgbaImage};

/// Below this many matching pixels a region is treated as noise.
const MIN_REGION_PIXELS: usize = 100;

/// AprilTag families we support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AprilTagFamily {
    #[default]
    Tag36h11,
    Tag25h9,
    Tag16h5,
}

impl AprilTagFamily {
    fn to_family(self) -> apriltag::Family {
        match self {
            Self::Tag36h11 => apriltag::Family::tag_36h11(),
            Self::Tag25h9 => apriltag::Family::tag_25h9(),
            Self::Tag16h5 => apriltag::Family::tag_16h5(),
        }
    }
}

/// One detected marker. Corner order (AprilTag, image coords, Y down):
/// `[0]=left-bottom, [1]=right-bottom, [2]=right-top, [3]=left-top`.
#[derive(Debug, Clone)]
pub struct AprilTagDetection {
    pub id: u32,
    pub corners: [[f32; 2]; 4],
    pub center: [f32; 2],
    pub decision_margin: f32,
}

/// Detector wrapper around the `apriltag` crate.
pub struct AprilTagDetector {
    detector: apriltag::Detector,
}

impl AprilTagDetector {
    pub fn new(family: AprilTagFamily) -> Self {
        let detector = apriltag::Detector::builder()
            .add_family_bits(family.to_family(), 1)
            .build()
            .expect("failed to build AprilTag detector");
        Self { detector }
    }

    pub fn detect(&mut self, image: &GrayImage) -> Vec<AprilTagDetection> {
        let (w, h) = image.dimensions();
        let mut at_image = apriltag::Image::zeros_with_alignment(w as usize, h as usize, 96)
            .expect("failed to allocate AprilTag image");
        for (x, y, p) in image.enumerate_pixels() {
            at_image[(x as usize, y as usize)] = p[0];
        }
        self.detector
            .detect(&at_image)
            .into_iter()
            .map(|d| {
                let c = d.corners();
                let center = d.center();
                AprilTagDetection {
                    id: d.id() as u32,
                    corners: [
                        [c[0][0] as f32, c[0][1] as f32],
                        [c[1][0] as f32, c[1][1] as f32],
                        [c[2][0] as f32, c[2][1] as f32],
                        [c[3][0] as f32, c[3][1] as f32],
                    ],
                    center: [center[0] as f32, center[1] as f32],
                    decision_margin: d.decision_margin(),
                }
            })
            .collect()
    }
}

impl Default for AprilTagDetector {
    fn default() -> Self {
        Self::new(AprilTagFamily::default())
    }
}

/// Render an AprilTag marker (id) to a grayscale image via the C library.
/// Works for any valid id without pre-generated assets.
pub fn generate_marker(family: AprilTagFamily, id: u32) -> anyhow::Result<GrayImage> {
    unsafe {
        let fam = match family {
            AprilTagFamily::Tag36h11 => apriltag_sys::tag36h11_create(),
            AprilTagFamily::Tag25h9 => apriltag_sys::tag25h9_create(),
            AprilTagFamily::Tag16h5 => apriltag_sys::tag16h5_create(),
        };
        if fam.is_null() {
            anyhow::bail!("failed to create apriltag family");
        }
        let img = apriltag_sys::apriltag_to_image(fam, id as i32);
        let destroy_fam = |fam| match family {
            AprilTagFamily::Tag36h11 => apriltag_sys::tag36h11_destroy(fam),
            AprilTagFamily::Tag25h9 => apriltag_sys::tag25h9_destroy(fam),
            AprilTagFamily::Tag16h5 => apriltag_sys::tag16h5_destroy(fam),
        };
        if img.is_null() {
            destroy_fam(fam);
            anyhow::bail!("apriltag_to_image returned null for id {id}");
        }
        let i = &*img;
        let (w, h, stride) = (i.width as u32, i.height as u32, i.stride as usize);
        let mut gray = GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = *i.buf.add(y as usize * stride + x as usize);
                gray.put_pixel(x, y, image::Luma([v]));
            }
        }
        apriltag_sys::image_u8_destroy(img);
        destroy_fam(fam);
        Ok(gray)
    }
}

/// Discrete per-output colors for the geometry pass. Output index `i` shows
/// `PALETTE[i % PALETTE.len()]`.
pub const PALETTE: [[u8; 3]; 6] = [
    [255, 0, 0],   // red
    [0, 255, 0],   // green
    [0, 0, 255],   // blue
    [255, 255, 0], // yellow
    [255, 0, 255], // magenta
    [0, 255, 255], // cyan
];

/// Whether each RGB channel of `pixel` is within `tolerance` of `color`.
fn color_matches(pixel: &[u8; 3], color: [u8; 3], tolerance: u8) -> bool {
    (0..3).all(|i| pixel[i].abs_diff(color[i]) <= tolerance)
}

/// Find the quad corners (TL, TR, BR, BL) of the region showing `color`.
///
/// ponytail: the corners are the x±y extremes over ALL matching pixels — a
/// projected rectangle's corners are its extremes along both diagonals. The
/// known ceiling: scattered same-color noise elsewhere in the frame skews the
/// result. Upgrade path: connected-component labeling, take the largest blob.
pub fn find_color_quad(frame: &RgbaImage, color: [u8; 3], tolerance: u8) -> Option<[[f32; 2]; 4]> {
    let mut count = 0usize;
    // (x + y) extremes → TL / BR, (x - y) extremes → TR / BL.
    let (mut tl, mut br, mut tr, mut bl) = (f32::MAX, f32::MIN, f32::MIN, f32::MAX);
    let (mut tl_p, mut br_p, mut tr_p, mut bl_p) = ([0.0; 2], [0.0; 2], [0.0; 2], [0.0; 2]);

    for (x, y, p) in frame.enumerate_pixels() {
        if !color_matches(&[p[0], p[1], p[2]], color, tolerance) {
            continue;
        }
        count += 1;
        let point = [x as f32, y as f32];
        let sum = point[0] + point[1];
        let diff = point[0] - point[1];
        if sum < tl {
            tl = sum;
            tl_p = point;
        }
        if sum > br {
            br = sum;
            br_p = point;
        }
        if diff > tr {
            tr = diff;
            tr_p = point;
        }
        if diff < bl {
            bl = diff;
            bl_p = point;
        }
    }

    if count < MIN_REGION_PIXELS {
        return None;
    }
    Some([tl_p, tr_p, br_p, bl_p])
}

/// Axis-aligned bounding box of an overlap region, in frame pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlapRegion {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub pixel_count: usize,
}

/// Where two projector outputs showing colors `a` and `b` overlap, the camera
/// sees the additive mix. Match pixels where each channel is within
/// `tolerance` of `min(a + b, 255)` per channel; return their AABB.
pub fn measure_overlap(
    frame: &RgbaImage,
    a: [u8; 3],
    b: [u8; 3],
    tolerance: u8,
) -> Option<OverlapRegion> {
    let target = [
        a[0].saturating_add(b[0]),
        a[1].saturating_add(b[1]),
        a[2].saturating_add(b[2]),
    ];

    let mut count = 0usize;
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for (x, y, p) in frame.enumerate_pixels() {
        if !color_matches(&[p[0], p[1], p[2]], target, tolerance) {
            continue;
        }
        count += 1;
        let (xf, yf) = (x as f32, y as f32);
        min = [min[0].min(xf), min[1].min(yf)];
        max = [max[0].max(xf), max[1].max(yf)];
    }

    if count < MIN_REGION_PIXELS {
        return None;
    }
    Some(OverlapRegion {
        min,
        max,
        pixel_count: count,
    })
}

/// Mean RGB over the region (white-pass photometry). Returns black for an
/// empty/out-of-bounds region.
pub fn average_color(frame: &RgbaImage, region_min: [f32; 2], region_max: [f32; 2]) -> [f32; 3] {
    let (w, h) = (frame.width() as f32, frame.height() as f32);
    let x0 = region_min[0].max(0.0) as u32;
    let y0 = region_min[1].max(0.0) as u32;
    let x1 = (region_max[0].min(w) as u32).min(frame.width());
    let y1 = (region_max[1].min(h) as u32).min(frame.height());

    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = frame.get_pixel(x, y);
            for (i, s) in sum.iter_mut().enumerate() {
                *s += u64::from(p[i]);
            }
            count += 1;
        }
    }
    if count == 0 {
        return [0.0; 3];
    }
    [
        sum[0] as f32 / count as f32,
        sum[1] as f32 / count as f32,
        sum[2] as f32 / count as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, imageops};

    fn fill_rect(frame: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                frame.put_pixel(x, y, Rgba([color[0], color[1], color[2], 255]));
            }
        }
    }

    #[test]
    fn apriltag_roundtrip_synthetic_frame() {
        let marker = generate_marker(AprilTagFamily::Tag36h11, 0).unwrap();
        let scaled = imageops::resize(
            &marker,
            marker.width() * 10,
            marker.height() * 10,
            imageops::FilterType::Nearest,
        );

        // Black 640×480 frame, tag pasted at a known position.
        let (ox, oy) = (200u32, 150u32);
        let mut frame = GrayImage::from_pixel(640, 480, image::Luma([0]));
        imageops::replace(&mut frame, &scaled, ox as i64, oy as i64);
        let expected_center = [
            ox as f32 + scaled.width() as f32 / 2.0,
            oy as f32 + scaled.height() as f32 / 2.0,
        ];

        let mut detector = AprilTagDetector::default();
        let detections = detector.detect(&frame);

        assert_eq!(detections.len(), 1, "expected one tag, got {detections:?}");
        let d = &detections[0];
        assert_eq!(d.id, 0);
        assert!(
            (d.center[0] - expected_center[0]).abs() < 5.0
                && (d.center[1] - expected_center[1]).abs() < 5.0,
            "center {:?} too far from {expected_center:?}",
            d.center
        );
    }

    #[test]
    fn find_color_quad_finds_rect_ignores_decoy() {
        let mut frame = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        fill_rect(&mut frame, 100, 80, 220, 180, PALETTE[0]); // red target
        fill_rect(&mut frame, 400, 300, 500, 400, PALETTE[1]); // green decoy

        let [tl, tr, br, bl] = find_color_quad(&frame, PALETTE[0], 10).unwrap();
        assert!(
            (tl[0] - 100.0).abs() <= 1.0 && (tl[1] - 80.0).abs() <= 1.0,
            "TL {tl:?}"
        );
        assert!(
            (tr[0] - 219.0).abs() <= 1.0 && (tr[1] - 80.0).abs() <= 1.0,
            "TR {tr:?}"
        );
        assert!(
            (br[0] - 219.0).abs() <= 1.0 && (br[1] - 179.0).abs() <= 1.0,
            "BR {br:?}"
        );
        assert!(
            (bl[0] - 100.0).abs() <= 1.0 && (bl[1] - 179.0).abs() <= 1.0,
            "BL {bl:?}"
        );
    }

    #[test]
    fn measure_overlap_finds_additive_mix() {
        let mut frame = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        fill_rect(&mut frame, 0, 0, 100, 100, PALETTE[0]); // red only
        fill_rect(&mut frame, 50, 150, 150, 250, PALETTE[1]); // green only
        // red + green overlap as seen by the camera: yellow.
        fill_rect(&mut frame, 30, 300, 90, 380, PALETTE[3]);

        let region = measure_overlap(&frame, PALETTE[0], PALETTE[1], 10).unwrap();
        assert_eq!(region.pixel_count, 60 * 80);
        assert_eq!(region.min, [30.0, 300.0]);
        assert_eq!(region.max, [89.0, 379.0]);

        let mean = average_color(
            &frame,
            region.min,
            [region.max[0] + 1.0, region.max[1] + 1.0],
        );
        assert!((mean[0] - 255.0).abs() < 1.0 && (mean[1] - 255.0).abs() < 1.0 && mean[2] < 1.0);
    }

    #[test]
    fn empty_frame_yields_none() {
        let frame = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        assert_eq!(find_color_quad(&frame, PALETTE[0], 10), None);
        assert_eq!(measure_overlap(&frame, PALETTE[0], PALETTE[1], 10), None);
        assert_eq!(average_color(&frame, [10.0, 10.0], [0.0, 0.0]), [0.0; 3]);
    }
}
