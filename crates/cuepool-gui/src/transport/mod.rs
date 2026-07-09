//! Transport controls — Go, Stop, Pause buttons.

use crate::app::{AppCommand, GuiMeterData, SharedStateHandle};
use egui::{Button, Color32, RichText, Vec2};

/// `HH:MM:SS.ff` at a display-only frame rate (triggers store seconds).
fn format_timecode(secs: f64, fps: f32) -> String {
    let fps = fps.max(1.0) as f64;
    let t = secs.max(0.0);
    let frames = (t * fps).round() as u64;
    let fph = (fps * 3600.0) as u64;
    let fpm = (fps * 60.0) as u64;
    let fpsu = fps as u64;
    format!(
        "{:02}:{:02}:{:02}.{:02}",
        frames / fph,
        (frames % fph) / fpm,
        (frames % fpm) / fpsu,
        frames % fpsu
    )
}

/// Parse `[HH:][MM:]SS[.ff]` — the `.ff` part is frames at the display rate,
/// matching the readout. `5:30`, `01:05:30.15`, and plain `90` all work.
fn parse_timecode(text: &str, fps: f32) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (clock, frames) = match text.split_once('.') {
        Some((c, f)) => (c, f.trim().parse::<u32>().ok()?),
        None => (text, 0),
    };
    let mut secs = 0.0f64;
    for part in clock.split(':') {
        secs = secs * 60.0 + part.trim().parse::<u32>().ok()? as f64;
    }
    Some(secs + frames as f64 / fps.max(1.0) as f64)
}

pub fn show(ui: &mut egui::Ui, state: &SharedStateHandle) {
    ui.horizontal(|ui| {
        let button_size = Vec2::new(60.0, 32.0);

        let go_btn = Button::new(RichText::new("▶ GO").strong().color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 180, 0))
            .min_size(button_size);
        if ui.add(go_btn).clicked() {
            if let Ok(mut state) = state.lock() {
                state.command_queue.push(AppCommand::Go);
            }
        }

        let stop_btn = Button::new(RichText::new("⏹ STOP").strong())
            .fill(Color32::from_rgb(200, 0, 0))
            .min_size(button_size);
        if ui.add(stop_btn).clicked() {
            if let Ok(mut state) = state.lock() {
                state.command_queue.push(AppCommand::Stop);
            }
        }

        let pause_btn = Button::new(RichText::new("⏸ PAUSE"))
            .min_size(button_size);
        if ui.add(pause_btn).clicked() {
            if let Ok(mut state) = state.lock() {
                state.command_queue.push(AppCommand::Pause);
            }
        }

        let (show_time, show_paused, next_tc, tc_fps) = {
            let Ok(state) = state.lock() else { return };
            (
                state.show_time,
                state.show_paused,
                state.next_timecode,
                state.show_file.show_settings.timecode_fps,
            )
        };
        let step_btn = Button::new(RichText::new("⏭")).min_size(Vec2::new(36.0, 32.0));
        if ui
            .add_enabled(show_paused, step_btn)
            .on_hover_text("Step one video frame forward (paused only); the show clock follows")
            .clicked()
        {
            if let Ok(mut state) = state.lock() {
                state.command_queue.push(AppCommand::FrameStep);
            }
        }

        let preload_btn = Button::new(RichText::new("PRELOAD"))
            .min_size(Vec2::new(70.0, 32.0));
        if ui.add(preload_btn).clicked() {
            if let Ok(mut state) = state.lock() {
                state.command_queue.push(AppCommand::Preload);
            }
        }

        ui.separator();

        // Show clock (the clock timecode triggers fire against) + next trigger.
        let tc_color = if show_paused {
            Color32::from_rgb(230, 190, 60)
        } else if show_time.is_some() {
            Color32::from_rgb(120, 220, 120)
        } else {
            Color32::from_gray(110)
        };
        let tc_text = match show_time {
            Some(t) => format_timecode(t, tc_fps),
            None => "--:--:--.--".into(),
        };
        // Click the readout to type a new position (playhead seek).
        let edit_id = egui::Id::new("tc_edit");
        let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
        if let Some(mut text) = editing {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(150.0)
                    .font(egui::TextStyle::Monospace),
            );
            resp.request_focus();
            let confirmed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape))
                || (resp.lost_focus() && !confirmed);
            if confirmed {
                match parse_timecode(&text, tc_fps) {
                    Some(secs) => {
                        if let Ok(mut state) = state.lock() {
                            state.command_queue.push(AppCommand::SeekShowClock { secs });
                        }
                    }
                    None => log::warn!("Unparseable timecode '{text}' — use HH:MM:SS.ff"),
                }
            }
            if confirmed || cancelled {
                ui.data_mut(|d| d.remove::<String>(edit_id));
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
        } else {
            let resp = ui
                .add(
                    egui::Label::new(
                        RichText::new(tc_text).monospace().size(20.0).color(tc_color),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(if show_paused {
                    "Show clock (paused) — click to jump the playhead"
                } else {
                    "Show clock — click to jump the playhead"
                });
            if resp.clicked() {
                let initial = show_time.map(|t| format_timecode(t, tc_fps)).unwrap_or_default();
                ui.data_mut(|d| d.insert_temp(edit_id, initial));
            }
        }
        if let Some((qid, t)) = next_tc {
            ui.label(
                RichText::new(format!("next: Q{qid} @ {}", format_timecode(t, tc_fps)))
                    .monospace()
                    .small()
                    .weak(),
            );
        }

        ui.separator();

        // Show / Edit mode toggle
        let mode = {
            let Ok(state) = state.lock() else { return };
            state.show_mode
        };

        let mode_label = match mode {
            crate::app::ShowMode::Edit => "Edit Mode",
            crate::app::ShowMode::Show => "Show Mode",
        };
        let mode_color = match mode {
            crate::app::ShowMode::Edit => Color32::from_rgb(60, 60, 60),
            crate::app::ShowMode::Show => Color32::from_rgb(180, 140, 0),
        };

        let mode_btn = Button::new(RichText::new(mode_label).strong().color(Color32::WHITE))
            .fill(mode_color)
            .min_size(Vec2::new(100.0, 32.0));
        if ui.add(mode_btn).clicked() {
            if let Ok(mut state) = state.lock() {
                let snapshot = crate::app::Snapshot::from_state(&state);
                state.undo_redo.push(snapshot);
                state.show_mode = match state.show_mode {
                    crate::app::ShowMode::Edit => crate::app::ShowMode::Show,
                    crate::app::ShowMode::Show => crate::app::ShowMode::Edit,
                };
                state.dirty = true;
            }
        }

        // Master meter bridge
        let meter_data = {
            let Ok(state) = state.lock() else { return };
            state.meter_data
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw_meter(ui, &meter_data);
        });
    });
}

fn draw_meter(ui: &mut egui::Ui, data: &GuiMeterData) {
    let width = 8.0;
    let height = 32.0;
    let _gap = 4.0;

    for &(peak_db, rms_db) in &[(data.peak_l_db, data.rms_l_db), (data.peak_r_db, data.rms_r_db)] {
        let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 1.0, Color32::from_rgb(30, 30, 30));

        // Draw segments from bottom up
        let segments = 12i32;
        let seg_h = height / segments as f32;
        for i in 0..segments {
            let seg_db = -60.0 + (i as f32 / segments as f32) * 60.0; // -60dB to 0dB
            let seg_y = rect.max.y - (i as f32 + 0.5) * seg_h;
            let seg_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, seg_y),
                egui::vec2(width - 2.0, seg_h - 1.0),
            );

            let lit = rms_db >= seg_db || peak_db >= seg_db;
            let peak_lit = peak_db >= seg_db;
            let colour = if seg_db >= 0.0 {
                Color32::RED
            } else if seg_db >= -12.0 {
                Color32::YELLOW
            } else {
                Color32::GREEN
            };

            if peak_lit {
                painter.rect_filled(seg_rect, 1.0, colour);
            } else if lit {
                painter.rect_filled(seg_rect, 1.0, colour.gamma_multiply(0.5));
            }
        }
    }

    // GR meter (gain reduction)
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 1.0, Color32::from_rgb(30, 30, 30));

    let gr_segments = 12i32;
    let seg_h = height / gr_segments as f32;
    let gr_db = data.limiter_gr_db;
    for i in 0..gr_segments {
        let seg_db = -(i as f32 / gr_segments as f32) * 30.0; // 0 to -30 dB
        let seg_y = rect.min.y + (i as f32 + 0.5) * seg_h;
        let seg_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, seg_y),
            egui::vec2(width - 2.0, seg_h - 1.0),
        );
        let lit = gr_db <= seg_db;
        let colour = if seg_db <= -20.0 {
            Color32::RED
        } else if seg_db <= -10.0 {
            Color32::YELLOW
        } else {
            Color32::from_rgb(100, 200, 255)
        };
        if lit {
            painter.rect_filled(seg_rect, 1.0, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_timecode;

    #[test]
    fn timecode_parsing() {
        use super::parse_timecode;
        assert_eq!(parse_timecode("90", 30.0), Some(90.0));
        assert_eq!(parse_timecode("5:30", 30.0), Some(330.0));
        assert_eq!(parse_timecode("01:05:30.15", 30.0), Some(3930.5));
        assert_eq!(parse_timecode("0:00.15", 30.0), Some(0.5), "frames at display fps");
        assert_eq!(parse_timecode("", 30.0), None);
        assert_eq!(parse_timecode("abc", 30.0), None);
        // Round-trip with the formatter.
        let t = parse_timecode(&super::format_timecode(3930.5, 30.0), 30.0).unwrap();
        assert!((t - 3930.5).abs() < 1.0 / 30.0);
    }

    #[test]
    fn timecode_formatting() {
        assert_eq!(format_timecode(0.0, 30.0), "00:00:00.00");
        assert_eq!(format_timecode(1.0, 30.0), "00:00:01.00");
        assert_eq!(format_timecode(0.5, 30.0), "00:00:00.15");
        assert_eq!(format_timecode(3661.5, 25.0), "01:01:01.13", "25fps half-second = frame 12.5 → 13");
        assert_eq!(format_timecode(-3.0, 30.0), "00:00:00.00", "negative clamps");
    }
}
