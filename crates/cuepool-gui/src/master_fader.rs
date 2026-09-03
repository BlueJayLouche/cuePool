//! Master output fader — the room trim, in Project Settings → Audio under the
//! limiter. Mirrors OSC `/qplayer/volume`; saved with the show.

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

pub(crate) fn format_master_db(db: f32) -> String {
    if db <= MASTER_VOLUME_DB_MIN {
        "−∞ dB".to_string()
    } else {
        format!("{db:+.1} dB")
    }
}

/// Slider plus dB readout bound to a show setting. Returns true when `db`
/// changed (drag, or double-click back to 0 dB); the caller marks the show
/// dirty and re-applies the audio levels.
pub fn show(ui: &mut egui::Ui, db: &mut f32) -> bool {
    let mut pos = db_to_fader(*db);
    ui.spacing_mut().slider_width = 160.0;
    let response = ui
        .add(
            egui::Slider::new(&mut pos, 0.0..=1.0)
                .show_value(false)
                .trailing_fill(true),
        )
        .on_hover_text("Drag, or double-click for 0 dB");
    let new_db = if response.double_clicked() {
        Some(0.0)
    } else if response.changed() {
        Some(fader_to_db(pos))
    } else {
        None
    };
    ui.label(RichText::new(format_master_db(*db)).monospace())
        .on_hover_text("Master output gain");
    match new_db {
        Some(value) if value != *db => {
            *db = value;
            true
        }
        _ => false,
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
        assert_eq!(format_master_db(0.0), "+0.0 dB");
        assert_eq!(format_master_db(-6.04), "-6.0 dB");
        assert_eq!(format_master_db(-96.0), "−∞ dB");
    }
}
