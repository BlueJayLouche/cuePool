//! CuePool GUI — egui + wgpu immediate-mode interface.
//!
//! Replaces all WPF Views and ViewModels.

pub use cuepool_core::build_identity::build_identity;

pub mod active_cues;
pub mod app;
pub mod atomic_write;
pub mod cue_list;
pub mod cue_order;
pub mod inspector;
pub mod lighting_panel;
pub mod log_window;
pub mod logging;
pub mod master_fader;
pub mod osc_destination;
pub mod preview;
pub mod projection_panel;
pub mod recorder_panel;
mod scrub;
pub mod status_panel;
pub mod take_editor;
pub mod transport;
pub mod waveform;

pub use app::{
    ActiveCueInfo, AppCommand, CuePoolApp, DecodeTiming, Diagnostics, GuiMeterData,
    OutputDiagnostics, PreparedProject, RELEASE_NOTES_VERSION, SharedState, SharedStateHandle,
    ShowMode, VideoDiagnostics, VideoTimings, prepare_unattended_project,
};

pub(crate) fn cue_type_label(cue: &cuepool_core::Cue) -> &'static str {
    use cuepool_core::Cue;

    match cue {
        Cue::Group { .. } => "GRP",
        Cue::Sound { .. } => "SND",
        Cue::Video { .. } => "VID",
        Cue::Stop { .. } => "STP",
        Cue::Volume { .. } => "VOL",
        Cue::Dummy { .. } => "DUM",
        Cue::TimeCode { .. } => "TC",
        Cue::Network { .. } => "NET",
        Cue::Text { .. } => "TXT",
        Cue::Image { .. } => "IMG",
        Cue::Goto { .. } => "GTO",
        Cue::Lighting { .. } => "LX",
        Cue::DmxShow { .. } => "DMX",
        Cue::PixelMap { .. } => "PXM",
    }
}

pub(crate) fn colour_to_egui(c: cuepool_core::SerializedColour) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_identity_includes_the_workspace_version() {
        assert_eq!(
            super::build_identity(),
            cuepool_core::build_identity::BUILD.display
        );
        assert!(super::build_identity().starts_with(env!("CARGO_PKG_VERSION")));
    }
}
