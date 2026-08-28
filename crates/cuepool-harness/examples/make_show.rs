//! Build a one-cue CuePool project that plays a given video file, for manually
//! exercising a codec path in the real app.
//!
//! Usage: cargo run -p cuepool-harness --example make_show -- <video> <out.qproj>

use cuepool_core::{AudioRouting, Cue, CueBase, LoopMode, ShowFile, Timespan, TriggerMode};
use rust_decimal::Decimal;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let video = args.next().expect("usage: make_show <video> <out.qproj>");
    let out = args.next().expect("usage: make_show <video> <out.qproj>");

    let mut base = CueBase {
        qid: Decimal::from(1),
        name: "NotchLC playback".into(),
        trigger: TriggerMode::Go,
        ..CueBase::default()
    };
    base.loop_mode = LoopMode::LoopedInfinite;

    let show = ShowFile {
        cues: vec![Cue::Video {
            base,
            // Absolute, so the project can live anywhere.
            path: video.clone().into(),
            start_time: Timespan::ZERO,
            // Zero means "play to the end".
            duration: Timespan::ZERO,
            volume: 1.0,
            pan: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_type: Default::default(),
            eq: None,
            routing: AudioRouting::default(),
            follow_mtc: false,
            mtc_start: Timespan::ZERO,
        }],
        ..ShowFile::default()
    };

    std::fs::write(&out, serde_json::to_vec_pretty(&show)?)?;
    println!("wrote {out} -> cue 1 plays {video}");
    Ok(())
}
