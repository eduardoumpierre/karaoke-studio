//! Autotune em streaming: detector de altura YIN + quantizacao para a escala +
//! deslocador de altura no dominio do tempo (linha de atraso com saltos de um periodo).
//!
//! Tudo aqui e puro (sem I/O) e processa blocos de qualquer tamanho, como faria
//! o callback de audio do motor final. Nao aloca dentro de `process`.

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Chromatic,
    Major,
    Minor,
    PentMajor,
    PentMinor,
}

impl Scale {
    pub fn parse(s: &str) -> Option<Scale> {
        match s.to_lowercase().as_str() {
            "chromatic" | "cromatica" => Some(Scale::Chromatic),
            "major" | "maior" => Some(Scale::Major),
            "minor" | "menor" => Some(Scale::Minor),
            "pent-major" | "pent-maior" => Some(Scale::PentMajor),
            "pent-minor" | "pent-menor" => Some(Scale::PentMinor),
            _ => None,
        }
    }

    /// Notas permitidas, relativas a tonica (indice 0 = tonica).
    pub fn mask(self) -> [bool; 12] {
        let steps: &[usize] = match self {
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::PentMajor => &[0, 2, 4, 7, 9],
            Scale::PentMinor => &[0, 3, 5, 7, 10],
        };
        let mut m = [false; 12];
        for &s in steps {
            m[s] = true;
        }
        m
    }
}

/// "C", "C#", "Db", "a"... -> 0..11
pub fn parse_key(s: &str) -> Option<u8> {
    let t = s.trim();
    let mut ch = t.chars();
    let base: i32 = match ch.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let acc = match ch.as_str() {
        "" => 0,
        "#" => 1,
        "b" => -1,
        _ => return None,
    };
    Some((base + acc).rem_euclid(12) as u8)
}

pub fn hz_to_midi(f: f32) -> f32 {
    69.0 + 12.0 * (f / 440.0).log2()
}

/// Nota MIDI inteira permitida mais proxima de `midi`.
pub fn nearest_allowed(midi: f32, key: u8, mask: &[bool; 12]) -> i32 {
    let lo = midi.floor() as i32 - 2;
    let hi = midi.ceil() as i32 + 2;
    let mut best = lo;
    let mut best_d = f32::MAX;
    for n in lo..=hi {
        let rel = (n - key as i32).rem_euclid(12) as usize;
        if mask[rel] {
            let d = (n as f32 - midi).abs();
            if d < best_d {
                best_d = d;
                best = n;
            }
        }
    }
    best
}

// ---------------------------------------------------------------- YIN

pub struct Yin {
    fs: f32,
    w: usize,
    tau_min: usize,
    tau_max: usize,
    thr: f32,
    pub min_rms: f32,
    cmnd: Vec<f32>,
    /// Contadores de diagnostico: quadros abaixo do volume minimo, sem altura clara, com voz.
    pub n_gate: u64,
    pub n_unclear: u64,
    pub n_voiced: u64,
}

impl Yin {
    pub fn new(fs: f32, fmin: f32, fmax: f32, thr: f32) -> Self {
        let w = (fs * 0.016) as usize; // janela de integracao de 16 ms
        let tau_max = (fs / fmin) as usize;
        let tau_min = ((fs / fmax) as usize).max(2);
        Yin { fs, w, tau_min, tau_max, thr, min_rms: 0.0005, cmnd: vec![1.0; tau_max + 2], n_gate: 0, n_unclear: 0, n_voiced: 0 }
    }

    /// Amostras necessarias no quadro: as `w` mais recentes e `tau_max` de historico antes delas.
    pub fn frame_len(&self) -> usize {
        self.w + self.tau_max + 1
    }

    pub fn window_ms(&self) -> f32 {
        self.w as f32 / self.fs * 1000.0
    }

    /// `x` tem exatamente `frame_len()` amostras; a mais recente e a ultima.
    /// Devolve (f0 em Hz, clareza 0..1) ou None se sem voz.
    pub fn detect(&mut self, x: &[f32]) -> Option<(f32, f32)> {
        let w = self.w;
        let tmax = self.tau_max;
        assert!(x.len() >= tmax + w + 1);
        let x = &x[x.len() - (tmax + w + 1)..];
        let recent = &x[tmax + 1..tmax + 1 + w];
        let rms = (recent.iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt();
        if rms < self.min_rms {
            self.n_gate += 1;
            return None;
        }
        // funcao diferenca d(tau) e sua media cumulativa normalizada
        self.cmnd[0] = 1.0;
        let mut running = 0.0f32;
        for tau in 1..=tmax {
            let lagged = &x[tmax + 1 - tau..tmax + 1 - tau + w];
            let mut s = 0.0f32;
            for j in 0..w {
                let d = recent[j] - lagged[j];
                s += d * d;
            }
            running += s;
            self.cmnd[tau] = if running > 0.0 { s * tau as f32 / running } else { 1.0 };
        }
        // primeiro vale abaixo do limiar
        let mut tau = 0usize;
        let mut t = self.tau_min;
        while t <= tmax {
            if self.cmnd[t] < self.thr {
                while t < tmax && self.cmnd[t + 1] < self.cmnd[t] {
                    t += 1;
                }
                tau = t;
                break;
            }
            t += 1;
        }
        if tau == 0 {
            let mut best = self.tau_min;
            for t in self.tau_min..=tmax {
                if self.cmnd[t] < self.cmnd[best] {
                    best = t;
                }
            }
            if self.cmnd[best] > 0.3 {
                self.n_unclear += 1;
                return None;
            }
            tau = best;
        }
        // interpolacao parabolica
        let mut tf = tau as f32;
        if tau > 1 && tau < tmax {
            let (a, b, c) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
            let den = a - 2.0 * b + c;
            if den.abs() > 1e-9 {
                tf += 0.5 * (a - c) / den;
            }
        }
        let f0 = self.fs / tf;
        if !(self.fs / (self.tau_max as f32 + 1.0)..=self.fs / self.tau_min as f32).contains(&f0) {
            self.n_unclear += 1;
            return None;
        }
        self.n_voiced += 1;
        Some((f0, 1.0 - self.cmnd[tau]))
    }
}

// ------------------------------------------------- deslocador de altura

const BUF: usize = 1 << 14;

struct Shifter {
    buf: Vec<f32>,
    n: i64,
    a: f64,
    b: f64,
    xf_pos: u32,
    xf_len: u32,
    xf_active: bool,
    last_d: f64,
}

impl Shifter {
    fn new(initial_delay: f64) -> Self {
        Shifter { buf: vec![0.0; BUF], n: 0, a: -initial_delay, b: 0.0, xf_pos: 0, xf_len: 1, xf_active: false, last_d: initial_delay }
    }

    #[inline]
    fn read(&self, pos: f64) -> f32 {
        let i = pos.floor();
        let frac = (pos - i) as f32;
        let i = i as i64;
        let s0 = self.buf[(i as usize) & (BUF - 1)];
        let s1 = self.buf[((i + 1) as usize) & (BUF - 1)];
        s0 + (s1 - s0) * frac
    }

    /// `ratio` > 1 sobe a altura. `t` e o periodo do sinal em amostras.
    fn process(&mut self, x: f32, ratio: f64, t: f64) -> f32 {
        self.buf[(self.n as usize) & (BUF - 1)] = x;
        let nf = self.n as f64;
        let d = nf - self.a;
        self.last_d = d;
        if !self.xf_active {
            let mut jump = 0.0;
            if (ratio - 1.0).abs() > 1e-4 {
                if d < t {
                    jump = -t;
                } else if d > 3.0 * t {
                    jump = t;
                }
            }
            // guardas absolutas: nunca ler o futuro nem dados ja sobrescritos
            if d < 8.0 {
                jump = -t.max(64.0);
            } else if d > 8000.0 {
                jump = t.max(64.0);
            }
            if jump != 0.0 {
                self.b = self.a + jump;
                self.xf_pos = 0;
                self.xf_len = ((0.5 * t) as u32).max(48);
                self.xf_active = true;
            }
        }
        let out;
        if self.xf_active {
            let w = self.xf_pos as f32 / self.xf_len as f32;
            out = (1.0 - w) * self.read(self.a) + w * self.read(self.b);
            self.a += ratio;
            self.b += ratio;
            self.xf_pos += 1;
            if self.xf_pos >= self.xf_len {
                self.a = self.b;
                self.xf_active = false;
            }
        } else {
            out = self.read(self.a);
            self.a += ratio;
        }
        self.n += 1;
        out
    }
}

// ------------------------------------------------------------ autotune

#[derive(Clone, Debug)]
pub struct Config {
    pub key: u8,
    pub scale: Scale,
    /// Constante de tempo da correcao, em ms (0 = o mais rapido possivel, efeito "robo").
    pub speed_ms: f32,
    /// 0 = sem correcao, 1 = correcao total.
    pub strength: f32,
    pub fmin: f32,
    pub fmax: f32,
    /// Limiar do YIN (menor = mais exigente).
    pub threshold: f32,
    /// Histerese, em semitons, para nao ficar trocando de nota na fronteira.
    pub hysteresis: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config { key: 0, scale: Scale::Chromatic, speed_ms: 20.0, strength: 1.0, fmin: 80.0, fmax: 1000.0, threshold: 0.15, hysteresis: 0.15 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub hops: u64,
    pub voiced_hops: u64,
    pub sum_abs_corr_cents: f64,
    pub d_sum: f64,
    pub d_count: u64,
    pub d_min: f64,
    pub d_max: f64,
}

pub struct AutoTune {
    fs: f32,
    yin: Yin,
    mask: [bool; 12],
    key: u8,
    hysteresis: f32,
    strength: f32,
    alpha: f32,
    hop: usize,
    since_hop: usize,
    ring: Vec<f32>,
    scratch: Vec<f32>,
    widx: usize,
    filled: usize,
    shifter: Shifter,
    held: Option<i32>,
    unvoiced_ms: f32,
    period: f32,
    corr_target: f32,
    corr: f32,
    pub stats: Stats,
}

impl AutoTune {
    pub fn new(fs: f32, cfg: &Config) -> Self {
        let yin = Yin::new(fs, cfg.fmin, cfg.fmax, cfg.threshold);
        let n = yin.frame_len();
        let tau = cfg.speed_ms.max(1.0) / 1000.0;
        AutoTune {
            fs,
            mask: cfg.scale.mask(),
            key: cfg.key,
            hysteresis: cfg.hysteresis,
            strength: cfg.strength.clamp(0.0, 1.0),
            alpha: 1.0 - (-1.0 / (fs * tau)).exp(),
            hop: (fs * 0.005) as usize,
            since_hop: 0,
            ring: vec![0.0; n],
            scratch: vec![0.0; n],
            widx: 0,
            filled: 0,
            shifter: Shifter::new(0.012 * fs as f64),
            held: None,
            unvoiced_ms: 0.0,
            period: 0.008 * fs,
            corr_target: 0.0,
            corr: 0.0,
            yin,
            stats: Stats { d_min: f64::MAX, ..Default::default() },
        }
    }

    pub fn set_gate(&mut self, min_rms: f32) {
        self.yin.min_rms = min_rms;
    }

    /// (quadros abaixo do volume minimo, quadros sem altura clara, quadros com voz)
    pub fn detector_counts(&self) -> (u64, u64, u64) {
        (self.yin.n_gate, self.yin.n_unclear, self.yin.n_voiced)
    }

    pub fn detector_window_ms(&self) -> f32 {
        self.yin.window_ms()
    }

    pub fn hop_ms(&self) -> f32 {
        self.hop as f32 / self.fs * 1000.0
    }

    fn analyze(&mut self) {
        self.stats.hops += 1;
        let n = self.ring.len();
        if self.filled < n {
            return;
        }
        let tail = n - self.widx;
        self.scratch[..tail].copy_from_slice(&self.ring[self.widx..]);
        self.scratch[tail..].copy_from_slice(&self.ring[..self.widx]);
        match self.yin.detect(&self.scratch) {
            Some((f0, _clarity)) => {
                self.unvoiced_ms = 0.0;
                let m = hz_to_midi(f0);
                let cand = nearest_allowed(m, self.key, &self.mask);
                let target = match self.held {
                    Some(h) if cand != h && (m - cand as f32).abs() + self.hysteresis >= (m - h as f32).abs() => h,
                    _ => cand,
                };
                self.held = Some(target);
                let new_period = self.fs / f0;
                self.period = if (new_period / self.period - 1.0).abs() < 0.2 { 0.5 * self.period + 0.5 * new_period } else { new_period };
                self.corr_target = (self.strength * (target as f32 - m) * 100.0).clamp(-300.0, 300.0);
                self.stats.voiced_hops += 1;
                self.stats.sum_abs_corr_cents += self.corr_target.abs() as f64;
            }
            None => {
                self.unvoiced_ms += self.hop_ms();
                self.corr_target = 0.0;
                if self.unvoiced_ms > 60.0 {
                    self.held = None;
                }
            }
        }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        assert_eq!(input.len(), output.len());
        let n = self.ring.len();
        for (i, &x) in input.iter().enumerate() {
            self.ring[self.widx] = x;
            self.widx += 1;
            if self.widx == n {
                self.widx = 0;
            }
            if self.filled < n {
                self.filled += 1;
            }
            self.since_hop += 1;
            if self.since_hop >= self.hop {
                self.since_hop = 0;
                self.analyze();
            }
            self.corr += self.alpha * (self.corr_target - self.corr);
            let ratio = 2f64.powf(self.corr as f64 / 1200.0);
            output[i] = self.shifter.process(x, ratio, self.period as f64);
            if self.held.is_some() && self.unvoiced_ms == 0.0 {
                let d = self.shifter.last_d;
                self.stats.d_sum += d;
                self.stats.d_count += 1;
                self.stats.d_min = self.stats.d_min.min(d);
                self.stats.d_max = self.stats.d_max.max(d);
            }
        }
    }
}

// ---------------------------------------------------------- analise offline

/// Altura (em Hz) a cada ~5 ms, ou None sem voz.
pub fn pitch_track(samples: &[f32], fs: f32, fmin: f32, fmax: f32) -> Vec<Option<f32>> {
    let mut yin = Yin::new(fs, fmin, fmax, 0.15);
    let n = yin.frame_len();
    let hop = (fs * 0.005) as usize;
    let mut out = Vec::new();
    let mut pos = n;
    while pos <= samples.len() {
        out.push(yin.detect(&samples[pos - n..pos]).map(|(f, _)| f));
        pos += hop;
    }
    out
}

/// (mediana, media) do desvio absoluto, em cents, ate a nota permitida mais proxima.
pub fn scale_deviation(track: &[Option<f32>], key: u8, scale: Scale) -> Option<(f32, f32)> {
    let mask = scale.mask();
    let mut devs: Vec<f32> = track
        .iter()
        .flatten()
        .map(|&f| {
            let m = hz_to_midi(f);
            (m - nearest_allowed(m, key, &mask) as f32).abs() * 100.0
        })
        .collect();
    if devs.is_empty() {
        return None;
    }
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = devs.iter().sum::<f32>() / devs.len() as f32;
    Some((devs[devs.len() / 2], mean))
}

#[cfg(test)]
pub fn median_hz(track: &[Option<f32>]) -> Option<f32> {
    let mut v: Vec<f32> = track.iter().flatten().copied().collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(v[v.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn harmonic(fs: f32, f: f32, secs: f32, vibrato: f32) -> Vec<f32> {
        let n = (fs * secs) as usize;
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / fs;
                let fi = f * (1.0 + vibrato * (TAU * 5.5 * t).sin());
                phase += TAU * fi / fs;
                (1..=8).map(|k| (k as f32 * phase).sin() / k as f32).sum::<f32>() * 0.3
            })
            .collect()
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                ((s >> 8) as f32 / 8388608.0 - 1.0) * 0.2
            })
            .collect()
    }

    fn cents(a: f32, b: f32) -> f32 {
        1200.0 * (a / b).log2()
    }

    fn run(fs: f32, cfg: &Config, x: &[f32], block: usize) -> Vec<f32> {
        let mut at = AutoTune::new(fs, cfg);
        let mut out = vec![0.0; x.len()];
        for (i, o) in x.chunks(block).zip(out.chunks_mut(block)) {
            at.process(i, o);
        }
        out
    }

    fn tail_median(y: &[f32], fs: f32) -> f32 {
        let tail = &y[y.len() - (fs * 0.6) as usize..];
        median_hz(&pitch_track(tail, fs, 80.0, 1000.0)).expect("sem voz detectada")
    }

    #[test]
    fn yin_detects_known_pitches() {
        for &fs in &[44_100.0f32, 48_000.0] {
            for &f in &[110.0f32, 196.0, 220.0, 330.0, 440.0, 880.0] {
                let x = harmonic(fs, f, 0.3, 0.0);
                let mut yin = Yin::new(fs, 80.0, 1000.0, 0.15);
                let n = yin.frame_len();
                let (got, clarity) = yin.detect(&x[x.len() - n..]).expect("deveria detectar");
                assert!(cents(got, f).abs() < 5.0, "fs {fs} f {f}: {got}");
                assert!(clarity > 0.8);
            }
        }
    }

    #[test]
    fn yin_rejects_noise() {
        let x = noise(48_000, 7);
        let t = pitch_track(&x, 48_000.0, 80.0, 1000.0);
        let voiced = t.iter().flatten().count();
        assert!(voiced * 10 < t.len(), "ruido classificado como voz em {voiced}/{}", t.len());
    }

    #[test]
    fn key_and_scale_parsing_and_snapping() {
        assert_eq!(parse_key("C"), Some(0));
        assert_eq!(parse_key("c#"), Some(1));
        assert_eq!(parse_key("Db"), Some(1));
        assert_eq!(parse_key("Bb"), Some(10));
        assert_eq!(parse_key("H"), None);
        let major = Scale::Major.mask();
        assert_eq!(nearest_allowed(61.4, 0, &major), 62); // C# fora de Do maior -> Re
        assert_eq!(nearest_allowed(60.2, 0, &major), 60);
        assert_eq!(nearest_allowed(61.4, 1, &major), 61); // em Do# maior, Do# existe
        assert_eq!(nearest_allowed(61.4, 0, &Scale::Chromatic.mask()), 61);
    }

    #[test]
    fn detuned_tone_is_pulled_to_the_note() {
        let fs = 48_000.0;
        let f_in = 220.0 * 2f32.powf(40.0 / 1200.0);
        let x = harmonic(fs, f_in, 2.0, 0.0);
        let cfg = Config { speed_ms: 0.0, ..Default::default() };
        let y = run(fs, &cfg, &x, 128);
        let got = tail_median(&y, fs);
        assert!(cents(got, 220.0).abs() < 10.0, "saiu {got} Hz ({:.1} cents de 220)", cents(got, 220.0));
    }

    #[test]
    fn strength_scales_the_correction() {
        let fs = 48_000.0;
        let f_in = 220.0 * 2f32.powf(40.0 / 1200.0);
        let x = harmonic(fs, f_in, 2.0, 0.0);
        let cfg = Config { speed_ms: 0.0, strength: 0.5, ..Default::default() };
        let y = run(fs, &cfg, &x, 128);
        let residual = cents(tail_median(&y, fs), 220.0);
        assert!((residual - 20.0).abs() < 8.0, "residual {residual:.1} cents (esperado ~20)");
    }

    #[test]
    fn works_at_44100_and_low_voice() {
        let fs = 44_100.0;
        let f_in = 110.0 * 2f32.powf(-35.0 / 1200.0);
        let x = harmonic(fs, f_in, 2.5, 0.0);
        let y = run(fs, &Config { speed_ms: 0.0, ..Default::default() }, &x, 256);
        let got = tail_median(&y, fs);
        assert!(cents(got, 110.0).abs() < 12.0, "saiu {got} Hz");
    }

    #[test]
    fn vibrato_with_slow_speed_keeps_some_movement() {
        let fs = 48_000.0;
        let x = harmonic(fs, 220.0, 2.0, 0.012);
        let slow = run(fs, &Config { speed_ms: 120.0, ..Default::default() }, &x, 128);
        let fast = run(fs, &Config { speed_ms: 0.0, ..Default::default() }, &x, 128);
        let spread = |y: &[f32]| {
            let t = pitch_track(&y[y.len() - 48_000..], fs, 80.0, 1000.0);
            let c: Vec<f32> = t.iter().flatten().map(|&f| cents(f, 220.0)).collect();
            let mean = c.iter().sum::<f32>() / c.len() as f32;
            (c.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / c.len() as f32).sqrt()
        };
        assert!(spread(&slow) > spread(&fast) + 5.0, "lento {:.1} rapido {:.1}", spread(&slow), spread(&fast));
    }

    #[test]
    fn on_pitch_signal_keeps_level_and_pitch() {
        let fs = 48_000.0;
        let x = harmonic(fs, 220.0, 2.0, 0.0);
        let y = run(fs, &Config::default(), &x, 128);
        let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
        let n = x.len();
        let ratio_db = 20.0 * (rms(&y[n - 24_000..]) / rms(&x[n - 24_000..])).log10();
        assert!(ratio_db.abs() < 1.5, "nivel mudou {ratio_db:.2} dB");
        assert!(cents(tail_median(&y, fs), 220.0).abs() < 6.0);
    }

    #[test]
    fn unvoiced_noise_passes_as_a_pure_delay() {
        let fs = 48_000.0;
        let x = noise(48_000, 3);
        let y = run(fs, &Config::default(), &x, 128);
        let delay = (0.012 * fs) as usize;
        for i in delay + 10..x.len() {
            assert!((y[i] - x[i - delay]).abs() < 1e-3, "amostra {i}");
        }
    }

    #[test]
    fn output_does_not_depend_on_block_size() {
        let fs = 48_000.0;
        let x = harmonic(fs, 233.0, 1.0, 0.01);
        let cfg = Config { speed_ms: 10.0, ..Default::default() };
        let a = run(fs, &cfg, &x, 1);
        let b = run(fs, &cfg, &x, 333);
        let c = run(fs, &cfg, &x, 4096);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn no_large_discontinuities_while_correcting() {
        let fs = 48_000.0;
        let f_in = 220.0 * 2f32.powf(45.0 / 1200.0);
        let x = harmonic(fs, f_in, 2.0, 0.0);
        let y = run(fs, &Config { speed_ms: 0.0, ..Default::default() }, &x, 128);
        let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let max_step = y.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        let in_step = x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(peak < 1.5 * x.iter().fold(0.0f32, |m, v| m.max(v.abs())), "pico {peak}");
        assert!(max_step < 2.0 * in_step + 0.05, "salto {max_step} vs entrada {in_step}");
    }
}
