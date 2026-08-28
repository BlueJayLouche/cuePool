//! Which decode path does a file actually take, given a GPU texture limit?
//! `decode_check` deliberately reports no GPU, so it can only ever show the
//! software path; this mirrors what the app passes.
//!
//! Usage: cargo run -p cuepool-video --example decode_path_check -- <file>

use cuepool_video::{FramePool, VideoSource};
use std::sync::Arc;

fn main() {
    let path = std::env::args().nth(1).expect("usage: decode_path_check <file>");
    let mut src =
        VideoSource::open_with_pool_and_hap(&path, Arc::new(FramePool::new(0)), Some(16_384))
            .expect("open failed");
    println!("{}: {}x{}", path, src.width(), src.height());
    println!("decode path: {}", src.decode_path());
    if let Some(reason) = src.fallback_reason() {
        println!("fallback reason: {reason}");
    }
    let mut frames = 0;
    while src.read_frame().is_some() && frames < 60 {
        frames += 1;
    }
    println!("decoded {frames} frames, path still: {}", src.decode_path());
}
