//! Efeitos da voz ao vivo: EQ de 3 bandas, compressor, eco e reverb.
//!
//! Tudo roda no callback de áudio: nada aqui aloca memória depois de `Fx::new` e os ajustes
//! chegam por átomos (`FxShared`), lidos uma vez por bloco. Com todos os efeitos em zero o sinal
//! passa intacto (`process` devolve a mesma amostra nos dois canais).

use std::f32::consts::PI;

use crate::playback::AtomicF32;

/// Ajustes dos efeitos, compartilhados com a thread de controle.
pub struct FxShared {
    /// Ganho das três bandas do EQ, em dB (−12 a +12).
    pub eq_low_db: AtomicF32,
    pub eq_mid_db: AtomicF32,
    pub eq_high_db: AtomicF32,
    /// Quanto o compressor trabalha (0 = desligado, 1 = forte).
    pub comp: AtomicF32,
    /// Volume do reverb (0 a 1) e tamanho da sala (0 a 1).
    pub reverb: AtomicF32,
    pub reverb_size: AtomicF32,
    /// Volume do eco (0 a 1) e tempo entre as repetições, em ms.
    pub echo: AtomicF32,
    pub echo_ms: AtomicF32,
}

impl FxShared {
    pub fn new() -> FxShared {
        FxShared {
            eq_low_db: AtomicF32::new(0.0),
            eq_mid_db: AtomicF32::new(0.0),
            eq_high_db: AtomicF32::new(0.0),
            comp: AtomicF32::new(0.0),
            reverb: AtomicF32::new(0.0),
            reverb_size: AtomicF32::new(0.5),
            echo: AtomicF32::new(0.0),
            echo_ms: AtomicF32::new(300.0),
        }
    }
}

impl Default for FxShared {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------------------------ biquad

/// Filtro biquad (receitas do Robert Bristow-Johnson), forma transposta II.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn identity() -> Biquad {
        Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn from_coefs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32, keep: &Biquad) -> Biquad {
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: keep.z1,
            z2: keep.z2,
        }
    }

    fn low_shelf(rate: f32, freq: f32, gain_db: f32, keep: &Biquad) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / 2.0 * 2f32.sqrt(); // S = 1
        let sq = 2.0 * a.sqrt() * alpha;
        Biquad::from_coefs(
            a * ((a + 1.0) - (a - 1.0) * cos + sq),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sq),
            (a + 1.0) + (a - 1.0) * cos + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sq,
            keep,
        )
    }

    fn high_shelf(rate: f32, freq: f32, gain_db: f32, keep: &Biquad) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / 2.0 * 2f32.sqrt();
        let sq = 2.0 * a.sqrt() * alpha;
        Biquad::from_coefs(
            a * ((a + 1.0) + (a - 1.0) * cos + sq),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sq),
            (a + 1.0) - (a - 1.0) * cos + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sq,
            keep,
        )
    }

    fn peaking(rate: f32, freq: f32, q: f32, gain_db: f32, keep: &Biquad) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * freq / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Biquad::from_coefs(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
            keep,
        )
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Abaixo disso o ganho do EQ conta como zero (a banda fica desligada).
const EQ_OFF_DB: f32 = 0.05;

/// EQ de 3 bandas: graves (prateleira em 120 Hz), médios/presença (sino em 3 kHz) e agudos
/// (prateleira em 9 kHz).
struct Eq3 {
    rate: f32,
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    gains: [f32; 3],
}

impl Eq3 {
    fn new(rate: f32) -> Eq3 {
        Eq3 {
            rate,
            low: Biquad::identity(),
            mid: Biquad::identity(),
            high: Biquad::identity(),
            gains: [0.0; 3],
        }
    }

    fn set(&mut self, low_db: f32, mid_db: f32, high_db: f32) {
        let want = [low_db, mid_db, high_db];
        if want == self.gains {
            return;
        }
        let on = |g: f32| g.abs() >= EQ_OFF_DB;
        if want[0] != self.gains[0] {
            self.low = if on(low_db) {
                Biquad::low_shelf(self.rate, 120.0, low_db, &self.low)
            } else {
                Biquad::identity()
            };
        }
        if want[1] != self.gains[1] {
            self.mid = if on(mid_db) {
                Biquad::peaking(self.rate, 3000.0, 0.8, mid_db, &self.mid)
            } else {
                Biquad::identity()
            };
        }
        if want[2] != self.gains[2] {
            self.high = if on(high_db) {
                Biquad::high_shelf(self.rate, 9000.0, high_db, &self.high)
            } else {
                Biquad::identity()
            };
        }
        self.gains = want;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let on = |g: f32| g.abs() >= EQ_OFF_DB;
        let mut y = x;
        if on(self.gains[0]) {
            y = self.low.process(y);
        }
        if on(self.gains[1]) {
            y = self.mid.process(y);
        }
        if on(self.gains[2]) {
            y = self.high.process(y);
        }
        y
    }
}

// ------------------------------------------------------------------------------- compressor

/// Compressor de pico com joelho suave. Um único botão (`amount`) regula limiar, taxa e ganho
/// de compensação, no estilo "compressão de voz": 0 = desligado, 1 = forte.
struct Compressor {
    gr_db: f32,
    attack: f32,
    release: f32,
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
    amount: f32,
}

const KNEE_DB: f32 = 6.0;

impl Compressor {
    fn new(rate: f32) -> Compressor {
        let coef = |seconds: f32| 1.0 - (-1.0 / (seconds * rate)).exp();
        Compressor {
            gr_db: 0.0,
            attack: coef(0.005),
            release: coef(0.12),
            threshold_db: -12.0,
            ratio: 1.5,
            makeup_db: 0.0,
            amount: 0.0,
        }
    }

    fn set(&mut self, amount: f32) {
        if amount == self.amount {
            return;
        }
        self.amount = amount;
        self.threshold_db = -12.0 - 18.0 * amount;
        self.ratio = 1.5 + 3.5 * amount;
        // Devolve parte do volume perdido, para o efeito não "abafar" a voz.
        self.makeup_db = 0.5 * -self.threshold_db * (1.0 - 1.0 / self.ratio);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        if self.amount < 0.01 {
            self.gr_db *= 0.999; // sai do efeito sem estalo
            return x;
        }
        let level_db = 20.0 * x.abs().max(1e-6).log10();
        let over = level_db - self.threshold_db;
        let out_db = if 2.0 * over < -KNEE_DB {
            level_db
        } else if 2.0 * over.abs() <= KNEE_DB {
            let t = over + KNEE_DB / 2.0;
            level_db + (1.0 / self.ratio - 1.0) * t * t / (2.0 * KNEE_DB)
        } else {
            self.threshold_db + over / self.ratio
        };
        let target = (level_db - out_db).max(0.0);
        let coef = if target > self.gr_db {
            self.attack
        } else {
            self.release
        };
        self.gr_db += (target - self.gr_db) * coef;
        x * 10f32.powf((self.makeup_db - self.gr_db) / 20.0)
    }
}

// ----------------------------------------------------------------------------------- reverb

/// Linha de atraso simples (anel).
struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

impl Delay {
    fn new(len: usize) -> Delay {
        Delay {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }
}

struct Comb {
    d: Delay,
    filt: f32,
}

impl Comb {
    fn new(len: usize) -> Comb {
        Comb {
            d: Delay::new(len),
            filt: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.d.buf[self.d.pos];
        self.filt = out * (1.0 - damp) + self.filt * damp;
        self.d.buf[self.d.pos] = input + self.filt * feedback;
        self.d.pos += 1;
        if self.d.pos == self.d.buf.len() {
            self.d.pos = 0;
        }
        out
    }
}

struct Allpass {
    d: Delay,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass { d: Delay::new(len) }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.d.buf[self.d.pos];
        let out = buffered - input;
        self.d.buf[self.d.pos] = input + buffered * 0.5;
        self.d.pos += 1;
        if self.d.pos == self.d.buf.len() {
            self.d.pos = 0;
        }
        out
    }
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const REVERB_PREDELAY_S: f32 = 0.018;

/// Reverb no estilo Freeverb (8 filtros em pente + 4 passa-tudo por canal), com pré-atraso para
/// a voz continuar nítida e os dois canais levemente diferentes (sensação de espaço).
struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    allpass_l: Vec<Allpass>,
    allpass_r: Vec<Allpass>,
    predelay: Delay,
    predelay_len: usize,
    feedback: f32,
    damp: f32,
    size: f32,
}

impl Reverb {
    fn new(rate: f32) -> Reverb {
        let scale = rate / 44_100.0;
        let len = |n: usize| ((n as f32 * scale) as usize).max(8);
        let spread = (STEREO_SPREAD as f32 * scale) as usize;
        let predelay_len = ((REVERB_PREDELAY_S * rate) as usize).max(1);
        let mut r = Reverb {
            combs_l: COMB_TUNING.iter().map(|&n| Comb::new(len(n))).collect(),
            combs_r: COMB_TUNING
                .iter()
                .map(|&n| Comb::new(len(n) + spread))
                .collect(),
            allpass_l: ALLPASS_TUNING
                .iter()
                .map(|&n| Allpass::new(len(n)))
                .collect(),
            allpass_r: ALLPASS_TUNING
                .iter()
                .map(|&n| Allpass::new(len(n) + spread))
                .collect(),
            predelay: Delay::new(predelay_len),
            predelay_len,
            feedback: 0.0,
            damp: 0.4,
            size: -1.0,
        };
        r.set_size(0.5);
        r
    }

    fn set_size(&mut self, size: f32) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.feedback = 0.7 + 0.28 * size; // sala pequena (0,7) a catedral (0,98)
        self.damp = 0.5 - 0.25 * size; // salas grandes são um pouco mais brilhantes
    }

    /// Só a parte molhada, já em estéreo.
    #[inline]
    fn process(&mut self, x: f32) -> (f32, f32) {
        let delayed = self.predelay.buf[self.predelay.pos];
        self.predelay.buf[self.predelay.pos] = x;
        self.predelay.pos += 1;
        if self.predelay.pos == self.predelay_len {
            self.predelay.pos = 0;
        }
        let input = delayed * 0.015;
        let (mut l, mut r) = (0.0, 0.0);
        for c in &mut self.combs_l {
            l += c.process(input, self.feedback, self.damp);
        }
        for c in &mut self.combs_r {
            r += c.process(input, self.feedback, self.damp);
        }
        for a in &mut self.allpass_l {
            l = a.process(l);
        }
        for a in &mut self.allpass_r {
            r = a.process(r);
        }
        (l * 3.0, r * 3.0)
    }
}

// -------------------------------------------------------------------------------------- eco

const ECHO_MAX_S: f32 = 1.0;
const ECHO_FEEDBACK: f32 = 0.38;

/// Eco com filtro no retorno: cada repetição fica mais escura (como um eco de fita).
struct Echo {
    buf: Vec<f32>,
    pos: usize,
    rate: f32,
    delay: f32,
    target: f32,
    lp: f32,
    lp_coef: f32,
}

impl Echo {
    fn new(rate: f32) -> Echo {
        Echo {
            buf: vec![0.0; (ECHO_MAX_S * rate) as usize + 4],
            pos: 0,
            rate,
            delay: 0.3 * rate,
            target: 0.3 * rate,
            lp: 0.0,
            lp_coef: 1.0 - (-2.0 * PI * 3500.0 / rate).exp(),
        }
    }

    fn set_time(&mut self, ms: f32) {
        self.target = (ms / 1000.0 * self.rate).clamp(8.0, (self.buf.len() - 4) as f32);
    }

    /// Só a parte molhada.
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        // O tempo desliza até o novo valor (evita estalo ao mexer no controle).
        self.delay += (self.target - self.delay) * 0.0005;
        let len = self.buf.len();
        let read = (self.pos as f32 + len as f32 - self.delay) % len as f32;
        let i = read as usize;
        let frac = read - i as f32;
        let a = self.buf[i];
        let b = self.buf[(i + 1) % len];
        let echo = a + (b - a) * frac;
        self.lp += (echo - self.lp) * self.lp_coef;
        self.buf[self.pos] = x + self.lp * ECHO_FEEDBACK;
        self.pos += 1;
        if self.pos == len {
            self.pos = 0;
        }
        echo
    }
}

// ----------------------------------------------------------------------------------- cadeia

/// Mistura abaixo disso conta como desligada (pula o processamento).
const MIX_OFF: f32 = 0.002;

/// Cadeia completa: EQ → compressor → eco → reverb.
pub struct Fx {
    eq: Eq3,
    comp: Compressor,
    reverb: Reverb,
    echo: Echo,
    reverb_mix: f32,
    reverb_target: f32,
    echo_mix: f32,
    echo_target: f32,
    smooth: f32,
}

impl Fx {
    pub fn new(rate: u32) -> Fx {
        let rate = rate as f32;
        Fx {
            eq: Eq3::new(rate),
            comp: Compressor::new(rate),
            reverb: Reverb::new(rate),
            echo: Echo::new(rate),
            reverb_mix: 0.0,
            reverb_target: 0.0,
            echo_mix: 0.0,
            echo_target: 0.0,
            smooth: 1.0 - (-1.0 / (0.02 * rate)).exp(),
        }
    }

    /// Uma vez por bloco: lê os ajustes e atualiza coeficientes só se mudaram.
    pub fn begin_block(&mut self, s: &FxShared) {
        self.eq.set(
            s.eq_low_db.load().clamp(-12.0, 12.0),
            s.eq_mid_db.load().clamp(-12.0, 12.0),
            s.eq_high_db.load().clamp(-12.0, 12.0),
        );
        self.comp.set(s.comp.load().clamp(0.0, 1.0));
        self.reverb.set_size(s.reverb_size.load().clamp(0.0, 1.0));
        self.reverb_target = s.reverb.load().clamp(0.0, 1.0);
        self.echo.set_time(s.echo_ms.load().clamp(60.0, 800.0));
        self.echo_target = s.echo.load().clamp(0.0, 1.0);
    }

    /// Voz mono entra; saem os dois canais (a voz seca no centro e o reverb em estéreo).
    #[inline]
    pub fn process(&mut self, x: f32) -> (f32, f32) {
        self.reverb_mix += (self.reverb_target - self.reverb_mix) * self.smooth;
        self.echo_mix += (self.echo_target - self.echo_mix) * self.smooth;

        let dry = self.comp.process(self.eq.process(x));
        let echo_on = self.echo_mix > MIX_OFF || self.echo_target > MIX_OFF;
        let echo = if echo_on {
            self.echo.process(dry) * self.echo_mix * 0.8
        } else {
            0.0
        };
        let reverb_on = self.reverb_mix > MIX_OFF || self.reverb_target > MIX_OFF;
        if !reverb_on {
            let y = dry + echo;
            return (y, y);
        }
        // O reverb também "ouve" o eco: as repetições ganham espaço.
        let (wl, wr) = self.reverb.process(dry + echo * 0.7);
        (
            dry + echo + wl * self.reverb_mix,
            dry + echo + wr * self.reverb_mix,
        )
    }

    /// Para os testes: o que está ligado agora.
    #[cfg(test)]
    fn all_off(&self) -> bool {
        self.reverb_target <= MIX_OFF && self.echo_target <= MIX_OFF && self.comp.amount < 0.01
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * PI * freq * i as f32 / R as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-9).log10()
    }

    fn run(fx: &mut Fx, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for chunk in input.chunks(480) {
            for &x in chunk {
                let (a, b) = fx.process(x);
                l.push(a);
                r.push(b);
            }
        }
        (l, r)
    }

    fn settled(fx: &mut Fx, s: &FxShared) {
        fx.begin_block(s);
        // deixa os suavizadores chegarem ao valor
        for _ in 0..R / 2 {
            fx.process(0.0);
        }
    }

    #[test]
    fn everything_off_is_bit_exact_passthrough() {
        let s = FxShared::new();
        let mut fx = Fx::new(R);
        fx.begin_block(&s);
        assert!(fx.all_off());
        let input = sine(440.0, 0.5, 4800);
        let (l, r) = run(&mut fx, &input);
        assert_eq!(l, input);
        assert_eq!(r, input);
    }

    #[test]
    fn eq_boosts_and_cuts_the_right_bands() {
        let gain_at = |freq: f32, low: f32, mid: f32, high: f32| {
            let s = FxShared::new();
            s.eq_low_db.store(low);
            s.eq_mid_db.store(mid);
            s.eq_high_db.store(high);
            let mut fx = Fx::new(R);
            fx.begin_block(&s);
            let input = sine(freq, 0.1, 48_000);
            let (l, _) = run(&mut fx, &input);
            db(rms(&l[24_000..]) / rms(&input[24_000..]))
        };
        assert!((gain_at(60.0, 9.0, 0.0, 0.0) - 9.0).abs() < 1.5);
        assert!(gain_at(3000.0, 9.0, 0.0, 0.0).abs() < 1.0); // graves não mexem em 3 kHz
        assert!((gain_at(3000.0, 0.0, 6.0, 0.0) - 6.0).abs() < 0.8);
        assert!((gain_at(14_000.0, 0.0, 0.0, -9.0) + 9.0).abs() < 2.0);
        assert!(gain_at(1000.0, 0.0, 0.0, 0.0).abs() < 0.01);
    }

    #[test]
    fn compressor_reduces_dynamic_range() {
        let level_diff = |comp: f32| {
            let s = FxShared::new();
            s.comp.store(comp);
            let mut fx = Fx::new(R);
            settled(&mut fx, &s);
            fx.begin_block(&s);
            let loud = sine(300.0, 0.7, 24_000);
            let quiet = sine(300.0, 0.05, 24_000);
            let (l, _) = run(&mut fx, &loud);
            let loud_rms = rms(&l[12_000..]);
            let (q, _) = run(&mut fx, &quiet);
            let quiet_rms = rms(&q[18_000..]);
            db(loud_rms) - db(quiet_rms)
        };
        let off = level_diff(0.0);
        let on = level_diff(0.8);
        assert!((off - 22.9).abs() < 0.5, "sem compressão: {off}");
        assert!(
            on < off - 6.0,
            "com compressão a diferença cai: {on} vs {off}"
        );
    }

    #[test]
    fn compressor_never_blows_up_on_peaks() {
        let s = FxShared::new();
        s.comp.store(1.0);
        let mut fx = Fx::new(R);
        fx.begin_block(&s);
        let input = sine(200.0, 1.0, 48_000);
        let (l, _) = run(&mut fx, &input);
        assert!(l.iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }

    #[test]
    fn reverb_adds_a_tail_that_decays_and_grows_with_room_size() {
        let tail_energy = |size: f32| {
            let s = FxShared::new();
            s.reverb.store(0.5);
            s.reverb_size.store(size);
            let mut fx = Fx::new(R);
            settled(&mut fx, &s);
            fx.begin_block(&s);
            let mut input = vec![0.0; R as usize * 3];
            for v in input.iter_mut().take(480) {
                *v = 0.5; // um estalo curto
            }
            let (l, _) = run(&mut fx, &input);
            // energia entre 1 s e 2 s depois do estalo
            rms(&l[R as usize..R as usize * 2])
        };
        let small = tail_energy(0.1);
        let big = tail_energy(1.0);
        assert!(big > small * 2.0, "sala grande dura mais: {big} vs {small}");
        assert!(big > 1e-4, "tem cauda audível: {big}");
    }

    #[test]
    fn reverb_is_stereo_and_stays_finite_and_stable() {
        let s = FxShared::new();
        s.reverb.store(1.0);
        s.reverb_size.store(1.0);
        let mut fx = Fx::new(R);
        settled(&mut fx, &s);
        fx.begin_block(&s);
        let input = sine(440.0, 0.8, R as usize * 4);
        let (l, r) = run(&mut fx, &input);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
        assert!(l.iter().chain(r.iter()).all(|v| v.abs() < 8.0));
        // os canais não são idênticos (sensação de espaço)
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
        assert!(diff > 1e-3, "diferença média {diff}");
    }

    #[test]
    fn echo_repeats_after_the_set_time_and_fades() {
        let s = FxShared::new();
        s.echo.store(1.0);
        s.echo_ms.store(250.0);
        let mut fx = Fx::new(R);
        // o tempo desliza até o valor: dá um tempo para chegar
        fx.begin_block(&s);
        for _ in 0..R * 3 {
            fx.process(0.0);
        }
        let mut input = vec![0.0; R as usize * 2];
        for v in input.iter_mut().take(100) {
            *v = 0.5;
        }
        let (l, _) = run(&mut fx, &input);
        let window = |from_ms: usize, len_ms: usize| {
            let a = from_ms * 48;
            rms(&l[a..a + len_ms * 48])
        };
        let first = window(240, 30); // a 1ª repetição, ~250 ms depois
        let between = window(120, 60);
        let second = window(490, 30); // a 2ª, ~500 ms
        assert!(
            first > between * 5.0,
            "1ª repetição: {first} vs vazio {between}"
        );
        assert!(
            second > 1e-4 && second < first,
            "2ª mais fraca: {second} vs {first}"
        );
    }

    #[test]
    fn mix_changes_are_smooth_no_clicks() {
        let s = FxShared::new();
        let mut fx = Fx::new(R);
        fx.begin_block(&s);
        let input = sine(300.0, 0.3, 9600);
        let (a, _) = run(&mut fx, &input);
        s.reverb.store(1.0);
        s.echo.store(1.0);
        fx.begin_block(&s);
        let (b, _) = run(&mut fx, &input);
        let max_step = |x: &[f32]| {
            x.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(max_step(&b) < max_step(&a) * 3.0 + 0.05);
    }
}
