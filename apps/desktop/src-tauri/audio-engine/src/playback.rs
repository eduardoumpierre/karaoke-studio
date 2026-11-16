//! O mixer. Tudo aqui é lógica pura (sem dispositivo) e roda dentro do callback de áudio,
//! então não aloca memória, não faz I/O e não espera por travas (só `try_lock`).
//!
//! Regras que evitam estalos:
//! - play, pause, seek e troca de música passam por uma rampa de ~8 ms (fade);
//! - mudanças de volume são suavizadas (~10 ms) em vez de saltar;
//! - o resultado passa por um limitador suave, para a soma das duas faixas não estourar.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::{FromSample, Sample};

use crate::voice::{VoiceOut, VoiceShared};

/// Duração da rampa de fade em segundos.
const FADE_SECONDS: f32 = 0.008;
/// Constante de tempo da suavização de volume em segundos.
const VOLUME_SECONDS: f32 = 0.01;

/// Música pronta para tocar, já na taxa de amostragem do dispositivo.
pub struct Loaded {
    pub rate: u32,
    pub frames: usize,
    /// Estéreo intercalado. `None` quando só existe o áudio original (sem separação).
    pub vocals: Option<Vec<f32>>,
    pub instrumental: Vec<f32>,
}

impl Loaded {
    /// Iguala o tamanho das duas faixas (a menor é completada com silêncio).
    pub fn new(rate: u32, vocals: Option<Vec<f32>>, mut instrumental: Vec<f32>) -> Self {
        let mut vocals = vocals;
        let len = instrumental
            .len()
            .max(vocals.as_ref().map_or(0, |v| v.len()));
        let len = len - len % 2;
        instrumental.resize(len, 0.0);
        if let Some(v) = vocals.as_mut() {
            v.resize(len, 0.0);
        }
        Loaded {
            rate,
            frames: len / 2,
            vocals,
            instrumental,
        }
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / self.rate as f64
    }
}

/// Volumes de 0.0 a 1.0 (a interface aplica a curva perceptual antes de enviar).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub master: f32,
    pub vocal: f32,
    pub instrumental: f32,
}

impl Default for Mix {
    fn default() -> Self {
        Mix {
            master: 1.0,
            vocal: 0.2,
            instrumental: 1.0,
        }
    }
}

#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        AtomicF32(AtomicU32::new(v.to_bits()))
    }
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Estado compartilhado entre a thread de controle e o callback de áudio.
pub struct Shared {
    pub playing: AtomicBool,
    pub master: AtomicF32,
    pub vocal: AtomicF32,
    pub instrumental: AtomicF32,
    /// Pedido de seek em frames; negativo = nenhum.
    pub seek_to: AtomicI64,
    /// Posição atual em frames.
    pub position: AtomicU64,
    /// Ativado pelo callback quando a música termina (a thread de controle limpa).
    pub ended: AtomicBool,
    /// Pico de saída desde a última leitura (bits de f32 não negativo; ordem dos bits = ordem dos valores).
    pub peak_l: AtomicU32,
    pub peak_r: AtomicU32,
    generation: AtomicU64,
    slot: Mutex<Option<(Arc<Loaded>, usize)>>,
    /// Ajustes e medidas da voz ao vivo.
    pub voice: Arc<VoiceShared>,
    voice_generation: AtomicU64,
    voice_slot: Mutex<Option<VoiceOut>>,
}

impl Shared {
    pub fn new(mix: Mix) -> Arc<Self> {
        Arc::new(Shared {
            playing: AtomicBool::new(false),
            master: AtomicF32::new(mix.master),
            vocal: AtomicF32::new(mix.vocal),
            instrumental: AtomicF32::new(mix.instrumental),
            seek_to: AtomicI64::new(-1),
            position: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            peak_l: AtomicU32::new(0),
            peak_r: AtomicU32::new(0),
            generation: AtomicU64::new(0),
            slot: Mutex::new(None),
            voice: VoiceShared::new(),
            voice_generation: AtomicU64::new(0),
            voice_slot: Mutex::new(None),
        })
    }

    /// Entrega uma música ao callback, que a assume com fade. `start_frame` é a posição inicial.
    pub fn install(&self, loaded: Arc<Loaded>, start_frame: usize) {
        *self.slot.lock().unwrap() = Some((loaded, start_frame));
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Liga (`Some`) ou desliga (`None`) a voz ao vivo no callback da saída.
    pub fn install_voice(&self, voice: Option<VoiceOut>) {
        *self.voice_slot.lock().unwrap() = voice;
        self.voice_generation.fetch_add(1, Ordering::Release);
    }

    pub fn set_mix(&self, mix: Mix) {
        self.master.store(mix.master.clamp(0.0, 1.5));
        self.vocal.store(mix.vocal.clamp(0.0, 1.5));
        self.instrumental.store(mix.instrumental.clamp(0.0, 1.5));
    }

    pub fn request_seek(&self, frame: usize) {
        self.seek_to.store(frame as i64, Ordering::Release);
    }

    /// Pico desde a última chamada (e zera).
    pub fn take_peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_l.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.peak_r.swap(0, Ordering::Relaxed)),
        )
    }
}

enum Pending {
    Seek(usize),
    Install(Arc<Loaded>, usize),
}

/// Limitador suave: transparente até 0,8; acima disso curva sem passar de 1,0.
#[inline]
fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

pub struct Playback {
    shared: Arc<Shared>,
    seen_generation: u64,
    seen_voice_generation: u64,
    voice: Option<VoiceOut>,
    loaded: Option<Arc<Loaded>>,
    pos: usize,
    pending: Option<Pending>,
    gain: f32,
    fade_step: f32,
    vol_coef: f32,
    v_master: f32,
    v_vocal: f32,
    v_instrumental: f32,
}

impl Playback {
    pub fn new(shared: Arc<Shared>, rate: u32) -> Self {
        let rate = rate as f32;
        let v_master = shared.master.load();
        let v_vocal = shared.vocal.load();
        let v_instrumental = shared.instrumental.load();
        Playback {
            seen_generation: shared.generation.load(Ordering::Acquire),
            seen_voice_generation: shared.voice_generation.load(Ordering::Acquire),
            voice: None,
            shared,
            loaded: None,
            pos: 0,
            pending: None,
            gain: 0.0,
            fade_step: 1.0 / (FADE_SECONDS * rate),
            vol_coef: 1.0 - (-1.0 / (VOLUME_SECONDS * rate)).exp(),
            v_master,
            v_vocal,
            v_instrumental,
        }
    }

    /// Pega uma música nova, se houver. Usa `try_lock`: se a thread de controle estiver
    /// mexendo no slot, tenta de novo no próximo callback (a troca atrasa alguns ms).
    fn poll_install(&mut self) {
        let generation = self.shared.generation.load(Ordering::Acquire);
        if generation == self.seen_generation {
            return;
        }
        if let Ok(mut slot) = self.shared.slot.try_lock() {
            if let Some((loaded, start)) = slot.take() {
                self.pending = Some(Pending::Install(loaded, start));
            }
            self.seen_generation = generation;
        }
    }

    /// Pega a voz nova (ou o pedido de desligar), se houver; mesmo esquema do `poll_install`.
    fn poll_voice(&mut self) {
        let generation = self.shared.voice_generation.load(Ordering::Acquire);
        if generation == self.seen_voice_generation {
            return;
        }
        if let Ok(mut slot) = self.shared.voice_slot.try_lock() {
            self.voice = slot.take();
            self.seen_voice_generation = generation;
        }
    }

    /// Preenche `out` (intercalado, `channels` canais) com o próximo trecho.
    pub fn render<T: Sample + FromSample<f32>>(&mut self, out: &mut [T], channels: usize) {
        let channels = channels.max(1);
        self.poll_install();
        self.poll_voice();
        if let Some(v) = &mut self.voice {
            v.begin_block();
        }
        let seek = self.shared.seek_to.swap(-1, Ordering::AcqRel);
        if seek >= 0 && self.loaded.is_some() {
            self.pending = Some(Pending::Seek(seek as usize));
        }
        let wants_play = self.shared.playing.load(Ordering::Acquire);
        let t_master = self.shared.master.load();
        let t_vocal = self.shared.vocal.load();
        let t_instrumental = self.shared.instrumental.load();
        let silence = T::from_sample(0.0f32);
        let (mut peak_l, mut peak_r) = (0.0f32, 0.0f32);
        let mut ended = false;

        for frame in out.chunks_mut(channels) {
            let target = if wants_play && !ended && self.loaded.is_some() && self.pending.is_none()
            {
                1.0
            } else {
                0.0
            };
            if self.gain < target {
                self.gain = (self.gain + self.fade_step).min(target);
            } else if self.gain > target {
                self.gain = (self.gain - self.fade_step).max(0.0);
            }
            // No silêncio total é seguro trocar de música ou de posição.
            if self.gain == 0.0 {
                if let Some(p) = self.pending.take() {
                    match p {
                        Pending::Install(l, start) => {
                            self.pos = start.min(l.frames);
                            self.loaded = Some(l);
                        }
                        Pending::Seek(f) => {
                            self.pos = f.min(self.loaded.as_ref().map_or(0, |l| l.frames));
                        }
                    }
                }
            }
            self.v_master += (t_master - self.v_master) * self.vol_coef;
            self.v_vocal += (t_vocal - self.v_vocal) * self.vol_coef;
            self.v_instrumental += (t_instrumental - self.v_instrumental) * self.vol_coef;

            let (mut l, mut r) = (0.0f32, 0.0f32);
            if self.gain > 0.0 {
                if let Some(track) = &self.loaded {
                    if self.pos >= track.frames {
                        ended = true;
                    } else {
                        let i = self.pos * 2;
                        l = track.instrumental[i] * self.v_instrumental;
                        r = track.instrumental[i + 1] * self.v_instrumental;
                        if let Some(v) = &track.vocals {
                            l += v[i] * self.v_vocal;
                            r += v[i + 1] * self.v_vocal;
                        }
                        let g = self.gain * self.v_master;
                        l *= g;
                        r *= g;
                        self.pos += 1;
                    }
                }
            }
            // A voz ao vivo soma com a música (que pode estar parada) e o limitador vale para os dois.
            let (vl, vr) = self.voice.as_mut().map_or((0.0, 0.0), |v| v.next_stereo());
            l = soft_limit(l + vl);
            r = soft_limit(r + vr);
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            match channels {
                1 => frame[0] = T::from_sample((l + r) * 0.5),
                _ => {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(r);
                    for extra in frame.iter_mut().skip(2) {
                        *extra = silence;
                    }
                }
            }
        }

        if ended {
            self.shared.playing.store(false, Ordering::Release);
            self.shared.ended.store(true, Ordering::Release);
        }
        self.shared
            .position
            .store(self.pos as u64, Ordering::Relaxed);
        self.shared
            .peak_l
            .fetch_max(peak_l.to_bits(), Ordering::Relaxed);
        self.shared
            .peak_r
            .fetch_max(peak_r.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn stereo(frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let (l, r) = f(i);
                [l, r]
            })
            .collect()
    }

    fn setup(vocals: Option<Vec<f32>>, instrumental: Vec<f32>) -> (Arc<Shared>, Playback) {
        let shared = Shared::new(Mix {
            master: 1.0,
            vocal: 1.0,
            instrumental: 1.0,
        });
        let mut pb = Playback::new(shared.clone(), RATE);
        shared.install(Arc::new(Loaded::new(RATE, vocals, instrumental)), 0);
        // Um buffer parado deixa a música ser assumida.
        let mut tmp = vec![0.0f32; 64];
        pb.render(&mut tmp, 2);
        (shared, pb)
    }

    fn render_n(pb: &mut Playback, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        pb.render(&mut out, 2);
        out
    }

    /// Renderiza `frames` em blocos de tamanhos variados, como um driver real faria.
    fn render_chunked(pb: &mut Playback, frames: usize) -> Vec<f32> {
        let sizes = [441, 480, 512, 1024, 333, 960, 128];
        let mut all = Vec::with_capacity(frames * 2);
        let (mut done, mut k) = (0, 0);
        while done < frames {
            let n = sizes[k % sizes.len()].min(frames - done);
            all.extend(render_n(pb, n));
            done += n;
            k += 1;
        }
        all
    }

    #[test]
    fn silent_until_played_and_position_does_not_advance() {
        let (shared, mut pb) = setup(None, stereo(4800, |_| (0.5, 0.5)));
        let out = render_n(&mut pb, 1000);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(shared.position.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn mixes_both_stems_with_independent_volumes() {
        let n = 48_000;
        let vocals = stereo(n, |_| (0.2, -0.2));
        let inst = stereo(n, |_| (0.3, 0.1));
        let (shared, mut pb) = setup(Some(vocals), inst);
        shared.set_mix(Mix {
            master: 1.0,
            vocal: 1.0,
            instrumental: 1.0,
        });
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000); // sobe o fade e assenta os volumes
        let out = render_n(&mut pb, 100);
        assert!((out[0] - 0.5).abs() < 1e-4 && (out[1] - (-0.1)).abs() < 1e-4);

        shared.set_mix(Mix {
            master: 0.5,
            vocal: 0.0,
            instrumental: 1.0,
        });
        render_n(&mut pb, 4800);
        let out = render_n(&mut pb, 100);
        assert!(
            (out[0] - 0.15).abs() < 1e-3 && (out[1] - 0.05).abs() < 1e-3,
            "{:?}",
            &out[..2]
        );

        shared.set_mix(Mix {
            master: 1.0,
            vocal: 1.0,
            instrumental: 0.0,
        });
        render_n(&mut pb, 4800);
        let out = render_n(&mut pb, 100);
        assert!((out[0] - 0.2).abs() < 1e-3 && (out[1] - (-0.2)).abs() < 1e-3);
    }

    #[test]
    fn without_vocal_stem_only_instrumental_plays() {
        let (shared, mut pb) = setup(None, stereo(48_000, |_| (0.4, 0.4)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000);
        let out = render_n(&mut pb, 10);
        assert!((out[0] - 0.4).abs() < 1e-4);
    }

    #[test]
    fn long_playback_has_no_drift_and_is_bit_exact_at_unity() {
        // 10 minutos em blocos de tamanhos variados: a posição bate com os frames renderizados
        // e cada amostra é exatamente a da fonte (ganho 1, sem limitador).
        let n = RATE as usize * 600;
        let src = stereo(n, |i| {
            (
                ((i % 1000) as f32 / 1000.0 - 0.5) * 0.6,
                ((i % 777) as f32 / 777.0 - 0.5) * 0.6,
            )
        });
        let (shared, mut pb) = setup(None, src.clone());
        shared.playing.store(true, Ordering::Release);
        let out = render_chunked(&mut pb, n - 100);
        let fade = (FADE_SECONDS * RATE as f32) as usize + 2;
        for i in fade..(n - 100) {
            assert!(
                (out[i * 2] - src[i * 2]).abs() < 1e-6
                    && (out[i * 2 + 1] - src[i * 2 + 1]).abs() < 1e-6,
                "diferença no frame {i}"
            );
        }
        assert_eq!(shared.position.load(Ordering::Relaxed) as usize, n - 100);
    }

    #[test]
    fn pause_and_resume_fade_without_clicks_and_keep_position() {
        let (shared, mut pb) = setup(None, stereo(48_000 * 10, |_| (0.5, 0.5)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000);
        shared.playing.store(false, Ordering::Release);
        let out = render_n(&mut pb, 1000);
        let max_step = out
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.01, "salto de {max_step} ao pausar");
        assert_eq!(out[out.len() - 2], 0.0);
        let pos = shared.position.load(Ordering::Relaxed);
        render_n(&mut pb, 5000);
        assert_eq!(
            shared.position.load(Ordering::Relaxed),
            pos,
            "pausado não avança"
        );
        shared.playing.store(true, Ordering::Release);
        let out = render_n(&mut pb, 1000);
        let max_step = out
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.01, "salto de {max_step} ao retomar");
    }

    #[test]
    fn repeated_seeks_do_not_click() {
        // Sinal contínuo e suave (seno de 220 Hz): qualquer corte brusco apareceria como salto.
        let n = RATE as usize * 60;
        let src = stereo(n, |i| {
            let s = 0.5 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / RATE as f32).sin();
            (s, s)
        });
        let (shared, mut pb) = setup(None, src);
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 4800);
        let mut worst = 0.0f32;
        let mut last = 0.0f32;
        for k in 0..40 {
            shared.request_seek((k * 7919 + 1234) % (n - 10_000));
            let out = render_n(&mut pb, 480);
            for f in out.chunks(2) {
                worst = worst.max((f[0] - last).abs());
                last = f[0];
            }
        }
        // Um seno de 220 Hz a 0,5 de amplitude muda no máximo ~0,03 por amostra.
        assert!(worst < 0.05, "estalo no seek: salto de {worst}");
    }

    #[test]
    fn seek_moves_position_even_when_paused() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize * 10, |_| (0.1, 0.1)));
        shared.request_seek(RATE as usize * 3);
        render_n(&mut pb, 480);
        assert_eq!(shared.position.load(Ordering::Relaxed), RATE as u64 * 3);
        shared.request_seek(usize::MAX / 4); // além do fim: limita ao fim
        render_n(&mut pb, 480);
        assert_eq!(shared.position.load(Ordering::Relaxed), RATE as u64 * 10);
    }

    #[test]
    fn reaching_the_end_stops_and_signals_once() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize / 2, |_| (0.2, 0.2)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, RATE as usize);
        assert!(shared.ended.swap(false, Ordering::AcqRel));
        assert!(!shared.playing.load(Ordering::Acquire));
        render_n(&mut pb, 4800);
        assert!(
            !shared.ended.load(Ordering::Acquire),
            "o aviso não se repete"
        );
        assert_eq!(shared.position.load(Ordering::Relaxed), (RATE / 2) as u64);
    }

    #[test]
    fn installing_a_new_track_while_playing_fades_and_starts_at_requested_frame() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize * 5, |_| (0.5, 0.5)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 4800);
        shared.install(
            Arc::new(Loaded::new(
                RATE,
                None,
                stereo(RATE as usize * 5, |_| (-0.5, -0.5)),
            )),
            1000,
        );
        let out = render_n(&mut pb, 4800);
        let max_step = out
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.01, "troca de música com estalo: {max_step}");
        assert!(
            out[out.len() - 2] < -0.4,
            "a nova música deve estar tocando"
        );
        let pos = shared.position.load(Ordering::Relaxed);
        assert!(
            pos > 1000 && pos < 1000 + 4800,
            "posição inicial respeitada: {pos}"
        );
    }

    #[test]
    fn limiter_never_exceeds_one_and_is_transparent_below_threshold() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.8), -0.8);
        for x in [0.9, 1.0, 1.5, 3.0, 100.0] {
            assert!(soft_limit(x) <= 1.0 && soft_limit(x) > 0.8, "{x}");
            assert_eq!(soft_limit(-x), -soft_limit(x));
        }
    }

    #[test]
    fn channel_mapping_mono_and_surround() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize, |_| (0.4, 0.2)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000);
        let mut mono = vec![0.0f32; 10];
        pb.render(&mut mono, 1);
        assert!((mono[5] - 0.3).abs() < 1e-4);
        let mut six = vec![9.0f32; 6 * 10];
        pb.render(&mut six, 6);
        assert!((six[6 * 5] - 0.4).abs() < 1e-4 && (six[6 * 5 + 1] - 0.2).abs() < 1e-4);
        assert!(six[6 * 5 + 2..6 * 5 + 6].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn works_with_integer_sample_formats() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize, |_| (0.5, -0.5)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000);
        let mut out = vec![0i16; 20];
        pb.render(&mut out, 2);
        assert!(
            (out[10] as i32 - 16384).abs() < 8 && (out[11] as i32 + 16384).abs() < 8,
            "{:?}",
            &out[10..12]
        );
    }

    #[test]
    fn peaks_are_reported_and_reset() {
        let (shared, mut pb) = setup(None, stereo(RATE as usize, |_| (0.5, 0.25)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 4000);
        let (l, r) = shared.take_peaks();
        assert!((l - 0.5).abs() < 1e-3 && (r - 0.25).abs() < 1e-3);
        shared.playing.store(false, Ordering::Release);
        render_n(&mut pb, 4000);
        shared.take_peaks();
        render_n(&mut pb, 4000);
        assert_eq!(shared.take_peaks(), (0.0, 0.0));
    }

    #[test]
    fn loaded_pads_shorter_stem_and_keeps_stereo_alignment() {
        let l = Loaded::new(RATE, Some(vec![1.0; 10]), vec![0.5; 7]);
        assert_eq!(l.frames, 5);
        assert_eq!(l.instrumental.len(), 10);
        assert_eq!(l.vocals.as_ref().unwrap().len(), 10);
        assert_eq!(&l.instrumental[7..], &[0.0; 3]);
    }

    // ---- voz ao vivo ------------------------------------------------------------------

    /// Microfone simulado: um tom de 400 Hz que continua sendo escrito a cada bloco.
    struct Mic {
        ring: Arc<crate::voice::Ring>,
        written: usize,
        amp: f32,
    }

    impl Mic {
        fn feed(&mut self, n: usize) {
            let (start, amp) = (self.written, self.amp);
            self.ring.write(n, |i| {
                amp * (2.0 * std::f32::consts::PI * 400.0 * (start + i) as f32 / RATE as f32).sin()
            });
            self.written += n;
        }

        /// Renderiza `frames` em blocos de 480, alimentando o microfone a cada bloco.
        fn render(&mut self, pb: &mut Playback, frames: usize) -> Vec<f32> {
            let mut all = Vec::new();
            for _ in 0..frames / 480 {
                self.feed(480);
                all.extend(render_n(pb, 480));
            }
            all
        }
    }

    fn with_voice(shared: &Arc<Shared>, amp: f32) -> Mic {
        use crate::voice::Ring;
        let ring = Ring::new();
        shared.voice.monitor.store(true, Ordering::Relaxed);
        shared.voice.volume.store(1.0);
        shared.voice.gate_on.store(false, Ordering::Relaxed);
        shared.install_voice(Some(VoiceOut::new(
            ring.clone(),
            shared.voice.clone(),
            RATE,
            RATE,
        )));
        let mut mic = Mic {
            ring,
            written: 0,
            amp,
        };
        mic.feed(2400); // estoque inicial
        mic
    }

    fn rms_of(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn live_voice_is_heard_even_with_no_music_loaded_or_paused() {
        let shared = Shared::new(Mix::default());
        let mut pb = Playback::new(shared.clone(), RATE);
        let mut mic = with_voice(&shared, 0.5);
        let out = mic.render(&mut pb, 9600);
        let tail = &out[2 * 4800..];
        assert!(rms_of(tail) > 0.3, "voz muda: {}", rms_of(tail));
        // dois canais iguais (voz centralizada)
        assert!(tail.chunks(2).all(|f| (f[0] - f[1]).abs() < 1e-6));
        assert_eq!(shared.voice.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn voice_adds_to_the_music_without_changing_it_and_stays_below_one() {
        let (shared, mut pb) = setup(None, stereo(96_000, |_| (0.3, 0.3)));
        shared.playing.store(true, Ordering::Release);
        render_n(&mut pb, 2000); // sobe o fade
        let plain = render_n(&mut pb, 480);
        assert!(plain.iter().all(|&v| (v - 0.3).abs() < 1e-5));
        let mut mic = with_voice(&shared, 0.4);
        let mixed = mic.render(&mut pb, 9600);
        assert!(mixed.iter().all(|v| v.abs() <= 1.0 && v.is_finite()));
        let tail = &mixed[2 * 4800..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!((mean - 0.3).abs() < 0.02, "a música deslocou: {mean}");
        assert!(rms_of(tail) > 0.3 + 0.05);
        // Voz muito alta + música alta: o limitador segura.
        shared.voice.volume.store(1.5);
        shared.set_mix(Mix {
            master: 1.0,
            vocal: 1.0,
            instrumental: 1.5,
        });
        let loud = mic.render(&mut pb, 4800);
        assert!(loud.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn removing_the_voice_goes_quiet_and_music_path_is_unchanged() {
        let (shared, mut pb) = setup(None, stereo(96_000, |_| (0.2, -0.2)));
        shared.playing.store(true, Ordering::Release);
        let mut mic = with_voice(&shared, 0.4);
        mic.render(&mut pb, 4800);
        shared.install_voice(None);
        render_n(&mut pb, 480);
        let out = render_n(&mut pb, 480);
        assert!(out
            .chunks(2)
            .all(|f| (f[0] - 0.2).abs() < 1e-5 && (f[1] + 0.2).abs() < 1e-5));
    }
}
