mod audio;
mod midi;
mod pitch;
mod state;
mod commands;

use tauri::Manager;

pub fn run() {
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    env_logger::init();

    tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_dialog::init())
    .manage(state::AppState::new())
    .invoke_handler(tauri::generate_handler![
        commands::start_audio_capture,
        commands::stop_audio_capture,
        commands::load_midi_file,
        commands::load_audio_file,
        commands::start_playback,
        commands::stop_playback,
        commands::set_bpm,
        commands::set_time_signature,
        commands::get_audio_devices,
        commands::get_recording_data,
    ])
    .setup(|app| {
        let window = app.get_webview_window("main").unwrap();
        // Remove window chrome for immersive UI
        #[cfg(target_os = "macos")]
        {
            use tauri::TitleBarStyle;
            window.set_title_bar_style(TitleBarStyle::Overlay)?;
        }
        Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
