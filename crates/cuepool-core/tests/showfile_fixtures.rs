//! Real on-disk documents from older format versions, loaded through the
//! production entry point. The unit tests in `showfile/migration.rs` feed
//! in-memory structs to each migrator; these prove the *loader* runs them.

use cuepool_core::showfile::FILE_FORMAT_VERSION;
use cuepool_core::{Cue, TriggerMode, parse_show_file};

#[test]
fn a_v9_file_migrates_its_nic_to_a_broadcast_destination() {
    let show = parse_show_file(include_str!("fixtures/v9_osc_nic.qproj")).unwrap();
    assert_eq!(show.file_format_version, FILE_FORMAT_VERSION);
    assert_eq!(show.show_settings.title, "Nine");
    assert_eq!(show.show_settings.osc_tx_host, "192.168.4.255");
}

#[test]
fn a_v6_file_converts_linear_volume_to_db() {
    let show = parse_show_file(include_str!("fixtures/v6_linear_volume.qproj")).unwrap();
    assert_eq!(show.file_format_version, FILE_FORMAT_VERSION);
    let Cue::Sound { volume, .. } = &show.cues[0] else {
        panic!("expected a SoundCue, got {:?}", show.cues[0]);
    };
    // 20 * log10(0.5)
    assert!((volume + 6.0206).abs() < 0.01, "volume = {volume}");
    // v6 -> v7 also flips enable_msc on for files that predate the setting.
    assert!(show.show_settings.enable_msc);
}

#[test]
fn a_v3_file_maps_halt_onto_trigger_mode() {
    let show = parse_show_file(include_str!("fixtures/v3_halt.qproj")).unwrap();
    assert_eq!(show.file_format_version, FILE_FORMAT_VERSION);
    assert_eq!(show.cues[0].base().trigger, TriggerMode::WithLast);
    assert_eq!(show.cues[1].base().trigger, TriggerMode::Go);
}
