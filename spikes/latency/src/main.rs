//! Spike 3a v2 (Fase 0): latencia de ida e volta (saida -> entrada) com cpal, WASAPI compartilhado.
//!
//! A cada segundo o programa emite um "chirp" (varredura de 1 a 8 kHz, 50 ms) pela saida e grava
//! 1 s da entrada. Depois procura o chirp na gravacao por correlacao cruzada normalizada (ncc):
//!   - a posicao do pico da correlacao da o atraso;
//!   - o valor do pico (0 a 1) diz o quanto a gravacao se parece com o chirp. Ruido, fala e
//!     teclado dao valores baixos e a medida e rejeitada, em vez de virar uma "latencia" falsa.
//!
//! O caminho medido inclui: buffer de saida do app, motor de audio do Windows, DAC, o ar ou o
//! cabo, ADC, motor de audio, buffer de entrada do app. O relogio e o do sistema (Instant) lido
//! na entrada de cada callback, entao ha incerteza de poucos ms: veja minimo e mediana.

use std::sync::atomic::{
    AtomicBool, AtomicU32, AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, SampleFormat, SupportedBufferSize};

const CHIRP_SECONDS: f32 = 0.05;
const WINDOW_SECONDS: f64 = 1.0;
const SEARCH_SECONDS: f64 = 0.7;

struct Shared {
    fire: AtomicBool,
    capturing: AtomicBool,
    done: AtomicBool,
    emit_ns: AtomicU64,
    cap_t0_ns: AtomicU64,
    cap_len: AtomicU64,
    cap: Vec<AtomicU32>,
    in_peak_bits: AtomicU32,
    out_cb_frames: AtomicU64,
    in_cb_frames: AtomicU64,
}

impl Shared {
    fn new(cap_samples: usize) -> Self {
        Shared {
            fire: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            done: AtomicBool::new(false),
            emit_ns: AtomicU64::new(0),
            cap_t0_ns: AtomicU64::new(0),
            cap_len: AtomicU64::new(0),
            cap: (0..cap_samples).map(|_| AtomicU32::new(0)).collect(),
            in_peak_bits: AtomicU32::new(0),
            out_cb_frames: AtomicU64::new(0),
            in_cb_frames: AtomicU64::new(0),
        }
    }
}

struct OutState {
    burst_pos: Option<usize>,
}

/// Varredura linear de 1 a 8 kHz com janela de Hann, amplitude 1.
fn make_chirp(rate: u32) -> Vec<f32> {
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

struct Match {
    lag: usize,
    ncc: f32,
    amp: f32,
}

/// Correlacao cruzada normalizada do modelo `tpl` com `sig`, testando atrasos de 0 a `max_lag`.
fn best_match(sig: &[f32], tpl: &[f32], max_lag: usize) -> Option<Match> {
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
        let dot: f64 = sig[lag..lag + m]
            .iter()
            .zip(tpl)
            .map(|(&a, &b)| a as f64 * b as f64)
            .sum();
        let ncc = dot / (tpl_energy * seg_energy.max(1e-18)).sqrt();
        if ncc > best_ncc {
            best_ncc = ncc;
            best_lag = lag;
            best_dot = dot;
        }
    }
    Some(Match { lag: best_lag, ncc: best_ncc as f32, amp: (best_dot / tpl_energy) as f32 })
}

/// Roda na thread de audio de saida.
fn render_out(data: &mut [f32], channels: usize, st: &mut OutState, burst: &[f32], sh: &Shared, epoch: Instant) {
    let cb_ns = epoch.elapsed().as_nanos() as u64;
    sh.out_cb_frames.store((data.len() / channels) as u64, Relaxed);
    if st.burst_pos.is_none() && sh.fire.swap(false, Relaxed) {
        st.burst_pos = Some(0);
        sh.emit_ns.store(cb_ns, Relaxed);
        sh.capturing.store(true, Release);
    }
    for frame in data.chunks_mut(channels) {
        let s = match st.burst_pos {
            Some(p) => {
                st.burst_pos = if p + 1 < burst.len() { Some(p + 1) } else { None };
                burst[p]
            }
            None => 0.0,
        };
        for x in frame.iter_mut() {
            *x = s;
        }
    }
}

/// Roda na thread de audio de entrada: copia a entrada (mono) para o buffer de gravacao.
fn capture(data: &[f32], channels: usize, rate: u32, sh: &Shared, epoch: Instant) {
    let now = epoch.elapsed().as_nanos() as u64;
    let frames = data.len() / channels;
    sh.in_cb_frames.store(frames as u64, Relaxed);
    let peak = data.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    sh.in_peak_bits.fetch_max(peak.to_bits(), Relaxed);

    if sh.capturing.load(Acquire) {
        let len = sh.cap_len.load(Relaxed) as usize;
        if len == 0 {
            let dur_ns = (frames as f64 / rate as f64 * 1e9) as u64;
            sh.cap_t0_ns.store(now.saturating_sub(dur_ns), Relaxed);
        }
        let n = frames.min(sh.cap.len() - len);
        for (i, frame) in data.chunks(channels).take(n).enumerate() {
            let s = frame.iter().sum::<f32>() / channels as f32;
            sh.cap[len + i].store(s.to_bits(), Relaxed);
        }
        let new_len = len + n;
        sh.cap_len.store(new_len as u64, Relaxed);
        if new_len >= sh.cap.len() {
            sh.capturing.store(false, Relaxed);
            sh.done.store(true, Release);
        }
    }
}

struct Args {
    input: Option<String>,
    output: Option<String>,
    buffer: Option<u32>,
    clicks: usize,
    min_ncc: f32,
    level: f32,
    list: bool,
}

fn parse_args() -> Result<Args> {
    let mut a = Args { input: None, output: None, buffer: None, clicks: 20, min_ncc: 0.3, level: 0.5, list: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().ok_or_else(|| anyhow!("{arg} precisa de {what}"));
        match arg.as_str() {
            "--list" => a.list = true,
            "--input" => a.input = Some(next("um nome")?),
            "--output" => a.output = Some(next("um nome")?),
            "--buffer" => a.buffer = Some(next("um numero")?.parse()?),
            "--clicks" => a.clicks = next("um numero")?.parse()?,
            "--min-ncc" => a.min_ncc = next("um valor")?.parse()?,
            "--level" => a.level = next("um valor")?.parse()?,
            _ => bail!("argumento inesperado: {arg}"),
        }
    }
    Ok(a)
}

fn find_device(host: &cpal::Host, name: &Option<String>, input: bool) -> Result<Device> {
    match name {
        Some(n) => {
            let needle = n.to_lowercase();
            let mut devices: Box<dyn Iterator<Item = Device>> = if input {
                Box::new(host.input_devices()?)
            } else {
                Box::new(host.output_devices()?)
            };
            devices
                .find(|d| d.name().map(|x| x.to_lowercase().contains(&needle)).unwrap_or(false))
                .ok_or_else(|| anyhow!("dispositivo '{n}' nao encontrado (use --list)"))
        }
        None => {
            let d = if input { host.default_input_device() } else { host.default_output_device() };
            d.ok_or_else(|| anyhow!("nenhum dispositivo padrao de {}", if input { "entrada" } else { "saida" }))
        }
    }
}

fn apply_buffer(config: &mut cpal::StreamConfig, supported: &cpal::SupportedStreamConfig, req: Option<u32>) {
    if let Some(req) = req {
        let size = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => req.clamp(*min, *max),
            SupportedBufferSize::Unknown => req,
        };
        config.buffer_size = BufferSize::Fixed(size);
    }
}

fn list_devices() -> Result<()> {
    for host_id in cpal::available_hosts() {
        let host = cpal::host_from_id(host_id)?;
        println!("Host: {}", host_id.name());
        println!("  Saidas:");
        for d in host.output_devices()? {
            println!("    - {}", d.name().unwrap_or_else(|_| "?".into()));
        }
        println!("  Entradas:");
        for d in host.input_devices()? {
            println!("    - {}", d.name().unwrap_or_else(|_| "?".into()));
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.list {
        return list_devices();
    }

    let host = cpal::default_host();
    let out_dev = find_device(&host, &args.output, false)?;
    let in_dev = find_device(&host, &args.input, true)?;
    let out_sup = out_dev.default_output_config()?;
    let in_sup = in_dev.default_input_config()?;
    if out_sup.sample_format() != SampleFormat::F32 || in_sup.sample_format() != SampleFormat::F32 {
        bail!("formatos nao suportados (saida {:?}, entrada {:?}); esperado F32", out_sup.sample_format(), in_sup.sample_format());
    }
    let mut out_cfg = out_sup.config();
    let mut in_cfg = in_sup.config();
    apply_buffer(&mut out_cfg, &out_sup, args.buffer);
    apply_buffer(&mut in_cfg, &in_sup, args.buffer);
    let out_rate = out_cfg.sample_rate.0;
    let in_rate = in_cfg.sample_rate.0;
    let out_ch = out_cfg.channels as usize;
    let in_ch = in_cfg.channels as usize;

    println!("Host: {}", host.id().name());
    println!("Saida:   {} | {} Hz, {} canais", out_dev.name().unwrap_or_default(), out_rate, out_ch);
    println!("Entrada: {} | {} Hz, {} canais", in_dev.name().unwrap_or_default(), in_rate, in_ch);

    let epoch = Instant::now();
    let cap_samples = (in_rate as f64 * WINDOW_SECONDS) as usize;
    let sh = Arc::new(Shared::new(cap_samples));

    let burst: Vec<f32> = make_chirp(out_rate).iter().map(|x| x * args.level).collect();
    let template = make_chirp(in_rate);
    let mut out_state = OutState { burst_pos: None };
    let sh_out = Arc::clone(&sh);
    let out_stream = out_dev.build_output_stream(
        &out_cfg,
        move |data: &mut [f32], _| render_out(data, out_ch, &mut out_state, &burst, &sh_out, epoch),
        |e| eprintln!("erro na saida: {e}"),
        None,
    )?;
    let sh_in = Arc::clone(&sh);
    let in_stream = in_dev.build_input_stream(
        &in_cfg,
        move |data: &[f32], _| capture(data, in_ch, in_rate, &sh_in, epoch),
        |e| eprintln!("erro na entrada: {e}"),
        None,
    )?;
    in_stream.play()?;
    out_stream.play()?;

    thread::sleep(Duration::from_millis(1000));
    sh.in_peak_bits.swap(0, Relaxed);
    thread::sleep(Duration::from_millis(1000));
    let noise = f32::from_bits(sh.in_peak_bits.swap(0, Relaxed));
    println!("Ruido de fundo na entrada (pico, em silencio): {noise:.4}");
    println!(
        "Buffers de callback: saida {} amostras ({:.1} ms), entrada {} amostras ({:.1} ms)",
        sh.out_cb_frames.load(Relaxed),
        sh.out_cb_frames.load(Relaxed) as f64 / out_rate as f64 * 1000.0,
        sh.in_cb_frames.load(Relaxed),
        sh.in_cb_frames.load(Relaxed) as f64 / in_rate as f64 * 1000.0,
    );
    println!("Nivel do chirp: {:.2} | confianca minima (ncc): {:.2}\n", args.level, args.min_ncc);

    let max_lag = (in_rate as f64 * SEARCH_SECONDS) as usize;
    let mut accepted: Vec<f64> = Vec::new();
    let mut rejected = 0usize;

    for n in 1..=args.clicks {
        sh.done.store(false, Relaxed);
        sh.cap_len.store(0, Relaxed);
        sh.fire.store(true, Relaxed);
        let t0 = Instant::now();
        while !sh.done.load(Acquire) && t0.elapsed() < Duration::from_millis(3000) {
            thread::sleep(Duration::from_millis(5));
        }
        if !sh.done.load(Acquire) {
            sh.capturing.store(false, Relaxed);
            println!("  teste {n:2}: a gravacao nao terminou (a entrada parou de chamar o callback?)");
            rejected += 1;
            continue;
        }
        let rec: Vec<f32> = sh.cap.iter().map(|a| f32::from_bits(a.load(Relaxed))).collect();
        let t_start = sh.cap_t0_ns.load(Relaxed) as f64;
        let emit = sh.emit_ns.load(Relaxed) as f64;
        match best_match(&rec, &template, max_lag) {
            Some(m) if m.ncc >= args.min_ncc => {
                let ms = (t_start - emit) / 1e6 + m.lag as f64 / in_rate as f64 * 1000.0;
                println!("  teste {n:2}: {ms:6.1} ms  (ncc {:.2}, ganho do caminho {:.3})", m.ncc, m.amp);
                accepted.push(ms);
            }
            Some(m) => {
                println!("  teste {n:2}: rejeitado, sem sinal reconhecivel (ncc {:.2}, minimo {:.2})", m.ncc, args.min_ncc);
                rejected += 1;
            }
            None => {
                println!("  teste {n:2}: gravacao curta demais");
                rejected += 1;
            }
        }
        thread::sleep(Duration::from_millis(300));
    }

    drop(out_stream);
    drop(in_stream);

    println!("\n== Resumo");
    println!("  medidas aceitas: {} de {} (rejeitadas: {rejected})", accepted.len(), args.clicks);
    if accepted.is_empty() {
        println!("  nenhuma medida valida: aproxime o microfone da caixa, aumente o volume (--level) ou reduza o ruido do ambiente.");
        return Ok(());
    }
    accepted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = accepted.len();
    let median = if n % 2 == 1 { accepted[n / 2] } else { 0.5 * (accepted[n / 2 - 1] + accepted[n / 2]) };
    let mean = accepted.iter().sum::<f64>() / n as f64;
    println!(
        "  ida e volta: minimo {:.1} ms | mediana {:.1} ms | media {:.1} ms | maximo {:.1} ms",
        accepted[0], median, mean, accepted[n - 1]
    );
    println!("  (acustico: cada metro entre a caixa e o microfone soma cerca de 3 ms)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ruido pseudoaleatorio deterministico, em [-amp, amp].
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
        let rate = 8_000;
        let tpl = make_chirp(rate);
        for seed in [1u64, 7, 12345, 0xDEAD_BEEF] {
            let sig = noise(4_000, 0.05, seed | 1);
            let m = best_match(&sig, &tpl, 3_000).unwrap();
            assert!(m.ncc < 0.3, "ncc = {} (seed {seed})", m.ncc);
        }
    }

    #[test]
    fn short_recording_gives_none() {
        let tpl = make_chirp(8_000);
        assert!(best_match(&[0.0; 10], &tpl, 100).is_none());
    }

    #[test]
    fn output_emits_chirp_once_after_fire_and_starts_capture() {
        let burst = make_chirp(8_000);
        let sh = Shared::new(100);
        let mut st = OutState { burst_pos: None };
        let epoch = Instant::now();
        let mut buf = vec![1.0f32; 2 * 64];
        render_out(&mut buf, 2, &mut st, &burst, &sh, epoch);
        assert!(buf.iter().all(|&x| x == 0.0), "sem fire, a saida e silencio");
        assert!(!sh.capturing.load(Relaxed));

        sh.fire.store(true, Relaxed);
        let mut emitted = 0usize;
        for _ in 0..8 {
            render_out(&mut buf, 2, &mut st, &burst, &sh, epoch);
            emitted += buf.chunks(2).filter(|f| f[0] != 0.0).count();
        }
        assert!(sh.capturing.load(Relaxed));
        assert!(!sh.fire.load(Relaxed));
        assert!(emitted >= burst.len() - 4 && emitted <= burst.len(), "emitidos = {emitted}");
    }

    #[test]
    fn capture_records_mono_window_then_signals_done() {
        let epoch = Instant::now();
        let sh = Shared::new(150);
        // sem capturing, nada e gravado
        capture(&[0.3; 100], 1, 48_000, &sh, epoch);
        assert_eq!(sh.cap_len.load(Relaxed), 0);

        thread::sleep(Duration::from_millis(20));
        sh.capturing.store(true, Release);
        // estereo: media dos canais; 2 buffers de 100 quadros, so cabem 150
        let stereo: Vec<f32> = (0..100).flat_map(|_| [0.2f32, 0.4]).collect();
        capture(&stereo, 2, 48_000, &sh, epoch);
        assert_eq!(sh.cap_len.load(Relaxed), 100);
        assert!(!sh.done.load(Acquire));
        capture(&stereo, 2, 48_000, &sh, epoch);
        assert_eq!(sh.cap_len.load(Relaxed), 150);
        assert!(sh.done.load(Acquire));
        assert!(!sh.capturing.load(Relaxed));
        assert!((f32::from_bits(sh.cap[0].load(Relaxed)) - 0.3).abs() < 1e-6);
        // o inicio da gravacao e anterior ao instante do primeiro callback
        let t0 = sh.cap_t0_ns.load(Relaxed) as f64 / 1e6;
        assert!(t0 >= 15.0 && t0 < 200.0, "t0 = {t0}");
    }
}
