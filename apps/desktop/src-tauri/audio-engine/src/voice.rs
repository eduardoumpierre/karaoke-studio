//! Voz ao vivo: a ponte entre o microfone (um relógio) e a saída (outro relógio) e a cadeia
//! de processamento da voz. Tudo aqui é lógica pura, sem dispositivo, e roda no callback.
//!
//! Fluxo: callback de entrada → `Ring` (mono) → `VoiceOut` (reamostra e compensa a deriva
//! entre os relógios) → cadeia (ganho, passa-altas, portão, autotune, efeitos) → somada à música no `Playback`.
//!
//! O microfone e a saída podem ser dispositivos diferentes (por exemplo, microfone do headset
//! e caixas USB), então nunca andam exatamente na mesma velocidade. A ponte mantém um pequeno
//! estoque de amostras e ajusta a velocidade de leitura em frações de 1% para ele não esvaziar
//! nem encher.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crate::fx::{Fx, FxShared};
use crate::playback::AtomicF32;
use crate::tune::{Tune, TuneShared};

const RING_SIZE: usize = 1 << 16;
const MASK: u64 = (RING_SIZE as u64) - 1;

/// Estoque-alvo inicial da ponte (ms). Sobe sozinho se faltarem amostras (underrun).
pub const TARGET_MS: f64 = 25.0;
const TARGET_MAX_MS: f64 = 80.0;
/// Quanto a velocidade de leitura pode variar para compensar a deriva (±0,5%).
const MAX_ADJUST: f64 = 0.005;
/// Ganho do controlador (por segundo de erro de estoque).
const KP: f64 = 0.1;

/// Fila circular de uma amostra por posição: um produtor (callback do microfone) e um
/// consumidor (callback da saída). Sem alocação nem travas.
pub struct Ring {
    buf: Vec<AtomicU32>,
    head: AtomicU64,
}

impl Ring {
    pub fn new() -> Arc<Ring> {
        Arc::new(Ring {
            buf: (0..RING_SIZE).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicU64::new(0),
        })
    }

    /// Escreve `n` amostras (produtor). Se o consumidor estiver parado, sobrescreve as antigas.
    pub fn write(&self, n: usize, mut sample: impl FnMut(usize) -> f32) {
        let h = self.head.load(Ordering::Relaxed);
        for i in 0..n {
            self.buf[((h + i as u64) & MASK) as usize]
                .store(sample(i).to_bits(), Ordering::Relaxed);
        }
        self.head.store(h + n as u64, Ordering::Release);
    }

    /// Total de amostras já escritas.
    pub fn head(&self) -> u64 {
        self.head.load(Ordering::Acquire)
    }

    #[inline]
    fn get(&self, index: u64) -> f32 {
        f32::from_bits(self.buf[(index & MASK) as usize].load(Ordering::Relaxed))
    }
}

/// Ajustes e medidas da voz, compartilhados com a thread de controle.
pub struct VoiceShared {
    /// Ouvir a voz na saída (retorno). Desligado = só mede o nível do microfone.
    pub monitor: AtomicBool,
    /// Volume do retorno (ganho linear, 0 a 1,5).
    pub volume: AtomicF32,
    /// Ganho de entrada (linear).
    pub input_gain: AtomicF32,
    pub gate_on: AtomicBool,
    /// Limiar do portão (amplitude linear).
    pub gate_threshold: AtomicF32,
    /// Estoque atual da ponte, em ms.
    pub fill_ms: AtomicF32,
    pub target_ms: AtomicF32,
    pub underruns: AtomicU64,
    /// EQ, compressor, eco e reverb.
    pub fx: FxShared,
    /// Autotune.
    pub tune: TuneShared,
}

impl VoiceShared {
    pub fn new() -> Arc<VoiceShared> {
        Arc::new(VoiceShared {
            monitor: AtomicBool::new(false),
            volume: AtomicF32::new(0.7),
            input_gain: AtomicF32::new(1.0),
            gate_on: AtomicBool::new(true),
            gate_threshold: AtomicF32::new(0.01),
            fill_ms: AtomicF32::new(0.0),
            target_ms: AtomicF32::new(TARGET_MS as f32),
            underruns: AtomicU64::new(0),
            fx: FxShared::new(),
            tune: TuneShared::new(),
        })
    }
}

/// Filtro passa-altas de 2ª ordem (Butterworth): tira graves de manuseio, vento e vibração.
struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl HighPass {
    fn new(rate: f32, cutoff: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * cutoff / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        HighPass {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Portão de ruído: abaixo do limiar (com histerese e retenção) a voz é abafada em ~30 dB,
/// sem cortar de vez (soa mais natural e evita "pumping" seco).
struct Gate {
    env: f32,
    gain: f32,
    hold: u32,
    hold_samples: u32,
    env_decay: f32,
    attack: f32,
    release: f32,
    open: bool,
}

const GATE_FLOOR: f32 = 0.03;

impl Gate {
    fn new(rate: f32) -> Self {
        let coef = |seconds: f32| 1.0 - (-1.0 / (seconds * rate)).exp();
        Gate {
            env: 0.0,
            gain: 1.0,
            hold: 0,
            hold_samples: (0.15 * rate) as u32,
            env_decay: (-1.0 / (0.02 * rate)).exp(),
            attack: coef(0.003),
            release: coef(0.08),
            open: true,
        }
    }

    #[inline]
    fn process(&mut self, x: f32, enabled: bool, threshold: f32) -> f32 {
        if !enabled {
            self.gain += (1.0 - self.gain) * self.attack;
            return x * self.gain;
        }
        self.env = x.abs().max(self.env * self.env_decay);
        // Histerese: para fechar, o nível precisa cair bem abaixo do que abriu.
        let limit = if self.open {
            threshold * 0.7
        } else {
            threshold
        };
        self.open = self.env > limit;
        if self.open {
            self.hold = self.hold_samples;
        } else if self.hold > 0 {
            self.hold -= 1;
        }
        let target = if self.open || self.hold > 0 {
            1.0
        } else {
            GATE_FLOOR
        };
        let coef = if target > self.gain {
            self.attack
        } else {
            self.release
        };
        self.gain += (target - self.gain) * coef;
        x * self.gain
    }
}

/// Lado da saída: lê o microfone da ponte já na taxa da saída e aplica a cadeia.
pub struct VoiceOut {
    ring: Arc<Ring>,
    shared: Arc<VoiceShared>,
    in_rate: f64,
    base_ratio: f64,
    pos: f64,
    head: u64,
    primed: bool,
    /// Só volta a tocar quando o microfone tiver escrito amostras novas além deste ponto.
    prime_after: u64,
    target: f64,
    fill_smooth: f64,
    adjust: f64,
    level: f32,
    level_coef: f32,
    hpf: HighPass,
    gate: Gate,
    tune: Tune,
    fx: Fx,
}

impl VoiceOut {
    pub fn new(ring: Arc<Ring>, shared: Arc<VoiceShared>, in_rate: u32, out_rate: u32) -> Self {
        let target = TARGET_MS / 1000.0 * in_rate as f64;
        VoiceOut {
            ring,
            shared,
            in_rate: in_rate as f64,
            base_ratio: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            head: 0,
            primed: false,
            prime_after: 0,
            target,
            fill_smooth: target,
            adjust: 0.0,
            level: 0.0,
            level_coef: 1.0 - (-1.0 / (0.01 * out_rate as f32)).exp(),
            hpf: HighPass::new(out_rate as f32, 80.0),
            gate: Gate::new(out_rate as f32),
            tune: Tune::new(out_rate),
            fx: Fx::new(out_rate),
        }
    }

    /// Chamar uma vez no começo de cada callback da saída.
    pub fn begin_block(&mut self) {
        self.head = self.ring.head();
        self.fx.begin_block(&self.shared.fx);
        self.tune.begin_block(&self.shared.tune);
        if !self.shared.monitor.load(Ordering::Relaxed) {
            // Sem retorno: não consome. Ao religar, recomeça no estoque-alvo (sem atraso acumulado).
            self.primed = false;
            return;
        }
        if !self.primed {
            if self.head as f64 >= self.target + 8.0 && self.head >= self.prime_after {
                self.pos = self.head as f64 - self.target;
                self.fill_smooth = self.target;
                self.adjust = 0.0;
                self.primed = true;
            }
            return;
        }
        let fill = self.head as f64 - self.pos;
        if fill > (RING_SIZE / 2) as f64 {
            // Ficou muito para trás (o sistema travou): descarta o velho e segue ao vivo.
            self.pos = self.head as f64 - self.target;
            self.fill_smooth = self.target;
            return;
        }
        self.fill_smooth += (fill - self.fill_smooth) * 0.05;
        let desired =
            (KP * (self.fill_smooth - self.target) / self.in_rate).clamp(-MAX_ADJUST, MAX_ADJUST);
        self.adjust += (desired - self.adjust) * 0.05;
        self.shared
            .fill_ms
            .store((self.fill_smooth / self.in_rate * 1000.0) as f32);
    }

    /// Próxima amostra de voz (já com efeitos e volume do retorno), uma por canal.
    #[inline]
    pub fn next_stereo(&mut self) -> (f32, f32) {
        let monitor = self.shared.monitor.load(Ordering::Relaxed);
        let target_level = if monitor {
            self.shared.volume.load()
        } else {
            0.0
        };
        self.level += (target_level - self.level) * self.level_coef;
        if !self.primed {
            return (0.0, 0.0);
        }
        let i = self.pos as u64;
        if i < 1 || i + 2 >= self.head {
            // Faltou amostra: silêncio, aumenta o estoque-alvo e espera encher de novo.
            self.primed = false;
            self.target = (self.target * 1.25).min(TARGET_MAX_MS / 1000.0 * self.in_rate);
            self.shared
                .target_ms
                .store((self.target / self.in_rate * 1000.0) as f32);
            self.prime_after = self.head + self.target as u64;
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            return (0.0, 0.0);
        }
        let f = (self.pos - i as f64) as f32;
        let (p0, p1, p2, p3) = (
            self.ring.get(i - 1),
            self.ring.get(i),
            self.ring.get(i + 1),
            self.ring.get(i + 2),
        );
        // Interpolação de Hermite de 4 pontos: bem mais fiel que a linear nos agudos da voz.
        let c1 = 0.5 * (p2 - p0);
        let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
        let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
        let x = ((c3 * f + c2) * f + c1) * f + p1;
        self.pos += self.base_ratio * (1.0 + self.adjust);

        let x = x * self.shared.input_gain.load();
        let x = self.hpf.process(x);
        let x = self.gate.process(
            x,
            self.shared.gate_on.load(Ordering::Relaxed),
            self.shared.gate_threshold.load(),
        );
        // Autotune logo após o portão: corrige a voz limpa, antes de EQ, compressor e reverb.
        let x = self.tune.process(x);
        let (l, r) = self.fx.process(x);
        (l * self.level, r * self.level)
    }

    /// Versão mono (média dos canais): usada pelos testes de ponte e de taxa.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        let (l, r) = self.next_stereo();
        (l + r) * 0.5
    }

    /// Estimativa do atraso que o app adiciona (estoque da ponte), em ms.
    pub fn bridge_ms(&self) -> f32 {
        (self.fill_smooth / self.in_rate * 1000.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    fn sine(rate: f64, freq: f64, amp: f32, n: usize, offset: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                amp * (2.0 * std::f64::consts::PI * freq * (i + offset) as f64 / rate).sin() as f32
            })
            .collect()
    }

    fn setup(in_rate: u32, out_rate: u32) -> (Arc<Ring>, Arc<VoiceShared>, VoiceOut) {
        let ring = Ring::new();
        let shared = VoiceShared::new();
        shared.monitor.store(true, Ordering::Relaxed);
        shared.volume.store(1.0);
        shared.gate_on.store(false, Ordering::Relaxed);
        let out = VoiceOut::new(ring.clone(), shared.clone(), in_rate, out_rate);
        (ring, shared, out)
    }

    fn pull(out: &mut VoiceOut, n: usize) -> Vec<f32> {
        out.begin_block();
        (0..n).map(|_| out.next()).collect()
    }

    /// Frequência estimada por cruzamentos de zero (subindo).
    fn freq(x: &[f32], rate: f64) -> f64 {
        let crossings: Vec<usize> = (1..x.len())
            .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
            .collect();
        let (a, b) = (crossings[0], *crossings.last().unwrap());
        (crossings.len() - 1) as f64 * rate / (b - a) as f64
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn ring_keeps_order_across_wraparound() {
        let ring = Ring::new();
        let mut counter = 0u64;
        for _ in 0..20 {
            ring.write(10_000, |i| (counter + i as u64) as f32);
            counter += 10_000;
            let h = ring.head();
            assert_eq!(h, counter);
            assert_eq!(ring.get(h - 1), (counter - 1) as f32);
            assert_eq!(ring.get(h - 10_000), (counter - 10_000) as f32);
        }
    }

    #[test]
    fn passes_a_sine_through_at_the_same_rate() {
        let (ring, _s, mut out) = setup(R, R);
        let input = sine(R as f64, 440.0, 0.5, 48_000, 0);
        let mut got = Vec::new();
        for chunk in input.chunks(480) {
            ring.write(chunk.len(), |i| chunk[i]);
            got.extend(pull(&mut out, 480));
        }
        let tail = &got[24_000..];
        assert!((freq(tail, R as f64) - 440.0).abs() < 1.0);
        // O passa-altas de 80 Hz quase não mexe em 440 Hz.
        assert!(
            (rms(tail) - 0.5 / 2f32.sqrt()).abs() < 0.02,
            "rms {}",
            rms(tail)
        );
    }

    #[test]
    fn converts_between_sample_rates_without_changing_pitch() {
        let (ring, _s, mut out) = setup(44_100, 48_000);
        let input = sine(44_100.0, 1000.0, 0.5, 44_100 * 2, 0);
        let mut got = Vec::new();
        for chunk in input.chunks(441) {
            ring.write(chunk.len(), |i| chunk[i]);
            got.extend(pull(&mut out, 480));
        }
        let tail = &got[24_000..];
        assert!(
            (freq(tail, 48_000.0) - 1000.0).abs() < 3.0,
            "freq {}",
            freq(tail, 48_000.0)
        );
        assert!(rms(tail) > 0.3);
    }

    fn run_drift(ppm: f64, seconds: usize) -> (VoiceOut, Arc<VoiceShared>, Vec<f32>) {
        let (ring, shared, mut out) = setup(R, R);
        let mut produced = 0.0f64;
        let mut written = 0usize;
        let mut got = Vec::new();
        let mut min_fill = f64::MAX;
        let mut max_fill = 0.0f64;
        let sizes = [480usize, 440, 520, 480, 500, 460];
        for block in 0..(seconds * 100) {
            // Produtor: média de 480 * (1 + ppm) amostras por bloco, em pedaços irregulares.
            produced += 480.0 * (1.0 + ppm * 1e-6);
            let target_total = produced as usize;
            while written < target_total {
                let n = sizes[block % sizes.len()].min(target_total - written);
                let phase = written;
                ring.write(n, |i| {
                    (2.0 * std::f64::consts::PI * 300.0 * (phase + i) as f64 / R as f64).sin()
                        as f32
                        * 0.5
                });
                written += n;
            }
            got.extend(pull(&mut out, 480));
            if block > 500 {
                let f = shared.fill_ms.load() as f64;
                min_fill = min_fill.min(f);
                max_fill = max_fill.max(f);
            }
        }
        let _ = (min_fill, max_fill);
        (out, shared, got)
    }

    #[test]
    fn fast_microphone_clock_does_not_overflow_or_add_delay() {
        let (out, shared, got) = run_drift(300.0, 120); // microfone 300 ppm mais rápido
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
        let fill = out.bridge_ms();
        assert!(
            (fill - TARGET_MS as f32).abs() < 4.0,
            "estoque final {fill} ms"
        );
        assert!((freq(&got[480_000..], R as f64) - 300.0).abs() < 0.5);
    }

    #[test]
    fn slow_microphone_clock_does_not_run_dry() {
        let (out, shared, got) = run_drift(-300.0, 120);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
        assert!(
            (out.bridge_ms() - TARGET_MS as f32).abs() < 4.0,
            "estoque {}",
            out.bridge_ms()
        );
        // Sem saltos: a voz de 300 Hz não pode ter degraus bruscos.
        let max_step = got[100_000..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 0.1, "degrau de {max_step}");
    }

    #[test]
    fn underrun_gives_silence_raises_target_and_recovers() {
        let (ring, shared, mut out) = setup(R, R);
        let tone = sine(R as f64, 300.0, 0.5, 4800, 0);
        ring.write(4800, |i| tone[i]);
        let first = pull(&mut out, 480);
        assert!(rms(&first[100..]) > 0.1);
        // Produtor para: a ponte esvazia.
        let mut silent = Vec::new();
        for _ in 0..20 {
            silent.extend(pull(&mut out, 480));
        }
        assert!(shared.underruns.load(Ordering::Relaxed) >= 1);
        assert!(silent.iter().all(|v| v.is_finite()));
        assert!(shared.target_ms.load() > TARGET_MS as f32);
        assert!(rms(&silent[silent.len() - 480..]) < 1e-6);
        // Volta a ter dados: retoma sozinha.
        ring.write(9600, |i| tone[i % tone.len()]);
        let mut again = Vec::new();
        for _ in 0..4 {
            again.extend(pull(&mut out, 480));
        }
        assert!(rms(&again[960..]) > 0.1);
    }

    #[test]
    fn monitor_off_consumes_nothing_and_reenables_at_target_latency() {
        let (ring, shared, mut out) = setup(R, R);
        shared.monitor.store(false, Ordering::Relaxed);
        for _ in 0..500 {
            ring.write(480, |i| (i as f32 * 0.001).sin() * 0.3);
            let block = pull(&mut out, 480);
            assert!(block.iter().all(|&v| v == 0.0));
        }
        shared.monitor.store(true, Ordering::Relaxed);
        ring.write(480, |_| 0.2);
        out.begin_block();
        assert!(out.primed);
        assert!((out.bridge_ms() - TARGET_MS as f32).abs() < 1.0);
    }

    #[test]
    fn a_stalled_consumer_jumps_back_to_live_instead_of_playing_old_audio() {
        let (ring, _s, mut out) = setup(R, R);
        ring.write(2000, |_| 0.1);
        pull(&mut out, 480);
        // O callback da saída "trava" enquanto o microfone continua escrevendo 1 s.
        for _ in 0..100 {
            ring.write(480, |_| 0.1);
        }
        out.begin_block();
        let fill = out.head as f64 - out.pos;
        assert!(
            fill < RING_SIZE as f64 / 2.0 && fill <= out.target + 1.0,
            "fill {fill}"
        );
    }

    #[test]
    fn high_pass_removes_rumble_but_keeps_the_voice() {
        let mut hp = HighPass::new(R as f32, 80.0);
        let low: Vec<f32> = sine(R as f64, 20.0, 0.5, 48_000, 0)
            .iter()
            .map(|&x| hp.process(x))
            .collect();
        let mut hp = HighPass::new(R as f32, 80.0);
        let mid: Vec<f32> = sine(R as f64, 1000.0, 0.5, 48_000, 0)
            .iter()
            .map(|&x| hp.process(x))
            .collect();
        assert!(rms(&low[24_000..]) < 0.5 / 2f32.sqrt() * 0.1);
        assert!((rms(&mid[24_000..]) - 0.5 / 2f32.sqrt()).abs() < 0.01);
    }

    #[test]
    fn gate_mutes_noise_passes_voice_and_can_be_bypassed() {
        let mut g = Gate::new(R as f32);
        let noise: Vec<f32> = sine(R as f64, 700.0, 0.003, 48_000, 0)
            .iter()
            .map(|&x| g.process(x, true, 0.01))
            .collect();
        assert!(
            rms(&noise[24_000..]) < 0.003 / 2f32.sqrt() * 0.1,
            "ruído passou: {}",
            rms(&noise[24_000..])
        );
        let voice: Vec<f32> = sine(R as f64, 700.0, 0.3, 48_000, 0)
            .iter()
            .map(|&x| g.process(x, true, 0.01))
            .collect();
        assert!(
            rms(&voice[4_800..]) > 0.3 / 2f32.sqrt() * 0.97,
            "voz cortada: {}",
            rms(&voice[4_800..])
        );
        // Silêncio depois da voz: fecha, mas só depois da retenção.
        let after: Vec<f32> = (0..48_000).map(|_| g.process(0.002, true, 0.01)).collect();
        assert!(after[0] > 0.0015 && after[after.len() - 1].abs() < 0.0002);
        // Desligado: passa tudo.
        let mut g = Gate::new(R as f32);
        let out: Vec<f32> = sine(R as f64, 700.0, 0.003, 48_000, 0)
            .iter()
            .map(|&x| g.process(x, false, 0.01))
            .collect();
        assert!((rms(&out[4_800..]) - 0.003 / 2f32.sqrt()).abs() < 1e-4);
    }

    #[test]
    fn input_gain_and_volume_scale_the_monitor() {
        let (ring, shared, mut out) = setup(R, R);
        shared.input_gain.store(2.0);
        shared.volume.store(0.5);
        let tone = sine(R as f64, 1000.0, 0.2, 4800, 0);
        let mut got = Vec::new();
        for _ in 0..20 {
            ring.write(480, |i| tone[i]);
            got.extend(pull(&mut out, 480));
        }
        let expected = 0.2 / 2f32.sqrt() * 2.0 * 0.5;
        assert!(
            (rms(&got[9_000..]) - expected).abs() < 0.01,
            "rms {}",
            rms(&got[9_000..])
        );
    }
}
