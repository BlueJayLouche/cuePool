//! Background network-stream capture for camera-based calibration.
//!
//! Decodes a network stream URL (RTSP/HLS/HTTP) via ffmpeg-next to RGBA
//! frames on a background thread, keeping only the latest frame in a shared
//! slot. Software decode only; audio and other streams are skipped.

use crate::video_source::open_input_with_options;
use ffmpeg_next::format::{self, Pixel};
use ffmpeg_next::software::scaling;
use ffmpeg_next::{codec, frame, media::Type};
use image::RgbaImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Latest-frame view of a network camera stream.
///
/// ponytail: single latest-frame slot (`Arc<Mutex<Option<RgbaImage>>>`), no
/// ring buffer, no PTS handling — calibration is not real-time.
pub struct StreamCapture {
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<RgbaImage>>>,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl StreamCapture {
    /// Open the stream and spawn the decode thread. Open errors surface here
    /// synchronously, so a bad URL fails before `start` returns.
    pub fn start(url: &str) -> anyhow::Result<Self> {
        ffmpeg_next::init()?;
        let stop = Arc::new(AtomicBool::new(false));
        // Open on the calling thread; the interrupt flag also lets Drop abort
        // a blocked open/read.
        // Camera streams only. HLS playlists and RTSP setups may reference
        // further URLs; the whitelist keeps those on the network too.
        let mut options = ffmpeg_next::Dictionary::new();
        options.set(
            "protocol_whitelist",
            "rtsp,rtsps,rtp,http,https,tcp,udp,tls,crypto,srtp,hls,applehttp",
        );
        let ictx = open_input_with_options(url, &stop, options)?;

        let latest = Arc::new(Mutex::new(None));
        let running = Arc::new(AtomicBool::new(true));
        let thread = {
            let stop = Arc::clone(&stop);
            let latest = Arc::clone(&latest);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                match decode_loop(ictx, &stop, &latest) {
                    // ponytail: no retry loop — on error/EOF we log and exit;
                    // the caller sees it via `is_running`.
                    Ok(()) => log::info!("stream capture ended"),
                    Err(error) => log::warn!("stream capture ended: {error:#}"),
                }
                running.store(false, Ordering::Relaxed);
            })
        };

        Ok(Self {
            stop,
            latest,
            running,
            thread: Some(thread),
        })
    }

    /// Clone of the most recent decoded frame, `None` if none has arrived yet.
    pub fn latest_frame(&self) -> Option<RgbaImage> {
        self.latest.lock().ok()?.clone()
    }

    /// Whether the decode thread is still alive.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for StreamCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn decode_loop(
    mut ictx: format::context::Input,
    stop: &AtomicBool,
    latest: &Mutex<Option<RgbaImage>>,
) -> anyhow::Result<()> {
    let stream_index = ictx
        .streams()
        .best(Type::Video)
        .map(|stream| {
            let index = stream.index();
            (index, stream.parameters())
        })
        .ok_or_else(|| anyhow::anyhow!("no video stream in capture source"))?;
    let mut decoder = codec::Context::from_parameters(stream_index.1)?
        .decoder()
        .video()?;
    let stream_index = stream_index.0;

    // Recreated if the decoded format/size changes mid-stream.
    let mut scaler: Option<((Pixel, u32, u32), scaling::Context)> = None;
    let mut decoded = frame::Video::empty();
    let mut rgb = frame::Video::empty();

    for (stream, packet) in ictx.packets() {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        if stream.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet)?;
        while decoder.receive_frame(&mut decoded).is_ok() {
            let key = (decoded.format(), decoded.width(), decoded.height());
            if !matches!(&scaler, Some((k, _)) if *k == key) {
                scaler = Some((
                    key,
                    scaling::Context::get(
                        key.0,
                        key.1,
                        key.2,
                        Pixel::RGBA,
                        key.1,
                        key.2,
                        scaling::Flags::BILINEAR,
                    )?,
                ));
            }
            scaler
                .as_mut()
                .expect("scaler just created")
                .1
                .run(&decoded, &mut rgb)?;
            if let Some(image) = frame_to_rgba(&rgb) {
                *latest.lock().unwrap_or_else(|error| error.into_inner()) = Some(image);
            }
        }
    }
    Ok(())
}

/// Copy an RGBA `frame::Video` into an `RgbaImage`, honouring the row stride.
fn frame_to_rgba(frame: &frame::Video) -> Option<RgbaImage> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let stride = frame.stride(0);
    let data = frame.data(0);
    if stride < w * 4 || data.len() < stride * h {
        return None;
    }
    let mut buf = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        buf.extend_from_slice(&data[y * stride..y * stride + w * 4]);
    }
    RgbaImage::from_raw(w as u32, h as u32, buf)
}
