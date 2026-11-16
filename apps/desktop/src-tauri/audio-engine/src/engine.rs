//! A thread do motor: é dona dos dispositivos cpal (os `Stream` não podem mudar de thread) e
//! recebe comandos por um canal. Quem chama usa o `Engine`, que é só um "controle remoto".

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig,
};
use serde::{Deserialize, Serialize};

use crate::playback::{Loaded, Mix, Playback, Shared};
use crate::voice::{Ring, VoiceOut, VoiceShared, MODE_BALANCED, MODE_MINIMAL};

/// Intervalo dos eventos de estado para a interface (~30 Hz).
const TICK: Duration = Duration::from_millis(33);

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MicInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Ajustes da voz ao vivo (vindos da interface).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSettings {
    /// Ouvir a voz na saída.
    pub monitor: bool,
    /// Volume do retorno, ganho linear (0 a 1,5).
    pub volume: f32,
    /// Ganho de entrada, linear.
    pub input_gain: f32,
    pub gate_on: bool,
    /// Limiar do portão, amplitude linear.
    pub gate_threshold: f32,
    /// Efeitos (todos opcionais: sem eles a voz passa limpa).
    #[serde(default)]
    pub eq_low_db: f32,
    #[serde(default)]
    pub eq_mid_db: f32,
    #[serde(default)]
    pub eq_high_db: f32,
    /// Compressão, 0 a 1.
    #[serde(default)]
    pub comp: f32,
    /// Reverb e tamanho da sala, 0 a 1.
    #[serde(default)]
    pub reverb: f32,
    #[serde(default = "default_reverb_size")]
    pub reverb_size: f32,
    /// Eco (volume 0 a 1) e tempo entre repetições (ms).
    #[serde(default)]
    pub echo: f32,
    #[serde(default = "default_echo_ms")]
    pub echo_ms: f32,
    /// Autotune: ligado, intensidade (0 a 1), velocidade (0 a 1) e notas permitidas (12 bits).
    #[serde(default)]
    pub tune_on: bool,
    #[serde(default)]
    pub tune_amount: f32,
    #[serde(default = "default_tune_speed")]
    pub tune_speed: f32,
    #[serde(default = "default_tune_scale")]
    pub tune_scale: u32,
    /// Modo de latência da voz: 0 = estável, 1 = equilibrado, 2 = mínimo.
    #[serde(default)]
    pub latency_mode: u32,
}

fn default_tune_speed() -> f32 {
    0.35
}

fn default_tune_scale() -> u32 {
    crate::tune::CHROMATIC
}

fn default_reverb_size() -> f32 {
    0.5
}

fn default_echo_ms() -> f32 {
    300.0
}

/// Eventos enviados à interface.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    State {
        position: f64,
        duration: f64,
        playing: bool,
        peak_l: f32,
        peak_r: f32,
    },
    Ended,
    Error {
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    MicLevel {
        peak: f32,
    },
    /// Medidas da ponte microfone → saída (~2 vezes por segundo, com o microfone aberto).
    #[serde(rename_all = "camelCase")]
    Voice {
        /// Estoque da ponte em ms: é o atraso que o app acrescenta (fora o do dispositivo).
        bridge_ms: f32,
        target_ms: f32,
        underruns: u64,
        /// Atraso extra do autotune em ms (0 com o autotune desligado).
        tune_ms: f32,
    },
    /// Atrasos que os dispositivos informam (~2 vezes por segundo): da captura do microfone até o
    /// app, e do app até o som sair. Servem para estimar a latência total e alinhar a letra.
    #[serde(rename_all = "camelCase")]
    Latency {
        input_ms: f32,
        output_ms: f32,
        /// Tamanho médio dos blocos de entrada e saída (ms).
        input_period_ms: f32,
        output_period_ms: f32,
    },
}

type Reply<T> = Sender<Result<T, String>>;

enum Cmd {
    ListDevices(Reply<DeviceList>),
    SetOutput(Option<String>, Reply<OutputInfo>),
    OutputInfo(Sender<Option<OutputInfo>>),
    Install {
        loaded: Arc<Loaded>,
        start_secs: f64,
        autoplay: bool,
        reply: Reply<()>,
    },
    Play,
    Pause,
    Seek(f64),
    SetMix(Mix),
    Position(Sender<(f64, bool)>),
    StartMic(Option<String>, Reply<MicInfo>),
    StopMic,
    SetVoice(VoiceSettings),
    StreamError(String),
    Shutdown,
}

/// Controle remoto do motor (pode ser clonado entre threads).
#[derive(Clone)]
pub struct Engine {
    tx: Sender<Cmd>,
}

impl Engine {
    /// Inicia a thread do motor. `on_event` é chamado de dentro dela (deve ser rápido).
    pub fn start(on_event: impl Fn(Event) + Send + 'static) -> Engine {
        crate::prio::boost_process();
        let (tx, rx) = mpsc::channel();
        let tx_thread = tx.clone();
        std::thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || EngineThread::new(tx_thread, Box::new(on_event)).run(rx))
            .expect("thread do motor de áudio");
        Engine { tx }
    }

    fn ask<T>(&self, make: impl FnOnce(Sender<T>) -> Cmd) -> Result<T, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(make(reply))
            .map_err(|_| "o motor de áudio parou".to_string())?;
        rx.recv_timeout(Duration::from_secs(20))
            .map_err(|_| "o motor de áudio não respondeu".to_string())
    }

    pub fn list_devices(&self) -> Result<DeviceList, String> {
        self.ask(Cmd::ListDevices)?
    }

    /// `None` = dispositivo padrão do sistema.
    pub fn set_output(&self, name: Option<String>) -> Result<OutputInfo, String> {
        self.ask(|r| Cmd::SetOutput(name, r))?
    }

    pub fn output_info(&self) -> Option<OutputInfo> {
        self.ask(Cmd::OutputInfo).ok().flatten()
    }

    /// Entrega uma música já na taxa da saída. Com `autoplay`, começa a tocar.
    pub fn install(
        &self,
        loaded: Arc<Loaded>,
        start_secs: f64,
        autoplay: bool,
    ) -> Result<(), String> {
        self.ask(|reply| Cmd::Install {
            loaded,
            start_secs,
            autoplay,
            reply,
        })?
    }

    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek(secs));
    }
    pub fn set_mix(&self, mix: Mix) {
        let _ = self.tx.send(Cmd::SetMix(mix));
    }

    /// (posição em segundos, está tocando)
    pub fn position(&self) -> (f64, bool) {
        self.ask(Cmd::Position).unwrap_or((0.0, false))
    }

    /// Abre o microfone (mede o nível; o som só sai na saída com o retorno ligado em `set_voice`).
    /// Se já estiver aberto o mesmo dispositivo, devolve os dados dele.
    pub fn start_mic(&self, name: Option<String>) -> Result<MicInfo, String> {
        self.ask(|r| Cmd::StartMic(name, r))?
    }
    pub fn stop_mic(&self) {
        let _ = self.tx.send(Cmd::StopMic);
    }
    pub fn set_voice(&self, settings: VoiceSettings) {
        let _ = self.tx.send(Cmd::SetVoice(settings));
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

/// Microfone aberto: o `Stream` escreve na `Ring`, e a saída lê dela.
struct Mic {
    _stream: Stream,
    peak: Arc<AtomicU32>,
    ring: Arc<Ring>,
    info: MicInfo,
    wanted: Option<String>,
}

struct EngineThread {
    tx: Sender<Cmd>,
    on_event: Box<dyn Fn(Event) + Send>,
    shared: Arc<Shared>,
    output: Option<(Stream, OutputInfo)>,
    /// Música atual (para a duração) — o callback guarda a própria referência.
    loaded: Option<Arc<Loaded>>,
    /// Músicas já trocadas; a memória é liberada aqui, nunca no callback de áudio.
    retired: Vec<Arc<Loaded>>,
    mic: Option<Mic>,
    /// Saída pedida pelo usuário (None = padrão do sistema), para reabrir ao mudar o buffer.
    output_wanted: Option<Option<String>>,
    /// Modo de latência atual (`MODE_*`).
    latency_mode: u32,
    /// Filas de microfones já fechados; a memória é liberada aqui, nunca no callback.
    retired_rings: Vec<Arc<Ring>>,
    ticks: u32,
    last_state: Option<Event>,
}

impl EngineThread {
    fn new(tx: Sender<Cmd>, on_event: Box<dyn Fn(Event) + Send>) -> Self {
        EngineThread {
            tx,
            on_event,
            shared: Shared::new(Mix::default()),
            output: None,
            loaded: None,
            retired: Vec::new(),
            mic: None,
            output_wanted: None,
            latency_mode: 0,
            retired_rings: Vec::new(),
            ticks: 0,
            last_state: None,
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        // Saída padrão desde o início; se falhar, o erro aparece quando alguém tentar tocar.
        if let Err(e) = self.set_output(None) {
            log::warn!("saída de áudio indisponível: {e}");
        }
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
    }

    fn emit(&self, e: Event) {
        (self.on_event)(e);
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::ListDevices(r) => {
                let _ = r.send(list_devices());
            }
            Cmd::SetOutput(name, r) => {
                let _ = r.send(self.set_output(name));
            }
            Cmd::OutputInfo(r) => {
                let _ = r.send(self.output.as_ref().map(|(_, i)| i.clone()));
            }
            Cmd::Install {
                loaded,
                start_secs,
                autoplay,
                reply,
            } => {
                let _ = reply.send(self.install(loaded, start_secs, autoplay));
            }
            Cmd::Play => {
                if self.loaded.is_some() {
                    self.shared.playing.store(true, Ordering::Release);
                }
            }
            Cmd::Pause => self.shared.playing.store(false, Ordering::Release),
            Cmd::Seek(secs) => {
                if let Some(l) = &self.loaded {
                    let frame = (secs.max(0.0) * l.rate as f64) as usize;
                    self.shared.request_seek(frame.min(l.frames));
                }
            }
            Cmd::SetMix(mix) => self.shared.set_mix(mix),
            Cmd::Position(r) => {
                let _ = r.send(self.position());
            }
            Cmd::StartMic(name, r) => {
                let _ = r.send(self.start_mic(name));
            }
            Cmd::StopMic => self.stop_mic(),
            Cmd::SetVoice(v) => self.set_voice(v),
            Cmd::StreamError(message) => {
                log::error!("erro no dispositivo de áudio: {message}");
                self.shared.playing.store(false, Ordering::Release);
                self.emit(Event::Error { message });
            }
            Cmd::Shutdown => {}
        }
    }

    fn position(&self) -> (f64, bool) {
        let rate = self.output.as_ref().map_or(44_100, |(_, i)| i.sample_rate) as f64;
        (
            self.shared.position.load(Ordering::Relaxed) as f64 / rate,
            self.shared.playing.load(Ordering::Acquire),
        )
    }

    fn tick(&mut self) {
        self.retired.retain(|a| Arc::strong_count(a) > 1);
        if self.shared.ended.swap(false, Ordering::AcqRel) {
            self.emit(Event::Ended);
        }
        self.retired_rings.retain(|r| Arc::strong_count(r) > 1);
        self.ticks = self.ticks.wrapping_add(1);
        if let Some(mic) = &self.mic {
            let p = f32::from_bits(mic.peak.swap(0, Ordering::Relaxed));
            self.emit(Event::MicLevel { peak: p });
            if self.ticks % 15 == 0 {
                let v = &self.shared.voice;
                self.emit(Event::Voice {
                    bridge_ms: v.fill_ms.load(),
                    target_ms: v.target_ms.load(),
                    underruns: v.underruns.load(Ordering::Relaxed),
                    tune_ms: v.tune.latency_ms.load(),
                });
            }
        }
        if self.ticks % 15 == 0 && self.output.is_some() {
            let v = &self.shared.voice;
            let mic_on = self.mic.is_some();
            self.emit(Event::Latency {
                input_ms: if mic_on { v.in_latency_ms.load() } else { 0.0 },
                output_ms: v.out_latency_ms.load(),
                input_period_ms: if mic_on { v.in_period_ms.load() } else { 0.0 },
                output_period_ms: v.out_period_ms.load(),
            });
        }
        let (position, playing) = self.position();
        let (peak_l, peak_r) = self.shared.take_peaks();
        let duration = self.loaded.as_ref().map_or(0.0, |l| l.duration_secs());
        let state = Event::State {
            position,
            duration,
            playing,
            peak_l,
            peak_r,
        };
        // Parado e sem mudança: não precisa avisar a interface a cada 33 ms.
        let quiet = |e: &Event| match e {
            Event::State {
                position,
                duration,
                playing,
                peak_l,
                peak_r,
            } => (
                *position,
                *duration,
                *playing,
                *peak_l > 0.0 || *peak_r > 0.0,
            ),
            _ => (0.0, 0.0, false, false),
        };
        if self.last_state.as_ref().map(quiet) != Some(quiet(&state))
            || playing
            || peak_l > 0.0
            || peak_r > 0.0
        {
            self.emit(state.clone());
        }
        self.last_state = Some(state);
    }

    fn install(
        &mut self,
        loaded: Arc<Loaded>,
        start_secs: f64,
        autoplay: bool,
    ) -> Result<(), String> {
        let rate = self
            .output
            .as_ref()
            .map(|(_, i)| i.sample_rate)
            .ok_or("nenhum dispositivo de saída disponível")?;
        if loaded.rate != rate {
            return Err(format!(
                "a música está em {} Hz, mas a saída usa {rate} Hz (recarregue a música)",
                loaded.rate
            ));
        }
        let start = ((start_secs.max(0.0) * rate as f64) as usize).min(loaded.frames);
        if let Some(old) = self.loaded.replace(loaded.clone()) {
            self.retired.push(old);
        }
        self.shared.ended.store(false, Ordering::Release);
        self.shared.playing.store(autoplay, Ordering::Release);
        self.shared.install(loaded, start);
        Ok(())
    }

    fn set_output(&mut self, name: Option<String>) -> Result<OutputInfo, String> {
        self.output_wanted = Some(name.clone());
        let was_playing = self.shared.playing.swap(false, Ordering::AcqRel);
        self.output = None; // fecha o dispositivo anterior primeiro (o modo exclusivo exige)
        let _ = was_playing;
        let host = cpal::default_host();
        let device = match &name {
            Some(n) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map_or(false, |dn| &dn == n))
                .ok_or_else(|| format!("dispositivo de saída não encontrado: {n}"))?,
            None => host
                .default_output_device()
                .ok_or("nenhum dispositivo de saída padrão")?,
        };
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let info = OutputInfo {
            name: device.name().unwrap_or_else(|_| "(sem nome)".into()),
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        };
        // Música já carregada tem a taxa antiga: quem chamou precisa recarregá-la.
        if let Some(old) = self.loaded.take() {
            self.retired.push(old);
        }
        self.shared.position.store(0, Ordering::Relaxed);
        self.shared.seek_to.store(-1, Ordering::Release);
        // A voz ao vivo precisa da taxa da nova saída: refaz o lado da saída da ponte.
        if let Some(mic) = &self.mic {
            self.shared.install_voice(Some(VoiceOut::new(
                mic.ring.clone(),
                self.shared.voice.clone(),
                mic.info.sample_rate,
                info.sample_rate,
            )));
        }
        let frames = buffer_frames(self.latency_mode, info.sample_rate);
        let stream = open_output(&device, &config, frames, &self.shared, &self.tx)?;
        stream
            .play()
            .map_err(|e| format!("não foi possível iniciar a saída: {e}"))?;
        self.output = Some((stream, info.clone()));
        log::info!(
            "saída de áudio: {} ({} Hz, {} canais)",
            info.name,
            info.sample_rate,
            info.channels
        );
        Ok(info)
    }

    fn stop_mic(&mut self) {
        if let Some(mic) = self.mic.take() {
            self.shared.install_voice(None);
            self.shared.voice.monitor.store(false, Ordering::Relaxed);
            self.retired_rings.push(mic.ring.clone());
        }
    }

    /// Troca o modo de latência: ajusta a ponte na hora e reabre os dispositivos com o novo
    /// tamanho de buffer (a música e a posição são mantidas).
    fn set_latency_mode(&mut self, mode: u32) {
        let mode = mode.min(MODE_MINIMAL);
        if mode == self.latency_mode {
            return;
        }
        let old_frames = self
            .output
            .as_ref()
            .and_then(|(_, i)| buffer_frames(self.latency_mode, i.sample_rate));
        self.latency_mode = mode;
        self.shared
            .voice
            .latency_mode
            .store(mode, Ordering::Relaxed);
        let new_frames = self
            .output
            .as_ref()
            .and_then(|(_, i)| buffer_frames(mode, i.sample_rate));
        if old_frames == new_frames {
            return; // o buffer é o mesmo: só a ponte mudou
        }
        let loaded = self.loaded.clone();
        let (position, playing) = self.position();
        if let Some(name) = self.output_wanted.clone() {
            match self.set_output(name) {
                Ok(_) => {
                    if let Some(l) = loaded {
                        if let Err(e) = self.install(l, position, playing) {
                            log::warn!("não foi possível retomar a música: {e}");
                        }
                    }
                }
                Err(e) => log::warn!("não foi possível reabrir a saída: {e}"),
            }
        }
        if let Some(wanted) = self.mic.as_ref().map(|m| m.wanted.clone()) {
            self.stop_mic();
            if let Err(e) = self.start_mic(wanted) {
                log::warn!("não foi possível reabrir o microfone: {e}");
            }
        }
        log::info!("modo de latência: {mode} (buffer {new_frames:?} quadros)");
    }

    fn set_voice(&mut self, v: VoiceSettings) {
        self.set_latency_mode(v.latency_mode);
        let shared = &self.shared.voice;
        shared.volume.store(v.volume.clamp(0.0, 1.5));
        shared.input_gain.store(v.input_gain.clamp(0.0, 16.0));
        shared.gate_on.store(v.gate_on, Ordering::Relaxed);
        shared
            .gate_threshold
            .store(v.gate_threshold.clamp(0.0, 0.5));
        let fx = &shared.fx;
        fx.eq_low_db.store(v.eq_low_db.clamp(-12.0, 12.0));
        fx.eq_mid_db.store(v.eq_mid_db.clamp(-12.0, 12.0));
        fx.eq_high_db.store(v.eq_high_db.clamp(-12.0, 12.0));
        fx.comp.store(v.comp.clamp(0.0, 1.0));
        fx.reverb.store(v.reverb.clamp(0.0, 1.0));
        fx.reverb_size.store(v.reverb_size.clamp(0.0, 1.0));
        fx.echo.store(v.echo.clamp(0.0, 1.0));
        fx.echo_ms.store(v.echo_ms.clamp(60.0, 800.0));
        let tune = &shared.tune;
        tune.on.store(v.tune_on, Ordering::Relaxed);
        tune.amount.store(v.tune_amount.clamp(0.0, 1.0));
        tune.speed.store(v.tune_speed.clamp(0.0, 1.0));
        tune.scale_mask
            .store(v.tune_scale & crate::tune::CHROMATIC, Ordering::Relaxed);
        // O retorno só liga com o microfone aberto.
        let on = v.monitor && self.mic.is_some();
        shared.monitor.store(on, Ordering::Relaxed);
    }

    fn start_mic(&mut self, name: Option<String>) -> Result<MicInfo, String> {
        if let Some(mic) = &self.mic {
            if mic.wanted == name {
                return Ok(mic.info.clone());
            }
        }
        self.stop_mic();
        let out_rate = self
            .output
            .as_ref()
            .map(|(_, i)| i.sample_rate)
            .ok_or("nenhum dispositivo de saída disponível")?;
        let host = cpal::default_host();
        let device = match &name {
            Some(n) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map_or(false, |dn| &dn == n))
                .ok_or_else(|| format!("dispositivo de entrada não encontrado: {n}"))?,
            None => host
                .default_input_device()
                .ok_or("nenhum microfone padrão")?,
        };
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let info = MicInfo {
            name: device.name().unwrap_or_else(|_| "(sem nome)".into()),
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        };
        let peak = Arc::new(AtomicU32::new(0));
        let ring = Ring::new();
        let frames = buffer_frames(self.latency_mode, info.sample_rate);
        let stream = open_input(
            &device,
            &config,
            frames,
            &ring,
            &peak,
            &self.shared.voice,
            &self.tx,
        )?;
        stream
            .play()
            .map_err(|e| format!("não foi possível abrir o microfone: {e}"))?;
        self.shared.install_voice(Some(VoiceOut::new(
            ring.clone(),
            self.shared.voice.clone(),
            info.sample_rate,
            out_rate,
        )));
        log::info!(
            "microfone: {} ({} Hz, {} canais)",
            info.name,
            info.sample_rate,
            info.channels
        );
        self.mic = Some(Mic {
            _stream: stream,
            peak,
            ring,
            info: info.clone(),
            wanted: name,
        });
        Ok(info)
    }
}

/// Tamanho do buffer (em quadros) pedido ao dispositivo em cada modo; `None` = o padrão dele.
fn buffer_frames(mode: u32, rate: u32) -> Option<u32> {
    match mode {
        MODE_BALANCED => Some(rate / 200),          // ~5 ms
        MODE_MINIMAL => Some((rate / 400).max(32)), // ~2,5 ms
        _ => None,
    }
}

/// A configuração do dispositivo, com o buffer pedido (dentro do que ele aceita).
fn with_buffer(base: StreamConfig, supported: &SupportedBufferSize, frames: u32) -> StreamConfig {
    let frames = match supported {
        SupportedBufferSize::Range { min, max } => frames.clamp(*min, *max),
        SupportedBufferSize::Unknown => frames,
    };
    StreamConfig {
        buffer_size: BufferSize::Fixed(frames),
        ..base
    }
}

/// Abre a saída; se o dispositivo recusar o buffer pedido, tenta de novo com o padrão dele.
fn open_output(
    device: &cpal::Device,
    supported: &SupportedStreamConfig,
    frames: Option<u32>,
    shared: &Arc<Shared>,
    tx: &Sender<Cmd>,
) -> Result<Stream, String> {
    let rate = supported.sample_rate().0;
    let attempt = |cfg: StreamConfig| -> Result<Stream, String> {
        let playback = Playback::new(shared.clone(), rate);
        match supported.sample_format() {
            SampleFormat::F32 => build_output::<f32>(device, &cfg, playback, tx.clone()),
            SampleFormat::I16 => build_output::<i16>(device, &cfg, playback, tx.clone()),
            SampleFormat::U16 => build_output::<u16>(device, &cfg, playback, tx.clone()),
            SampleFormat::I32 => build_output::<i32>(device, &cfg, playback, tx.clone()),
            other => Err(format!("formato de amostra não suportado: {other:?}")),
        }
    };
    if let Some(f) = frames {
        let cfg = with_buffer(supported.config(), supported.buffer_size(), f);
        match attempt(cfg) {
            Ok(stream) => return Ok(stream),
            Err(e) => log::warn!("a saída recusou o buffer de {f} quadros ({e}); usando o padrão"),
        }
    }
    attempt(supported.config())
}

/// Abre o microfone; mesma regra de volta ao padrão se o buffer pedido for recusado.
fn open_input(
    device: &cpal::Device,
    supported: &SupportedStreamConfig,
    frames: Option<u32>,
    ring: &Arc<Ring>,
    peak: &Arc<AtomicU32>,
    voice: &Arc<VoiceShared>,
    tx: &Sender<Cmd>,
) -> Result<Stream, String> {
    let channels = supported.channels() as usize;
    let rate = supported.sample_rate().0;
    let attempt = |cfg: StreamConfig| -> Result<Stream, String> {
        let sink = MicSink {
            ring: ring.clone(),
            peak: peak.clone(),
            channels,
            voice: voice.clone(),
            rate,
        };
        match supported.sample_format() {
            SampleFormat::F32 => build_input::<f32>(device, &cfg, sink, tx.clone()),
            SampleFormat::I16 => build_input::<i16>(device, &cfg, sink, tx.clone()),
            SampleFormat::U16 => build_input::<u16>(device, &cfg, sink, tx.clone()),
            SampleFormat::I32 => build_input::<i32>(device, &cfg, sink, tx.clone()),
            other => Err(format!("formato de amostra não suportado: {other:?}")),
        }
    };
    if let Some(f) = frames {
        let cfg = with_buffer(supported.config(), supported.buffer_size(), f);
        match attempt(cfg) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                log::warn!("o microfone recusou o buffer de {f} quadros ({e}); usando o padrão")
            }
        }
    }
    attempt(supported.config())
}

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut playback: Playback,
    errors: Sender<Cmd>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32> + Sample,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;
    let voice = playback.voice_shared();
    let mut boosted = false;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    crate::prio::boost_audio_thread("saída");
                }
                let ts = info.timestamp();
                let ahead = ts
                    .playback
                    .duration_since(&ts.callback)
                    .map(|d| d.as_secs_f64());
                voice.note_block(false, data.len() / channels.max(1), rate, ahead);
                playback.render(data, channels)
            },
            move |e| {
                let _ = errors.send(Cmd::StreamError(e.to_string()));
            },
            None,
        )
        .map_err(|e| format!("não foi possível abrir a saída: {e}"))
}

/// O que o callback do microfone faz com cada bloco: mistura para mono, mede o pico e escreve na ponte.
struct MicSink {
    ring: Arc<Ring>,
    peak: Arc<AtomicU32>,
    channels: usize,
    voice: Arc<VoiceShared>,
    rate: u32,
}

impl MicSink {
    fn push<T>(&self, data: &[T], age_s: Option<f64>)
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let ch = self.channels.max(1);
        let frames = data.len() / ch;
        self.voice.note_block(true, frames, self.rate, age_s);
        let mut p = 0.0f32;
        self.ring.write(frames, |i| {
            let frame = &data[i * ch..(i + 1) * ch];
            let mut sum = 0.0f32;
            for &s in frame {
                sum += f32::from_sample(s);
            }
            let mono = sum / ch as f32;
            p = p.max(mono.abs());
            mono
        });
        self.peak.fetch_max(p.to_bits(), Ordering::Relaxed);
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sink: MicSink,
    errors: Sender<Cmd>,
) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut boosted = false;
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    crate::prio::boost_audio_thread("microfone");
                }
                let ts = info.timestamp();
                let age = ts
                    .callback
                    .duration_since(&ts.capture)
                    .map(|d| d.as_secs_f64());
                sink.push(data, age)
            },
            move |e| {
                let _ = errors.send(Cmd::StreamError(e.to_string()));
            },
            None,
        )
        .map_err(|e| format!("não foi possível abrir o microfone: {e}"))
}

fn list_devices() -> Result<DeviceList, String> {
    let host = cpal::default_host();
    let default_out = host.default_output_device().and_then(|d| d.name().ok());
    let default_in = host.default_input_device().and_then(|d| d.name().ok());
    let collect = |devices: Vec<cpal::Device>, default: &Option<String>| -> Vec<DeviceInfo> {
        let mut list: Vec<DeviceInfo> = devices
            .iter()
            .filter_map(|d| d.name().ok())
            .map(|name| DeviceInfo {
                is_default: Some(&name) == default.as_ref(),
                name,
            })
            .collect();
        list.dedup_by(|a, b| a.name == b.name);
        list
    };
    Ok(DeviceList {
        outputs: collect(
            host.output_devices().map_err(|e| e.to_string())?.collect(),
            &default_out,
        ),
        inputs: collect(
            host.input_devices().map_err(|e| e.to_string())?.collect(),
            &default_in,
        ),
    })
}
