//! Decodificação com o symphonia. A saída é sempre estéreo intercalado (L R L R...) em f32.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct Decoded {
    pub rate: u32,
    /// Estéreo intercalado.
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
}

pub fn decode_file(path: &Path) -> Result<Decoded, String> {
    let file =
        File::open(path).map_err(|e| format!("não foi possível abrir {}: {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("formato de áudio não reconhecido ({}): {e}", path.display()))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or("o arquivo não tem faixa de áudio")?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("codec não suportado: {e}"))?;

    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut samples: Vec<f32> = Vec::new();
    if let (Some(n), Some(r)) = (track.codec_params.n_frames, track.codec_params.sample_rate) {
        if r > 0 {
            samples.reserve(n as usize * 2);
        }
    }
    let mut buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(format!("erro ao ler o arquivo: {e}")),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // pacote ruim: pula
            Err(SymError::IoError(_)) => continue,
            Err(e) => return Err(format!("erro de decodificação: {e}")),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count();
        if buf
            .as_ref()
            .map_or(true, |b| b.capacity() < decoded.capacity() * channels)
        {
            buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        let data = b.samples();
        match channels {
            1 => samples.extend(data.iter().flat_map(|&s| [s, s])),
            2 => samples.extend_from_slice(data),
            n => samples.extend(data.chunks_exact(n).flat_map(|f| [f[0], f[1]])),
        }
    }

    if samples.is_empty() || rate == 0 {
        return Err(format!("nenhum áudio decodificado em {}", path.display()));
    }
    Ok(Decoded { rate, samples })
}
