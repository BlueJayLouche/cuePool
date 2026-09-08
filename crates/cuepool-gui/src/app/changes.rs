//! Offline changes for the exact source revision, separate from the once-per-
//! minor-release welcome modal.

pub(super) fn show(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("Changes")
        .open(open)
        .default_width(620.0)
        .default_height(440.0)
        .show(ctx, |ui| {
            let build = &cuepool_core::build_identity::BUILD;
            ui.label(build.display);
            if let Some(commit) = build.commit {
                ui.monospace(format!("Source commit: {commit}"));
            }
            ui.separator();
            ui.label(format!("Comparison baseline: {}", build.changes_baseline));
            ui.label("Publication status is the recorded status at build time; a version tag alone is not a published release.");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for line in build.changes.lines() {
                    if line.starts_with('#') {
                        ui.add_space(6.0);
                        ui.strong(line.trim_start_matches('#').trim());
                    } else if line.is_empty() {
                        ui.add_space(4.0);
                    } else {
                        ui.label(line);
                    }
                }
            });
        });
}
