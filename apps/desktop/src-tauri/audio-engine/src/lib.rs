//! Motor de áudio do Karaoke Studio.
//!
//! - `decode`: arquivos (FLAC, MP3, WAV) para amostras f32 estéreo.
//! - `resample`: conversão de taxa de amostragem (feita ao carregar, não no callback).
//! - `playback`: o mixer em si (lógica pura, sem dispositivo; roda dentro do callback de áudio).
//! - `engine`: a thread que é dona dos dispositivos cpal e recebe comandos.

pub mod decode;
pub mod engine;
pub mod playback;
pub mod resample;

pub use engine::{DeviceInfo, DeviceList, Engine, Event, OutputInfo};
pub use playback::{Loaded, Mix};

/// Decodifica um arquivo e o converte para a taxa do dispositivo de saída.
pub fn load_track(path: &std::path::Path, target_rate: u32) -> Result<Vec<f32>, String> {
    let decoded = decode::decode_file(path)?;
    Ok(resample::to_rate(
        &decoded.samples,
        decoded.rate,
        target_rate,
    ))
}
