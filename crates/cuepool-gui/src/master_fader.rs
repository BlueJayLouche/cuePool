//! Master output fader — the room trim, in the status bar next to the audio
//! state. Mirrors OSC `/qplayer/volume`; the binary persists it per machine.

use crate::app::{AppCommand, SharedStateHandle};
use cuepool_audio::engine::{MASTER_VOLUME_DB_MAX, MASTER_VOLUME_DB_MIN};
use egui::RichText;

/// Fader travel (0 = bottom, 1 = top) to dB. A squared taper puts unity at two
/// thirds of travel and spreads the audible range over most of the slider,
/// instead of parking everything useful in the top tenth of a linear -96..+12.
fn fader_to_db(pos: f32) -> f32 {
    let span = MASTER_VOLUME_DB_MAX - MASTER_VOLUME_DB_MIN;
    MASTER_VOLUME_DB_MAX - span * (1.0 - pos.clamp(0.0, 1.0)).powi(2)
}

fn db_to_fader(db: f32) -> f32 {
    let span = MASTER_VOLUME_DB_MAX - MASTER_VOLUME_DB_MIN;
    let db = db.clamp(MASTER_VOLUME_DB_MIN, MASTER_VOLUME_DB_MAX);
    1.0 - ((MASTER_VOLUME_DB_MAX - db) / span).sqrt()
}

fn format_master_db(db: f32) -> String {
    if db <= MASTER_VOLUME_DB_MIN {
        "Master −∞ dB".to_string()
    } else {
        format!("Master {db:+.1} dB")
    }
}

/// Readout then slider, for a right-to-left row: the value sits to the right
/// of the fader as egui's own sliders do. Queues [`AppCommand::SetMasterVolume`]
/// on drag, and on double-click (back to 0 dB).
pub fn show(ui: &mut egui::Ui, state: &SharedStateHandle) {
    let master_db = match state.lock() {
        Ok(state) => state.master_volume_db,
        Err(_) => return,
    };
    ui.label(
        RichText::new(format_master_db(master_db))
            .small()
            .monospace(),
    )
    .on_hover_text("Master output gain");

    let mut pos = db_to_fader(master_db);
    ui.spacing_mut().slider_width = 100.0;
    let response = ui
        .add(
            egui::Slider::new(&mut pos, 0.0..=1.0)
                .show_value(false)
                .trailing_fill(true),
        )
        .on_hover_text(
            "Master volume — drag, or double-click for 0 dB.\n\
             Also set by OSC /qplayer/volume. Saved per machine, not with the show.",
        );
    let new_db = if response.double_clicked() {
        Some(0.0)
    } else if response.changed() {
        Some(fader_to_db(pos))
    } else {
        None
    };
    if let Some(db) = new_db
        && let Ok(mut state) = state.lock()
    {
        state.command_queue.push(AppCommand::SetMasterVolume(db));
    }
}

#[cfg(test)]
mod tests {
    use super::{db_to_fader, fader_to_db, format_master_db};

    #[test]
    fn taper_round_trips_with_unity_at_two_thirds() {
        assert!((fader_to_db(1.0) - 12.0).abs() < 1e-4);
        assert!((fader_to_db(0.0) + 96.0).abs() < 1e-4);
        assert!(
            (db_to_fader(0.0) - 2.0 / 3.0).abs() < 1e-4,
            "unity sits at two thirds of travel"
        );
        for db in [-96.0, -60.0, -20.0, -6.0, 0.0, 3.0, 12.0] {
            assert!(
                (fader_to_db(db_to_fader(db)) - db).abs() < 1e-3,
                "round trip at {db} dB"
            );
        }
    }

    #[test]
    fn readout_formats_sign_and_floor() {
        assert_eq!(format_master_db(0.0), "Master +0.0 dB");
        assert_eq!(format_master_db(-6.04), "Master -6.0 dB");
        assert_eq!(format_master_db(-96.0), "Master −∞ dB");
    }
}
