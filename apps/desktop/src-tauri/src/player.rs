//! Ligação entre a interface (Tauri) e o motor de áudio (`audio-engine`).
//! Os comandos que esperam o motor rodam em threads de bloqueio, para não travar a interface.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use audio_engine::{DeviceList, Engine, Event, Loaded, MicInfo, Mix, OutputInfo, VoiceSettings};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    /// `None` = padrão do sistema.
    pub output_device: Option<String>,
    pub input_device: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoadRequest {
    pub vocals: Option<String>,
    pub instrumental: String,
    pub autoplay: bool,
    pub start: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedInfo {
    pub duration: f64,
    pub has_vocals: bool,
}

#[derive(Deserialize)]
pub struct MixArg {
    pub master: f32,
    pub vocal: f32,
    pub instrumental: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesReply {
    pub devices: DeviceList,
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub output: Option<OutputInfo>,
}

pub struct Player {
    engine: Engine,
    settings: Mutex<AudioSettings>,
    settings_path: PathBuf,
    current: Mutex<Option<LoadRequest>>,
    /// Número do último pedido de carregamento; pedidos antigos que terminam depois são descartados.
    load_id: AtomicU64,
    /// Últimos ajustes de voz pedidos pela interface (reaplicados ao reabrir o microfone).
    voice: Mutex<Option<VoiceSettings>>,
    /// O teste de microfone das configurações está aberto.
    mic_testing: AtomicBool,
}

fn read_settings(path: &Path) -> AudioSettings {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

impl Player {
    pub fn start(app: AppHandle, data_dir: &Path) -> Arc<Player> {
        let settings_path = data_dir.join("audio-settings.json");
        let settings = read_settings(&settings_path);
        let handle = app.clone();
        let engine = Engine::start(move |event: Event| {
            let _ = handle.emit("player-event", event);
        });
        let player = Arc::new(Player {
            engine: engine.clone(),
            settings: Mutex::new(settings.clone()),
            settings_path,
            current: Mutex::new(None),
            load_id: AtomicU64::new(0),
            voice: Mutex::new(None),
            mic_testing: AtomicBool::new(false),
        });
        // Aplica a saída salva sem travar a abertura da janela.
        if settings.output_device.is_some() {
            std::thread::spawn(move || {
                if let Err(e) = engine.set_output(settings.output_device.clone()) {
                    log::warn!("saída salva indisponível ({e}); usando a padrão");
                }
            });
        }
        player
    }

    pub fn shutdown(&self) {
        self.engine.shutdown();
    }

    fn save_settings(&self) {
        let settings = self.settings.lock().unwrap().clone();
        match serde_json::to_string_pretty(&settings) {
            Ok(text) => {
                if let Err(e) = fs::write(&self.settings_path, text) {
                    log::warn!("não foi possível salvar as configurações de áudio: {e}");
                }
            }
            Err(e) => log::warn!("configurações de áudio inválidas: {e}"),
        }
    }

    fn load(&self, req: LoadRequest) -> Result<LoadedInfo, String> {
        let id = self.load_id.fetch_add(1, Ordering::SeqCst) + 1;
        let rate = self
            .engine
            .output_info()
            .ok_or("nenhum dispositivo de saída disponível (veja Configurações de áudio)")?
            .sample_rate;
        // As duas faixas são decodificadas em paralelo e passam pelo mesmo processamento,
        // o que mantém a sincronia entre elas.
        let (inst, voc) = std::thread::scope(|s| {
            let inst = s.spawn(|| audio_engine::load_track(Path::new(&req.instrumental), rate));
            let voc = req
                .vocals
                .as_ref()
                .map(|v| s.spawn(move || audio_engine::load_track(Path::new(v), rate)));
            (inst.join(), voc.map(|h| h.join()))
        });
        let inst = inst.map_err(|_| "falha ao decodificar o instrumental")??;
        let vocals = match voc {
            Some(r) => Some(r.map_err(|_| "falha ao decodificar a voz")??),
            None => None,
        };
        if self.load_id.load(Ordering::SeqCst) != id {
            return Err("cancelado: outra música foi pedida".into());
        }
        let loaded = Arc::new(Loaded::new(rate, vocals, inst));
        let info = LoadedInfo {
            duration: loaded.duration_secs(),
            has_vocals: loaded.vocals.is_some(),
        };
        self.engine
            .install(loaded, req.start.unwrap_or(0.0), req.autoplay)?;
        *self.current.lock().unwrap() = Some(req);
        Ok(info)
    }

    fn monitoring(&self) -> bool {
        self.voice
            .lock()
            .unwrap()
            .as_ref()
            .map_or(false, |v| v.monitor)
    }

    /// Abre ou fecha o microfone conforme o que a interface precisa (cantar ou testar)
    /// e aplica os ajustes de voz. O microfone só fica aberto enquanto é usado.
    fn apply_voice(&self, voice: Option<VoiceSettings>) -> Result<Option<MicInfo>, String> {
        if let Some(v) = voice {
            *self.voice.lock().unwrap() = Some(v);
        }
        let current = self.voice.lock().unwrap().clone();
        let want_mic = self.monitoring() || self.mic_testing.load(Ordering::SeqCst);
        let info = if want_mic {
            let name = self.settings.lock().unwrap().input_device.clone();
            Some(self.engine.start_mic(name)?)
        } else {
            self.engine.stop_mic();
            None
        };
        if let Some(v) = current {
            self.engine.set_voice(v);
        }
        Ok(info)
    }

    fn set_output(&self, name: Option<String>) -> Result<OutputInfo, String> {
        let (position, was_playing) = self.engine.position();
        let info = self.engine.set_output(name.clone())?;
        self.settings.lock().unwrap().output_device = name;
        self.save_settings();
        // A música carregada está na taxa da saída anterior: refaz na nova e volta ao ponto.
        let current = self.current.lock().unwrap().clone();
        if let Some(mut req) = current {
            req.start = Some(position);
            req.autoplay = was_playing;
            self.load(req)?;
        }
        Ok(info)
    }
}

type Blocking<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Blocking<T> + Send + 'static,
) -> Blocking<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn player_devices(state: State<'_, Arc<Player>>) -> Blocking<DevicesReply> {
    let p = state.inner().clone();
    blocking(move || {
        let settings = p.settings.lock().unwrap().clone();
        Ok(DevicesReply {
            devices: p.engine.list_devices()?,
            output_device: settings.output_device,
            input_device: settings.input_device,
            output: p.engine.output_info(),
        })
    })
    .await
}

#[tauri::command]
pub async fn player_set_output(
    state: State<'_, Arc<Player>>,
    name: Option<String>,
) -> Blocking<OutputInfo> {
    let p = state.inner().clone();
    blocking(move || p.set_output(name)).await
}

#[tauri::command]
pub async fn player_set_input(state: State<'_, Arc<Player>>, name: Option<String>) -> Blocking<()> {
    let p = state.inner().clone();
    blocking(move || {
        p.settings.lock().unwrap().input_device = name;
        p.save_settings();
        // Com o microfone aberto, troca para o novo dispositivo na hora.
        if p.monitoring() || p.mic_testing.load(Ordering::SeqCst) {
            p.apply_voice(None)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn player_load(
    state: State<'_, Arc<Player>>,
    request: LoadRequest,
) -> Blocking<LoadedInfo> {
    let p = state.inner().clone();
    blocking(move || p.load(request)).await
}

#[tauri::command]
pub fn player_play(state: State<'_, Arc<Player>>) {
    state.engine.play();
}

#[tauri::command]
pub fn player_pause(state: State<'_, Arc<Player>>) {
    state.engine.pause();
}

#[tauri::command]
pub fn player_seek(state: State<'_, Arc<Player>>, seconds: f64) {
    state.engine.seek(seconds);
}

#[tauri::command]
pub fn player_set_mix(state: State<'_, Arc<Player>>, mix: MixArg) {
    state.engine.set_mix(Mix {
        master: mix.master,
        vocal: mix.vocal,
        instrumental: mix.instrumental,
    });
}

/// Liga/desliga a medição do microfone (só nível, sem tocar o som). Devolve o nome do dispositivo.
#[tauri::command]
pub async fn player_mic_test(
    state: State<'_, Arc<Player>>,
    enable: bool,
) -> Blocking<Option<String>> {
    let p = state.inner().clone();
    blocking(move || {
        p.mic_testing.store(enable, Ordering::SeqCst);
        let info = p.apply_voice(None)?;
        Ok(info.map(|i| i.name))
    })
    .await
}

/// Ajustes da voz ao vivo. Com `monitor` ligado abre o microfone e a voz sai junto com a música;
/// desligado, fecha o microfone (a menos que o teste das configurações esteja aberto).
#[tauri::command]
pub async fn player_voice(
    state: State<'_, Arc<Player>>,
    settings: VoiceSettings,
) -> Blocking<Option<MicInfo>> {
    let p = state.inner().clone();
    blocking(move || p.apply_voice(Some(settings))).await
}
