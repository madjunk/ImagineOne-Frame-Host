fn main() {
    // Our own commands get allow-<name> permissions (capabilities/*.json decide who may call them).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            // the title bar and its screens (local page "chrome")
            "frame_get_state",
            "frame_action",
            "frame_continue_org",
            "frame_save_settings",
            "frame_settings_closed",
            "frame_open_menu",
            "frame_online",
            "frame_win",
            // the ImagineOne web app (the configured server only, added at run time)
            "frame_user",
            "frame_sidebar",
            "frame_cred_get",
            "frame_cred_save",
            "frame_cred_clear",
            "frame_page_action",
        ]),
    ))
    .expect("failed to run tauri-build");
}
