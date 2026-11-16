//! Autotune natural: detecta a altura da voz, escolhe a nota-alvo da escala e corrige só o
//! quanto você pedir, sem robotizar.
//!
//! Como funciona (tudo no callback, sem alocação):
//! 1. A voz é reduzida para ~12 kHz e um detector YIN estima a frequência a cada ~5 ms.
//! 2. A altura é suavizada (~100 ms): o vibrato e as oscilações rápidas passam intactos, só o
//!    "centro" da nota é corrigido. Trocas reais de nota (>1,5 semitom) são seguidas na hora.
//! 3. A nota-alvo é a mais próxima da escala escolhida (com histerese, para não "pular").
//! 4. A correção (intensidade × erro) é suavizada pela velocidade e aplicada por um deslocador
//!    de altura de dois pontos com atraso variável (≈12 ms de atraso fixo enquanto ligado).
//!
//! Desligado, a voz passa exatamente como entrou e sem atraso.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::playback::AtomicF32;

/// Escala cromática: todas as 12 notas permitidas.
pub const CHROMATIC: u32 = 0xFFF;

/// Ajustes do autotune, compartilhados com a thread de controle.
pub struct TuneShared {
    pub on: AtomicBool,
    /// Intensidade da correção (0 = nenhuma, 1 = vai até a nota).
    pub amount: AtomicF32,
    /// Velocidade (0 = lenta e natural, 1 = instantânea).
    pub speed: AtomicF32,
    /// Notas permitidas: bit 0 = Dó, bit 1 = Dó♯, … bit 11 = Si.
    pub scale_mask: AtomicU32,
    /// Atraso que o autotune soma enquanto ligado (ms).
    pub latency_ms: AtomicF32,
}

impl TuneShared {
    pub fn new() -> TuneShared {
        TuneShared {
            on: AtomicBool::new(false),
            amount: AtomicF32::new(0.5),
            speed: AtomicF32::new(0.35),
            scale_mask: AtomicU32::new(CHROMATIC),
            latency_ms: AtomicF32::new(0.0),
        }
    }
}

impl Default for TuneShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Máscara de notas da escala maior ou menor natural com a tônica dada (0 = Dó).
pub fn scale_mask(tonic: u32, minor: bool) -> u32 {
    let steps: [u32; 7] = if minor {
        [0, 2, 3, 5, 7, 8, 10]
    } else {
        [0, 2, 4, 5, 7, 9, 11]
    };
    steps
        .iter()
        .fold(0, |m, s| m | 1 << ((tonic % 12 + s) % 12))
}

/// Nota MIDI (inteira) permitida mais próxima de `midi`.
pub fn nearest_in_scale(midi: f32, mask: u32) -> f32 {
    let mask = if mask & CHROMATIC == 0 {
        CHROMATIC
    } else {
        mask
    };
    let base = midi.round() as i32;
    let mut best = base as f32;
    let mut best_d = f32::MAX;
    for off in -7..=7 {
        let n = base + off;
        if mask >> n.rem_euclid(12) & 1 == 1 {
            let d = (n as f32 - midi).abs();
            if d < best_d {
                best_d = d;
                best = n as f32;
            }
        }
    }
    best
}

// ------------------------------------------------------------------------------- detector
const DET_RATE: f32 = 12_000.0;
const WIN: usize = 256; // janela de análise (≈21 ms a 12 kHz)
const HIST: usize = 512; // histórico (potência de 2)
const MAX_LAG: usize = 170; // ≈ 70 Hz a 12 kHz
const HOP: usize = 64; // ≈ 5,3 ms
const YIN_THRESHOLD: f32 = 0.18;
const YIN_REJECT: f32 = 0.3;
/// Energia média mínima (≈ −52 dBFS) para tentar detectar altura.
const MIN_POWER: f32 = 6.0e-6;

struct Detector {
    hist: [f32; HIST],
    pos: usize,
    scratch: [f32; WIN + MAX_LAG + 2],
    diff: [f32; MAX_LAG + 2],
    min_lag: usize,
    rate: f32,
}

impl Detector {
    fn new(rate: f32) -> Detector {
        Detector {
            hist: [0.0; HIST],
            pos: 0,
            scratch: [0.0; WIN + MAX_LAG + 2],
            diff: [0.0; MAX_LAG + 2],
            min_lag: ((rate / 1000.0) as usize).max(2),
            rate,
        }
    }

    fn reset(&mut self) {
        self.hist.fill(0.0);
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.hist[self.pos & (HIST - 1)] = x;
        self.pos = self.pos.wrapping_add(1);
    }

    /// Frequência (Hz) se a janela mais recente for tonal; `None` se não for voz afinável.
    fn detect(&mut self) -> Option<f32> {
        let len = WIN + MAX_LAG + 2;
        let start = self.pos.wrapping_sub(len);
        for (i, s) in self.scratch.iter_mut().enumerate() {
            *s = self.hist[start.wrapping_add(i) & (HIST - 1)];
        }
        let w = &self.scratch;
        let power = w[len - WIN..].iter().map(|v| v * v).sum::<f32>() / WIN as f32;
        if power < MIN_POWER {
            return None;
        }
        // Função de diferença (YIN, passo 2).
        let base = len - WIN - MAX_LAG - 1;
        for tau in 1..=MAX_LAG + 1 {
            let mut sum = 0.0;
            for j in 0..WIN {
                let d = w[base + MAX_LAG + 1 + j] - w[base + MAX_LAG + 1 + j - tau];
                sum += d * d;
            }
            self.diff[tau] = sum;
        }
        // Diferença normalizada pela média acumulada (passo 3) e primeiro vale abaixo do limiar.
        let mut running = 0.0;
        let mut cmnd = [1.0f32; MAX_LAG + 2];
        #[allow(clippy::needless_range_loop)]
        for tau in 1..=MAX_LAG + 1 {
            running += self.diff[tau];
            cmnd[tau] = if running > 0.0 {
                self.diff[tau] * tau as f32 / running
            } else {
                1.0
            };
        }
        let mut found = None;
        let mut tau = self.min_lag;
        while tau <= MAX_LAG {
            if cmnd[tau] < YIN_THRESHOLD {
                while tau < MAX_LAG && cmnd[tau + 1] < cmnd[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = match found {
            Some(t) => t,
            None => {
                let (t, v) = (self.min_lag..=MAX_LAG)
                    .map(|t| (t, cmnd[t]))
                    .fold((0, f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
                if v > YIN_REJECT {
                    return None;
                }
                t
            }
        };
        // Interpolação parabólica para precisão de fração de amostra.
        let (a, b, c) = (cmnd[tau - 1], cmnd[tau], cmnd[tau + 1]);
        let denom = a - 2.0 * b + c;
        let shift = if denom.abs() > 1e-9 {
            (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let period = tau as f32 + shift;
        (period > 0.0).then(|| self.rate / period)
    }
}

// ------------------------------------------------------------------------------- deslocador
/// Deslocador de altura de dois pontos: duas leituras do mesmo atraso, defasadas meio ciclo e
/// com janelas complementares, cuja velocidade de varredura define a razão de altura.
struct Shifter {
    buf: Vec<f32>,
    mask: usize,
    wpos: usize,
    phase: f32,
    window: f32,
}

impl Shifter {
    fn new(rate: f32) -> Shifter {
        let window = (0.024 * rate).round();
        let size = ((window as usize) * 2 + 8).next_power_of_two();
        Shifter {
            buf: vec![0.0; size],
            mask: size - 1,
            wpos: 0,
            phase: 0.0,
            window,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.phase = 0.0;
    }

    /// Amostra com `delay` amostras de atraso (≥ 1; 1 = a mais recente), interpolada.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let whole = delay.floor();
        let f = delay - whole;
        let idx = self.wpos.wrapping_sub(whole as usize);
        let newer = self.buf[idx & self.mask];
        let older = self.buf[idx.wrapping_sub(1) & self.mask];
        newer + (older - newer) * f
    }

    #[inline]
    fn process(&mut self, x: f32, ratio: f32) -> f32 {
        self.buf[self.wpos & self.mask] = x;
        self.wpos = self.wpos.wrapping_add(1);
        // delay = fase × janela; subir a altura (ratio > 1) encurta o atraso aos poucos.
        self.phase += (1.0 - ratio) / self.window;
        self.phase -= self.phase.floor();
        let p2 = if self.phase >= 0.5 {
            self.phase - 0.5
        } else {
            self.phase + 0.5
        };
        let w1 = (std::f32::consts::PI * self.phase).sin().powi(2);
        let w2 = 1.0 - w1;
        let d1 = self.phase * self.window + 1.0;
        let d2 = p2 * self.window + 1.0;
        self.read(d1) * w1 + self.read(d2) * w2
    }

    /// Atraso médio com a razão em 1 (fase parada) — para informar na interface.
    fn latency_samples(&self) -> f32 {
        self.window * 0.5 + 1.0
    }
}

// ------------------------------------------------------------------------------- autotune
const MIN_SPEED_TAU: f32 = 0.25;
const MAX_SPEED_TAU: f32 = 0.005;
const MAX_CORRECTION: f32 = 2.5; // semitons
/// Suavização do "centro" da nota: lenta (deixa o vibrato passar) em velocidades baixas e
/// rápida (acompanha e achata a deriva da voz) em velocidades altas.
const PITCH_TAU_SLOW: f32 = 0.12;
const PITCH_TAU_FAST: f32 = 0.02;
const NOTE_JUMP: f32 = 1.5;
const HOLD_TARGET: f32 = 0.65;
const SETTLE_FRAMES: u32 = 5;

pub struct Tune {
    rate: f32,
    dec: usize,
    dec_acc: f32,
    dec_n: usize,
    hop_n: usize,
    det: Detector,
    shifter: Shifter,
    // estado musical
    voiced: bool,
    /// Quadros tonais seguidos (o começo da nota ainda oscila: só corrige depois de estabilizar).
    voiced_frames: u32,
    m_slow: f32,
    target: Option<f32>,
    /// Correção desejada (semitons), atualizada a cada análise.
    want: f32,
    /// Correção aplicada (semitons), suavizada.
    corr: f32,
    mix: f32,
    mix_k: f32,
    // ajustes do bloco
    on: bool,
    amount: f32,
    mask: u32,
    k_corr: f32,
    k_unvoiced: f32,
    alpha_slow: f32,
    hop_s: f32,
    active: bool,
}

impl Tune {
    pub fn new(rate: u32) -> Tune {
        let rate = rate as f32;
        let dec = ((rate / DET_RATE).round() as usize).max(1);
        let det_rate = rate / dec as f32;
        let hop_s = HOP as f32 / det_rate;
        Tune {
            rate,
            dec,
            dec_acc: 0.0,
            dec_n: 0,
            hop_n: 0,
            det: Detector::new(det_rate),
            shifter: Shifter::new(rate),
            voiced: false,
            voiced_frames: 0,
            m_slow: 69.0,
            target: None,
            want: 0.0,
            corr: 0.0,
            mix: 0.0,
            mix_k: 1.0 - (-1.0 / (0.03 * rate)).exp(),
            on: false,
            amount: 0.0,
            mask: CHROMATIC,
            k_corr: 0.0,
            k_unvoiced: 1.0 - (-1.0 / (0.04 * rate)).exp(),
            alpha_slow: 1.0 - (-hop_s / PITCH_TAU_SLOW).exp(),
            hop_s,
            active: false,
        }
    }

    /// Uma vez por bloco: lê os ajustes e informa o atraso adicionado.
    pub fn begin_block(&mut self, s: &TuneShared) {
        self.on = s.on.load(Ordering::Relaxed);
        self.amount = s.amount.load().clamp(0.0, 1.0);
        self.mask = s.scale_mask.load(Ordering::Relaxed) & CHROMATIC;
        let speed = s.speed.load().clamp(0.0, 1.0);
        let tau = MIN_SPEED_TAU * (MAX_SPEED_TAU / MIN_SPEED_TAU).powf(speed);
        self.k_corr = 1.0 - (-1.0 / (tau * self.rate)).exp();
        // Acima de 50% de velocidade o centro da nota passa a ser seguido cada vez mais de perto.
        let fast = ((speed - 0.5) * 2.0).clamp(0.0, 1.0);
        let pitch_tau = PITCH_TAU_SLOW * (PITCH_TAU_FAST / PITCH_TAU_SLOW).powf(fast);
        self.alpha_slow = 1.0 - (-self.hop_s / pitch_tau).exp();
        let latency = if self.on {
            self.shifter.latency_samples() / self.rate * 1000.0
        } else {
            0.0
        };
        s.latency_ms.store(latency);
    }

    fn analyze(&mut self) {
        let Some(f0) = self.det.detect() else {
            self.voiced = false;
            self.voiced_frames = 0;
            self.target = None;
            self.want = 0.0;
            return;
        };
        let m = 69.0 + 12.0 * (f0 / 440.0).log2();
        if !self.voiced || (m - self.m_slow).abs() > NOTE_JUMP {
            self.m_slow = m; // começo de nota ou troca real de nota: segue na hora
            self.voiced_frames = 0;
            self.target = None;
        } else if self.voiced_frames < SETTLE_FRAMES {
            self.m_slow += (m - self.m_slow) * 0.5; // início da nota: acompanha rápido
        } else {
            self.m_slow += (m - self.m_slow) * self.alpha_slow;
        }
        self.voiced = true;
        self.voiced_frames = self.voiced_frames.saturating_add(1);
        if self.voiced_frames < SETTLE_FRAMES {
            self.want = 0.0; // ainda estabilizando
            return;
        }
        let keep = self
            .target
            .filter(|t| (self.m_slow - t).abs() <= HOLD_TARGET && self.allowed(*t));
        let target = keep.unwrap_or_else(|| nearest_in_scale(self.m_slow, self.mask));
        self.target = Some(target);
        self.want = (self.amount * (target - self.m_slow)).clamp(-MAX_CORRECTION, MAX_CORRECTION);
    }

    fn allowed(&self, note: f32) -> bool {
        self.mask >> (note as i32).rem_euclid(12) & 1 == 1
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let target_mix = if self.on { 1.0 } else { 0.0 };
        if !self.on && self.mix < 1e-4 {
            if self.active {
                // Acabou de desligar: zera o estado para religar limpo.
                self.active = false;
                self.mix = 0.0;
                self.corr = 0.0;
                self.want = 0.0;
                self.voiced = false;
                self.target = None;
                self.shifter.clear();
                self.det.reset();
            }
            return x;
        }
        self.active = true;
        self.mix += (target_mix - self.mix) * self.mix_k;

        // Detector em taxa reduzida (média de `dec` amostras).
        self.dec_acc += x;
        self.dec_n += 1;
        if self.dec_n == self.dec {
            self.det.push(self.dec_acc / self.dec as f32);
            self.dec_acc = 0.0;
            self.dec_n = 0;
            self.hop_n += 1;
            if self.hop_n == HOP {
                self.hop_n = 0;
                self.analyze();
            }
        }

        let k = if self.voiced {
            self.k_corr
        } else {
            self.k_unvoiced.max(self.k_corr)
        };
        self.corr += (self.want - self.corr) * k;
        let ratio = (self.corr * (1.0 / 12.0)).exp2();
        let wet = self.shifter.process(x, ratio);
        x + (wet - x) * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    /// Voz sintética: fundamental + harmônicos (mais parecida com voz que uma senoide pura).
    fn voice(freq_at: impl Fn(usize) -> f64, n: usize) -> Vec<f32> {
        let mut phase = 0.0f64;
        (0..n)
            .map(|i| {
                phase += 2.0 * std::f64::consts::PI * freq_at(i) / R as f64;
                (0.25 * phase.sin() + 0.12 * (2.0 * phase).sin() + 0.06 * (3.0 * phase).sin())
                    as f32
            })
            .collect()
    }

    fn cents(f: f64, reference: f64) -> f64 {
        1200.0 * (f / reference).log2()
    }

    /// Alturas detectadas (Hz) ao longo do sinal, uma por ~5 ms; `None` onde não há altura.
    fn pitches(x: &[f32]) -> Vec<Option<f64>> {
        let mut d = Detector::new(DET_RATE);
        let mut out = vec![];
        for (n, chunk) in x.chunks(4).enumerate() {
            d.push(chunk.iter().sum::<f32>() / chunk.len() as f32);
            if n % HOP == HOP - 1 {
                out.push(d.detect().map(|f| f as f64));
            }
        }
        out
    }

    /// Altura mediana (Hz); exige que quase todos os quadros sejam tonais.
    fn freq(x: &[f32]) -> f64 {
        let p = pitches(x);
        let mut v: Vec<f64> = p.iter().flatten().copied().collect();
        assert!(
            v.len() * 10 >= p.len() * 9,
            "saída perdeu a altura: {}/{}",
            v.len(),
            p.len()
        );
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    fn run(tune: &mut Tune, shared: &TuneShared, x: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(x.len());
        for chunk in x.chunks(480) {
            tune.begin_block(shared);
            out.extend(chunk.iter().map(|&v| tune.process(v)));
        }
        out
    }

    fn on(amount: f32, speed: f32, mask: u32) -> (Tune, TuneShared) {
        let s = TuneShared::new();
        s.on.store(true, Ordering::Relaxed);
        s.amount.store(amount);
        s.speed.store(speed);
        s.scale_mask.store(mask, Ordering::Relaxed);
        (Tune::new(R), s)
    }

    #[test]
    fn detector_finds_voice_range_pitches() {
        for f in [90.0, 110.0, 196.0, 220.0, 330.0, 440.0, 660.0, 880.0] {
            let mut d = Detector::new(DET_RATE);
            let x = voice(|_| f, R as usize);
            for chunk in x.chunks(4) {
                d.push(chunk.iter().sum::<f32>() / 4.0);
            }
            let got = d.detect().unwrap_or_else(|| panic!("{f} Hz não detectado"));
            assert!(cents(got as f64, f).abs() < 8.0, "{f} Hz → {got}");
        }
    }

    #[test]
    fn detector_rejects_silence_and_noise() {
        let mut d = Detector::new(DET_RATE);
        for _ in 0..1000 {
            d.push(0.0);
        }
        assert!(d.detect().is_none());
        let mut seed = 12345u32;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            d.push(((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.4);
        }
        assert!(d.detect().is_none(), "ruído branco não tem altura");
    }

    #[test]
    fn scale_masks_and_nearest_note() {
        let c_major = scale_mask(0, false);
        assert_eq!(c_major.count_ones(), 7);
        assert_eq!(nearest_in_scale(61.3, c_major), 62.0);
        assert_eq!(nearest_in_scale(60.4, c_major), 60.0);
        assert_eq!(nearest_in_scale(61.3, CHROMATIC), 61.0);
        // Lá menor tem as mesmas notas que Dó maior.
        assert_eq!(scale_mask(9, true), c_major);
        // Máscara vazia cai em cromática (nunca trava).
        assert_eq!(nearest_in_scale(61.4, 0), 61.0);
    }

    #[test]
    fn off_is_bit_exact_and_has_no_latency() {
        let x = voice(|_| 223.0, 20_000);
        let mut t = Tune::new(R);
        let s = TuneShared::new();
        let y = run(&mut t, &s, &x);
        assert_eq!(x, y);
        assert_eq!(s.latency_ms.load(), 0.0);
    }

    #[test]
    fn pulls_a_sharp_note_to_the_nearest_semitone() {
        // 226 Hz é ~46 cents acima de Lá3 (220 Hz).
        let x = voice(|_| 226.0, R as usize * 2);
        let (mut t, s) = on(1.0, 0.9, CHROMATIC);
        let y = run(&mut t, &s, &x);
        let f = freq(&y[R as usize + 8000..]);
        assert!(cents(f, 220.0).abs() < 8.0, "saiu {f} Hz");
    }

    #[test]
    fn amount_scales_the_correction() {
        let x = voice(|_| 226.0, R as usize * 2);
        let (mut t, s) = on(0.5, 0.9, CHROMATIC);
        let y = run(&mut t, &s, &x);
        let f = freq(&y[R as usize + 8000..]);
        // metade dos 46 cents: ~23 cents acima de 220 Hz
        assert!((cents(f, 220.0) - 23.0).abs() < 8.0, "saiu {f} Hz");
        let (mut t0, s0) = on(0.0, 0.9, CHROMATIC);
        let y0 = run(&mut t0, &s0, &x);
        assert!(cents(freq(&y0[R as usize + 8000..]), 226.0).abs() < 6.0);
    }

    #[test]
    fn respects_the_scale() {
        // Dó♯4 (277,2 Hz) ligeiramente alto, em Dó maior: vai para Ré4 (293,7 Hz), não para Dó♯.
        let x = voice(|_| 283.0, R as usize * 2);
        let (mut t, s) = on(1.0, 1.0, scale_mask(0, false));
        let y = run(&mut t, &s, &x);
        let f = freq(&y[R as usize + 8000..]);
        assert!(cents(f, 293.66).abs() < 12.0, "saiu {f} Hz");
    }

    #[test]
    fn slow_speed_keeps_vibrato() {
        // Vibrato de ±35 cents a 5,5 Hz em volta de Lá3 (já afinado).
        let vib = |i: usize| {
            220.0
                * 2f64.powf(
                    35.0 / 1200.0 * (2.0 * std::f64::consts::PI * 5.5 * i as f64 / R as f64).sin(),
                )
        };
        let x = voice(vib, R as usize * 3);
        let (mut t, s) = on(1.0, 0.3, CHROMATIC);
        let y = run(&mut t, &s, &x);
        let swing = |v: &[f32]| {
            let mut c: Vec<f64> = pitches(v)
                .into_iter()
                .flatten()
                .map(|f| cents(f, 220.0))
                .collect();
            c.sort_by(|a, b| a.partial_cmp(b).unwrap());
            c[c.len() * 9 / 10] - c[c.len() / 10]
        };
        let before = swing(&x[R as usize..]);
        let after = swing(&y[R as usize..]);
        assert!(
            after > before * 0.6,
            "vibrato {before:.0} → {after:.0} cents"
        );
    }

    #[test]
    fn output_level_and_silence_are_safe() {
        let x = voice(|_| 226.0, R as usize);
        let (mut t, s) = on(1.0, 1.0, CHROMATIC);
        let y = run(&mut t, &s, &x);
        let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
        let ratio = rms(&y[R as usize / 2..]) / rms(&x[R as usize / 2..]);
        assert!((0.8..1.2).contains(&ratio), "ganho {ratio}");
        let silence = run(&mut t, &s, &vec![0.0; 20_000]);
        // a linha de atraso ainda solta ~24 ms do que entrou; depois disso é silêncio
        assert!(silence.iter().all(|v| v.is_finite()));
        assert!(silence[2400..].iter().all(|v| v.abs() < 1e-4));
    }

    #[test]
    fn reports_latency_only_while_on() {
        let (mut t, s) = on(0.5, 0.5, CHROMATIC);
        t.begin_block(&s);
        let ms = s.latency_ms.load();
        assert!((10.0..16.0).contains(&ms), "{ms}");
        s.on.store(false, Ordering::Relaxed);
        t.begin_block(&s);
        assert_eq!(s.latency_ms.load(), 0.0);
    }

    #[test]
    fn turning_off_fades_out_and_returns_to_exact_bypass() {
        let x = voice(|_| 226.0, R as usize);
        let (mut t, s) = on(1.0, 1.0, CHROMATIC);
        run(&mut t, &s, &x);
        s.on.store(false, Ordering::Relaxed);
        let y = run(&mut t, &s, &x);
        let tail = &y[R as usize / 2..];
        assert_eq!(tail, &x[R as usize / 2..]);
    }

    /// Desvio-padrão (cents) da altura detectada ao longo do sinal.
    fn wobble_cents(x: &[f32]) -> f64 {
        let c: Vec<f64> = pitches(x)
            .into_iter()
            .flatten()
            .map(|f| cents(f, 220.0))
            .collect();
        let mean = c.iter().sum::<f64>() / c.len() as f64;
        (c.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / c.len() as f64).sqrt()
    }

    /// Voz com deriva lenta (1,7 Hz, ±30 cents) e um leve tremido (7 Hz) em volta de uma nota.
    fn wobbly() -> Vec<f32> {
        voice(
            |i| {
                let t = i as f64 / R as f64;
                let c = 30.0 * (2.0 * std::f64::consts::PI * 1.7 * t).sin()
                    + 6.0 * (2.0 * std::f64::consts::PI * 7.0 * t).sin();
                222.0 * 2f64.powf(c / 1200.0)
            },
            R as usize * 3,
        )
    }

    #[test]
    fn fast_speed_flattens_the_drift_but_slow_speed_keeps_it() {
        let x = wobbly();
        let before = wobble_cents(&x[R as usize..]);
        let (mut hard, s) = on(1.0, 1.0, CHROMATIC);
        let flat = wobble_cents(&run(&mut hard, &s, &x)[R as usize..]);
        println!("deriva: original {before:.0} cents, efeito {flat:.0} cents");
        assert!(flat < before * 0.4, "efeito: {before:.0} → {flat:.0} cents");
        let (mut soft, s) = on(1.0, 0.3, CHROMATIC);
        let kept = wobble_cents(&run(&mut soft, &s, &x)[R as usize..]);
        println!("deriva: natural {kept:.0} cents");
        assert!(
            kept > before * 0.6,
            "natural: {before:.0} → {kept:.0} cents"
        );
    }

    #[test]
    fn detects_a_noisy_breathy_voice() {
        // voz + ruído a ~20 dB abaixo + tremolo de amplitude
        let mut seed = 7u32;
        let x: Vec<f32> = voice(|_| 196.0, R as usize * 2)
            .iter()
            .enumerate()
            .map(|(i, v)| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.06;
                let trem =
                    0.8 + 0.2 * (i as f32 * 2.0 * std::f32::consts::PI * 5.0 / R as f32).sin();
                v * trem + noise
            })
            .collect();
        let p = pitches(&x[R as usize / 2..]);
        let hit = p.iter().flatten().count() as f64 / p.len() as f64;
        assert!(hit > 0.85, "só {:.0}% dos quadros detectados", hit * 100.0);
    }
}
