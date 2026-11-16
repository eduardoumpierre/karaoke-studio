mod player;
mod sidecar;

use std::sync::Arc;
use std::time::Duration;

use player::Player;
use sidecar::{BackendConfig, Sidecar};
use tauri::{Manager, RunEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

/// A UI chama isto para saber onde está o backend; espera ele ficar pronto (até 90 s).
#[tauri::command]
async fn get_backend(state: tauri::State<'_, Sidecar>) -> Result<BackendConfig, String> {
    let shared = state.shared();
    tauri::async_runtime::spawn_blocking(move || shared.wait_ready(Duration::from_secs(90)))
        .await
        .map_err(|e| e.to_string())?
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("karaoke-studio".into()),
                    }),
                ])
                .max_file_size(2_000_000)
                .rotation_strategy(RotationStrategy::KeepSome(3))
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            log::info!("pasta de dados: {}", data_dir.display());
            app.manage(Sidecar::start(data_dir.clone()));
            std::fs::create_dir_all(&data_dir).ok();
            app.manage(Player::start(app.handle().clone(), &data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_backend,
            player::player_devices,
            player::player_set_output,
            player::player_set_input,
            player::player_load,
            player::player_play,
            player::player_pause,
            player::player_seek,
            player::player_set_mix,
            player::player_mic_test,
        ])
        .build(tauri::generate_context!())
        .expect("erro ao construir o aplicativo Tauri");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(player) = handle.try_state::<Arc<Player>>() {
                player.shutdown();
            }
            if let Some(sidecar) = handle.try_state::<Sidecar>() {
                sidecar.shutdown();
            }
        }
    });
}
