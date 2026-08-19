//! Camera-based auto-blend calibration support.
//!
//! A camera (network stream) watches the projection surface while CuePool
//! shows AprilTag markers and discrete per-output colors on each projector
//! output:
//!
//! - [`capture::StreamCapture`] decodes the camera stream in the background
//!   and hands out the latest frame.
//! - [`detect`] finds AprilTag markers and colored regions in those frames,
//!   feeding the geometry, blend-overlap, and photometry passes.

pub mod capture;
pub mod detect;
