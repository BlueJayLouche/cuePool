//! Master-volume commands use the same queue and show setting as the GUI.
//! No cached OSC level: a query also observes show loads and device rebuilds.

use cuepool_audio::engine::clamp_master_volume_db;
use cuepool_audio::{AudioEngine, mixer::db_to_linear};
use cuepool_core::LockExt;
use cuepool_gui::{AppCommand, SharedStateHandle};
use cuepool_protocols::osc::OscEvent;
use rosc::{OscMessage, OscType};
use std::net::SocketAddr;

/// Apply the current show levels after edits, show loads or engine replacement.
pub fn apply_to_engine(state: &SharedStateHandle, audio: &AudioEngine) {
    let state = state.lock_unpoisoned();
    let settings = &state.show_file.show_settings;
    audio.set_master_volume_db(settings.master_volume_db);
    audio.set_limiter_threshold(db_to_linear(settings.limiter_threshold_db()));
}

/// Queue a setter and its optional confirmation together, preserving arrival
/// order with standalone queries. Other OSC events remain the adapter's job.
pub fn enqueue(state: &SharedStateHandle, event: OscEvent) -> Result<(), OscEvent> {
    let mut state = state.lock_unpoisoned();
    let reply = match event {
        OscEvent::Volume { db, reply } => {
            state.command_queue.push(AppCommand::SetMasterVolume(db));
            reply
        }
        OscEvent::VolumeQuery { src, request_id } => Some((src, request_id)),
        other => return Err(other),
    };
    if let Some((reply_to, request_id)) = reply {
        state.command_queue.push(AppCommand::QueryMasterVolume {
            reply_to,
            request_id,
        });
    }
    Ok(())
}

/// Called by the normal application command drain. Apply the audio setting
/// before continuing to the next command (which may be its confirmation).
/// The caller sends returned replies with OscManager::send_to, never broadcast.
pub fn process(
    state: &SharedStateHandle,
    command: AppCommand,
    apply_audio_levels: impl FnOnce(),
) -> Result<Option<(SocketAddr, OscMessage)>, AppCommand> {
    match command {
        AppCommand::SetMasterVolume(db) => {
            let db = clamp_master_volume_db(db);
            let changed = {
                let mut state = state.lock_unpoisoned();
                let changed = state.show_file.show_settings.master_volume_db != db;
                if changed {
                    state.show_file.show_settings.master_volume_db = db;
                    state.dirty = true;
                }
                changed
            };
            if changed {
                apply_audio_levels();
            }
            Ok(None)
        }
        AppCommand::QueryMasterVolume {
            reply_to,
            request_id,
        } => {
            let db = clamp_master_volume_db(
                state
                    .lock_unpoisoned()
                    .show_file
                    .show_settings
                    .master_volume_db,
            );
            Ok(Some((
                reply_to,
                OscMessage {
                    addr: "/qplayer/volume/state".into(),
                    args: vec![OscType::Int(request_id), OscType::Float(db)],
                },
            )))
        }
        other => Err(other),
    }
}
