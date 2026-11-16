//! Logica pura do spike 3b (sem WASAPI): chirp, correlacao cruzada, conversao de formatos de
//! amostra e o estado compartilhado entre as threads de saida e de entrada.
//! Tudo aqui e testado com `cargo test`, em qualquer sistema.

use std::sync::atomic::{
    AtomicBool, AtomicU32, AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};

pub const CHIRP_SECONDS: f32 = 0.05;
pub const WINDOW_SECONDS: f64 = 1.0;
pub const SEARCH_SECONDS: f64 = 0.7;

// ---------------------------------------------------------------- formatos de amostra

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFmt {
    I16,
    /// 24 bits empacotados em 3 bytes.
    I24,
    /// Inteiro de 32 bits (em WASAPI, 24 bits validos ficam alinhados a esquerda).
    I32,
    F32,
}

impl SampleFmt {
    pub fn bytes(self) -> usize {
        match self {
            SampleFmt::I16 => 2,
            SampleFmt::I24 => 3,
            SampleFmt::I32 | SampleFmt::F32 => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SampleFmt::I16 => "16 bits inteiro",
            SampleFmt::I24 => "24 bits inteiro (3 bytes)",
            SampleFmt::I32 => "32 bits inteiro",
            SampleFmt::F32 => "32 bits float",
        }
    }
}

/// Converte quadros intercalados para uma sequencia mono (media dos canais), em `out`.
pub fn decode_mono(bytes: &[u8], fmt: SampleFmt, channels: usize, out: &mut Vec<f32>) {
    out.clear();
    let bps = fmt.bytes();
    let frame_bytes = bps * channels;
    if frame_bytes == 0 {
        return;
    }
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            let s = &frame[ch * bps..(ch + 1) * bps];
            sum += match fmt {
                SampleFmt::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32_768.0,
                SampleFmt::I24 => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                SampleFmt::I32 => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                SampleFmt::F32 => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            };
        }
        out.push(sum / channels as f32);
    }
}

/// Converte uma sequencia mono para quadros intercalados (o mesmo valor em todos os canais), em `out`.
pub fn encode_frames(mono: &[f32], fmt: SampleFmt, channels: usize, out: &mut Vec<u8>) {
    out.clear();
    for &x in mono {
        let x = x.clamp(-1.0, 1.0);
        for _ in 0..channels {
            match fmt {
                SampleFmt::I16 => out.extend_from_slice(&((x * 32_767.0).round() as i16).to_le_bytes()),
                SampleFmt::I24 => {
                    let v = (x * 8_388_607.0).round() as i32;
                    out.extend_from_slice(&v.to_le_bytes()[..3]);
                }
                SampleFmt::I32 => {
                    out.extend_from_slice(&((x as f64 * 2_147_483_647.0).round() as i32).to_le_bytes())
                }
                SampleFmt::F32 => out.extend_from_slice(&x.to_le_bytes()),
            }
        }
    }
}

// ---------------------------------------------------------------- chirp e correlacao

/// Varredura linear de 1 a 8 kHz com janela de Hann, amplitude 1.
pub fn make_chirp(rate: u32) -> Vec<f32> {
    let n = (rate as f32 * CHIRP_SECONDS) as usize;
    let (f0, f1) = (1000.0f32, 8000.0f32);
    let t_total = n as f32 / rate as f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let phase = 2.0 * std::f32::consts::PI * (f0 * t + (f1 - f0) * t * t / (2.0 * t_total));
            let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos();
            hann * phase.sin()
        })
        .collect()
}

pub struct Match {
    pub lag: usize,
    pub ncc: f32,
    pub amp: f32,
}

/// Correlacao cruzada normalizada do modelo `tpl` com `sig`, testando atrasos de 0 a `max_lag`.
pub fn best_match(sig: &[f32], tpl: &[f32], max_lag: usize) -> Option<Match> {
    let m = tpl.len();
    if m == 0 || sig.len() < m {
        return None;
    }
    let max_lag = max_lag.min(sig.len() - m);
    let tpl_energy: f64 = tpl.iter().map(|&x| (x as f64) * (x as f64)).sum();
    let mut seg_energy: f64 = sig[..m].iter().map(|&x| (x as f64) * (x as f64)).sum();
    let mut best_lag = 0usize;
    let mut best_ncc = f64::MIN;
    let mut best_dot = 0.0f64;
    for lag in 0..=max_lag {
        if lag > 0 {
            let add = sig[lag + m - 1] as f64;
            let sub = sig[lag - 1] as f64;
            seg_energy += add * add - sub * sub;
        }
        let dot: f64 = sig[lag..lag + m].iter().zip(tpl).map(|(&a, &b)| a as f64 * b as f64).sum();
        let ncc = dot / (tpl_energy * seg_energy.max(1e-18)).sqrt();
        if ncc > best_ncc {
            best_ncc = ncc;
            best_lag = lag;
            best_dot = dot;
        }
    }
    Some(Match { lag: best_lag, ncc: best_ncc as f32, amp: (best_dot / tpl_energy) as f32 })
}

/// (minimo, mediana, media, maximo)
pub fn summarize(values: &mut [f64]) -> Option<(f64, f64, f64, f64)> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    let median = if n % 2 == 1 { values[n / 2] } else { 0.5 * (values[n / 2 - 1] + values[n / 2]) };
    let mean = values.iter().sum::<f64>() / n as f64;
    Some((values[0], median, mean, values[n - 1]))
}

// ---------------------------------------------------------------- estado compartilhado

pub struct Shared {
    pub fire: AtomicBool,
    pub capturing: AtomicBool,
    pub done: AtomicBool,
    pub quit: AtomicBool,
    pub emit_ns: AtomicU64,
    pub cap_t0_ns: AtomicU64,
    pub cap_len: AtomicU64,
    /// Quantidade de amostras da janela de gravacao (definida depois de abrir a entrada).
    pub cap_target: AtomicU64,
    pub cap: Vec<AtomicU32>,
    pub in_peak_bits: AtomicU32,
}

impl Shared {
    pub fn new(max_cap_samples: usize) -> Self {
        Shared {
            fire: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            done: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            emit_ns: AtomicU64::new(0),
            cap_t0_ns: AtomicU64::new(0),
            cap_len: AtomicU64::new(0),
            cap_target: AtomicU64::new(max_cap_samples as u64),
            cap: (0..max_cap_samples).map(|_| AtomicU32::new(0)).collect(),
            in_peak_bits: AtomicU32::new(0),
        }
    }
}

pub struct OutState {
    pub burst_pos: Option<usize>,
}

/// Preenche as amostras mono da saida: silencio, ou o chirp quando ha um disparo pendente.
/// Roda na thread de saida a cada evento; `cb_ns` e o instante em que a thread acordou.
pub fn fill_output(mono: &mut [f32], st: &mut OutState, burst: &[f32], sh: &Shared, cb_ns: u64) {
    if st.burst_pos.is_none() && sh.fire.swap(false, Relaxed) {
        st.burst_pos = Some(0);
        sh.emit_ns.store(cb_ns, Relaxed);
        sh.capturing.store(true, Release);
    }
    for x in mono.iter_mut() {
        *x = match st.burst_pos {
            Some(p) => {
                st.burst_pos = if p + 1 < burst.len() { Some(p + 1) } else { None };
                burst[p]
            }
            None => 0.0,
        };
    }
}

/// Copia amostras mono da entrada para a janela de gravacao, quando ha uma gravacao em curso.
/// `now_ns` e o instante em que a thread acordou; a ultima amostra do bloco e tomada como "agora".
pub fn capture_mono(mono: &[f32], rate: u32, now_ns: u64, sh: &Shared) {
    let peak = mono.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    sh.in_peak_bits.fetch_max(peak.to_bits(), Relaxed);
    if !sh.capturing.load(Acquire) {
        return;
    }
    let target = (sh.cap_target.load(Relaxed) as usize).min(sh.cap.len());
    let len = sh.cap_len.load(Relaxed) as usize;
    if len == 0 {
        let dur_ns = (mono.len() as f64 / rate as f64 * 1e9) as u64;
        sh.cap_t0_ns.store(now_ns.saturating_sub(dur_ns), Relaxed);
    }
    let n = mono.len().min(target.saturating_sub(len));
    for (i, &s) in mono.iter().take(n).enumerate() {
        sh.cap[len + i].store(s.to_bits(), Relaxed);
    }
    let new_len = len + n;
    sh.cap_len.store(new_len as u64, Relaxed);
    if new_len >= target {
        sh.capturing.store(false, Relaxed);
        sh.done.store(true, Release);
    }
}

// ---------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                ((x % 20_001) as f32 / 10_000.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn sample_formats_round_trip_stereo_and_mono() {
        let ramp: Vec<f32> = (0..200).map(|i| (i as f32 / 100.0 - 1.0) * 0.9).collect();
        for (fmt, tol) in [(SampleFmt::I16, 1e-4), (SampleFmt::I24, 1e-6), (SampleFmt::I32, 1e-6), (SampleFmt::F32, 0.0)] {
            for channels in [1usize, 2] {
                let mut bytes = Vec::new();
                encode_frames(&ramp, fmt, channels, &mut bytes);
                assert_eq!(bytes.len(), ramp.len() * channels * fmt.bytes(), "{fmt:?}");
                let mut back = Vec::new();
                decode_mono(&bytes, fmt, channels, &mut back);
                assert_eq!(back.len(), ramp.len());
                for (a, b) in ramp.iter().zip(&back) {
                    assert!((a - b).abs() <= tol + 1e-7, "{fmt:?} ch{channels}: {a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn decode_averages_channels_and_handles_negative_24_bit() {
        // quadro estereo I16: 0.5 e -0.25 -> media 0.125
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&((0.5f32 * 32_768.0) as i16).to_le_bytes());
        bytes.extend_from_slice(&((-0.25f32 * 32_768.0) as i16).to_le_bytes());
        let mut out = Vec::new();
        decode_mono(&bytes, SampleFmt::I16, 2, &mut out);
        assert!((out[0] - 0.125).abs() < 1e-4);
        // 24 bits negativo: -0.5 deve voltar como -0.5 (extensao de sinal)
        let mut b24 = Vec::new();
        encode_frames(&[-0.5], SampleFmt::I24, 1, &mut b24);
        decode_mono(&b24, SampleFmt::I24, 1, &mut out);
        assert!((out[0] + 0.5).abs() < 1e-6, "{}", out[0]);
        // bytes incompletos no fim sao ignorados
        decode_mono(&[1, 2, 3], SampleFmt::I16, 2, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn encode_clamps_out_of_range_values() {
        let mut bytes = Vec::new();
        encode_frames(&[2.0, -2.0], SampleFmt::I16, 1, &mut bytes);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 32_767);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), -32_767);
    }

    #[test]
    fn chirp_has_expected_length_and_bounds() {
        let c = make_chirp(48_000);
        assert_eq!(c.len(), 2_400);
        assert!(c[0].abs() < 1e-3 && c[c.len() - 1].abs() < 1e-3);
        let peak = c.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.9 && peak <= 1.0, "pico = {peak}");
    }

    #[test]
    fn recovers_known_delay_in_noise() {
        let rate = 8_000;
        let tpl = make_chirp(rate);
        let mut sig = noise(4_000, 0.01, 0x9E37_79B9_7F4A_7C15);
        let delay = 1_234;
        for (i, &t) in tpl.iter().enumerate() {
            sig[delay + i] += 0.05 * t;
        }
        let m = best_match(&sig, &tpl, 3_000).unwrap();
        assert_eq!(m.lag, delay);
        assert!(m.ncc > 0.8, "ncc = {}", m.ncc);
        assert!((m.amp - 0.05).abs() < 0.01, "amp = {}", m.amp);
    }

    #[test]
    fn noise_only_is_not_accepted_as_a_signal() {
        let tpl = make_chirp(8_000);
        for seed in [1u64, 7, 12345, 0xDEAD_BEEF] {
            let sig = noise(4_000, 0.05, seed | 1);
            let m = best_match(&sig, &tpl, 3_000).unwrap();
            assert!(m.ncc < 0.3, "ncc = {} (seed {seed})", m.ncc);
        }
    }

    #[test]
    fn short_recording_gives_none() {
        assert!(best_match(&[0.0; 10], &make_chirp(8_000), 100).is_none());
    }

    #[test]
    fn summarize_gives_min_median_mean_max() {
        let mut v = vec![5.0, 1.0, 3.0];
        assert_eq!(summarize(&mut v), Some((1.0, 3.0, 3.0, 5.0)));
        let mut v = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(summarize(&mut v), Some((1.0, 2.5, 2.5, 4.0)));
        assert_eq!(summarize(&mut []), None);
    }

    #[test]
    fn output_emits_chirp_once_after_fire_and_starts_capture() {
        let burst = make_chirp(8_000);
        let sh = Shared::new(100);
        let mut st = OutState { burst_pos: None };
        let mut buf = vec![1.0f32; 64];
        fill_output(&mut buf, &mut st, &burst, &sh, 42);
        assert!(buf.iter().all(|&x| x == 0.0), "sem fire, a saida e silencio");
        assert!(!sh.capturing.load(Relaxed));

        sh.fire.store(true, Relaxed);
        let mut emitted = 0usize;
        for _ in 0..8 {
            fill_output(&mut buf, &mut st, &burst, &sh, 1_000);
            emitted += buf.iter().filter(|&&x| x != 0.0).count();
        }
        assert!(sh.capturing.load(Relaxed));
        assert!(!sh.fire.load(Relaxed));
        assert_eq!(sh.emit_ns.load(Relaxed), 1_000);
        assert!(emitted >= burst.len() - 4 && emitted <= burst.len(), "emitidos = {emitted}");
    }

    #[test]
    fn capture_records_window_then_signals_done() {
        let epoch = Instant::now();
        let sh = Shared::new(500);
        sh.cap_target.store(150, Relaxed);
        let now = || epoch.elapsed().as_nanos() as u64;
        // sem gravacao em curso, nada e copiado
        capture_mono(&[0.3; 100], 48_000, now(), &sh);
        assert_eq!(sh.cap_len.load(Relaxed), 0);
        assert!(f32::from_bits(sh.in_peak_bits.load(Relaxed)) > 0.29);

        std::thread::sleep(Duration::from_millis(20));
        sh.capturing.store(true, Release);
        capture_mono(&[0.3; 100], 48_000, now(), &sh);
        assert_eq!(sh.cap_len.load(Relaxed), 100);
        assert!(!sh.done.load(Acquire));
        capture_mono(&[0.3; 100], 48_000, now(), &sh);
        assert_eq!(sh.cap_len.load(Relaxed), 150);
        assert!(sh.done.load(Acquire));
        assert!(!sh.capturing.load(Relaxed));
        assert!((f32::from_bits(sh.cap[149].load(Relaxed)) - 0.3).abs() < 1e-6);
        assert_eq!(sh.cap[150].load(Relaxed), 0, "nada alem da janela");
        let t0 = sh.cap_t0_ns.load(Relaxed) as f64 / 1e6;
        assert!(t0 >= 15.0 && t0 < 500.0, "t0 = {t0}");
    }
}
