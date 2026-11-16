//! Spike 2 (Fase 0): toca duas stems (vocal + instrumental) em um unico callback de audio.
//!
//! Objetivos do teste:
//!   - confirmar que vocal e instrumental ficam sincronizados (os dois sao lidos pela mesma
//!     posicao, no mesmo callback, entao nao ha deriva entre eles por construcao);
//!   - medir a diferenca entre o relogio do dispositivo de audio e o relogio do sistema;
//!   - medir o tempo gasto no callback (folga em relacao ao tamanho do buffer);
//!   - exercitar volume independente, pausa e seek sem estalos.
//!
//! Limitacoes assumidas neste spike (o motor real resolve depois):
//!   - reamostragem por interpolacao linear (o motor real usa um reamostrador de qualidade);
//!   - o seek faz apenas fade-in curto, sem fade-out/crossfade;
//!   - cpal usa o modo compartilhado do WASAPI; o modo exclusivo sera avaliado no spike de latencia.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, SupportedBufferSize};

const NO_SEEK: u64 = u64::MAX;
const RAMP_SECONDS: f32 = 0.005;
const VOL_COEF: f32 = 0.002;

struct Stem {
    /// Amostras intercaladas L,R (mono e duplicado para estereo).
    samples: Vec<f32>,
    frames: usize,
    rate: u32,
}

fn load_wav(path: &Path) -> Result<Stem> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("abrindo {}", path.display()))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels == 0 || channels > 2 {
        bail!("{}: {} canais nao suportado neste spike", path.display(), channels);
    }
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<f32>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample as u32 - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<f32>, _>>()?
        }
    };
    let frames = raw.len() / channels;
    let samples = if channels == 2 {
        raw
    } else {
        raw.iter().flat_map(|&s| [s, s]).collect()
    };
    Ok(Stem { samples, frames, rate: spec.sample_rate })
}

struct Shared {
    vocal_vol: AtomicU32,
    inst_vol: AtomicU32,
    master_vol: AtomicU32,
    playing: AtomicBool,
    looping: AtomicBool,
    finished: AtomicBool,
    quit: AtomicBool,
    seek_to: AtomicU64,
    seeks: AtomicU64,
    pos_frames: AtomicU64,
    out_frames: AtomicU64,
    max_cb_us: AtomicU64,
    last_cb_frames: AtomicU64,
    peak_bits: AtomicU32,
}

fn setf(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Relaxed);
}
fn getf(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}

struct CbState {
    pos: f64,
    step: f64,
    ramp: f32,
    play_gain: f32,
    cur_v: f32,
    cur_i: f32,
    cur_m: f32,
}

struct Stems {
    vocals: Stem,
    inst: Stem,
    frames: usize,
}

#[inline]
fn lerp(a: f32, b: f32, f: f32) -> f32 {
    a + (b - a) * f
}

/// Roda na thread de audio: sem alocacao, sem locks, sem I/O.
fn render(data: &mut [f32], channels: usize, st: &mut CbState, stems: &Stems, sh: &Shared) {
    let t0 = Instant::now();

    let seek = sh.seek_to.swap(NO_SEEK, Relaxed);
    if seek != NO_SEEK {
        let last = stems.frames.saturating_sub(2) as u64;
        st.pos = seek.min(last) as f64;
        st.play_gain = 0.0; // fade-in apos o seek
    }

    let mut playing = sh.playing.load(Relaxed);
    let looping = sh.looping.load(Relaxed);
    let tv = getf(&sh.vocal_vol);
    let ti = getf(&sh.inst_vol);
    let tm = getf(&sh.master_vol);
    let mut peak = 0.0f32;

    for frame in data.chunks_mut(channels) {
        st.cur_v += (tv - st.cur_v) * VOL_COEF;
        st.cur_i += (ti - st.cur_i) * VOL_COEF;
        st.cur_m += (tm - st.cur_m) * VOL_COEF;

        let target = if playing { 1.0 } else { 0.0 };
        if st.play_gain < target {
            st.play_gain = (st.play_gain + st.ramp).min(1.0);
        } else if st.play_gain > target {
            st.play_gain = (st.play_gain - st.ramp).max(0.0);
        }

        let (mut l, mut r) = (0.0f32, 0.0f32);
        if st.play_gain > 0.0 {
            let mut i = st.pos as usize;
            if i + 1 >= stems.frames {
                if looping {
                    st.pos = 0.0;
                    i = 0;
                } else {
                    sh.finished.store(true, Relaxed);
                    sh.playing.store(false, Relaxed);
                    playing = false;
                    st.play_gain = 0.0;
                }
            }
            if st.play_gain > 0.0 {
                let f = (st.pos - i as f64) as f32;
                let j = i + 1;
                let vl = lerp(stems.vocals.samples[2 * i], stems.vocals.samples[2 * j], f);
                let vr = lerp(stems.vocals.samples[2 * i + 1], stems.vocals.samples[2 * j + 1], f);
                let il = lerp(stems.inst.samples[2 * i], stems.inst.samples[2 * j], f);
                let ir = lerp(stems.inst.samples[2 * i + 1], stems.inst.samples[2 * j + 1], f);
                let g = st.cur_m * st.play_gain;
                l = (vl * st.cur_v + il * st.cur_i) * g;
                r = (vr * st.cur_v + ir * st.cur_i) * g;
                st.pos += st.step;
            }
        }

        peak = peak.max(l.abs()).max(r.abs());
        let (l, r) = (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0));
        if channels == 1 {
            frame[0] = 0.5 * (l + r);
        } else {
            frame[0] = l;
            frame[1] = r;
            for extra in frame.iter_mut().skip(2) {
                *extra = 0.0;
            }
        }
    }

    sh.pos_frames.store(st.pos as u64, Relaxed);
    sh.out_frames.fetch_add((data.len() / channels) as u64, Relaxed);
    sh.last_cb_frames.store((data.len() / channels) as u64, Relaxed);
    // para floats nao negativos, a ordem dos bits coincide com a ordem numerica
    sh.peak_bits.fetch_max(peak.to_bits(), Relaxed);
    sh.max_cb_us.fetch_max(t0.elapsed().as_micros() as u64, Relaxed);
}

struct Args {
    vocals: Option<PathBuf>,
    inst: Option<PathBuf>,
    device: Option<String>,
    buffer: Option<u32>,
    duration: Option<f64>,
    looping: bool,
    seek_stress: bool,
    list: bool,
    vocal_vol: f32,
    inst_vol: f32,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        vocals: None,
        inst: None,
        device: None,
        buffer: None,
        duration: None,
        looping: false,
        seek_stress: false,
        list: false,
        vocal_vol: 0.0,
        inst_vol: 1.0,
    };
    let mut it = std::env::args().skip(1);
    let mut positional = 0;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--list" => a.list = true,
            "--loop" => a.looping = true,
            "--seek-stress" => a.seek_stress = true,
            "--device" => a.device = Some(it.next().ok_or_else(|| anyhow!("--device precisa de um nome"))?),
            "--buffer" => a.buffer = Some(it.next().ok_or_else(|| anyhow!("--buffer precisa de um numero"))?.parse()?),
            "--duration" => a.duration = Some(it.next().ok_or_else(|| anyhow!("--duration precisa de segundos"))?.parse()?),
            "--vocal" => a.vocal_vol = it.next().ok_or_else(|| anyhow!("--vocal precisa de um valor"))?.parse()?,
            "--inst" => a.inst_vol = it.next().ok_or_else(|| anyhow!("--inst precisa de um valor"))?.parse()?,
            _ => {
                match positional {
                    0 => a.vocals = Some(PathBuf::from(arg)),
                    1 => a.inst = Some(PathBuf::from(arg)),
                    _ => bail!("argumento inesperado: {arg}"),
                }
                positional += 1;
            }
        }
    }
    Ok(a)
}

fn list_devices() -> Result<()> {
    for host_id in cpal::available_hosts() {
        let host = cpal::host_from_id(host_id)?;
        println!("Host: {}", host_id.name());
        let default_out = host.default_output_device().and_then(|d| d.name().ok());
        println!("  Saidas:");
        for d in host.output_devices()? {
            let name = d.name().unwrap_or_else(|_| "?".into());
            let mark = if Some(&name) == default_out.as_ref() { " (padrao)" } else { "" };
            match d.default_output_config() {
                Ok(c) => println!(
                    "    - {name}{mark}: {} Hz, {} canais, {:?}, buffer {:?}",
                    c.sample_rate().0,
                    c.channels(),
                    c.sample_format(),
                    c.buffer_size()
                ),
                Err(_) => println!("    - {name}{mark}"),
            }
        }
        let default_in = host.default_input_device().and_then(|d| d.name().ok());
        println!("  Entradas:");
        for d in host.input_devices()? {
            let name = d.name().unwrap_or_else(|_| "?".into());
            let mark = if Some(&name) == default_in.as_ref() { " (padrao)" } else { "" };
            println!("    - {name}{mark}");
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.list {
        return list_devices();
    }
    let (Some(vp), Some(ip)) = (args.vocals.clone(), args.inst.clone()) else {
        bail!("uso: spike-audio <vocals.wav> <no_vocals.wav> [--device NOME] [--buffer N] [--duration S] [--loop] [--seek-stress] [--vocal V] [--inst V]\n     spike-audio --list");
    };

    let vocals = load_wav(&vp)?;
    let inst = load_wav(&ip)?;
    if vocals.rate != inst.rate {
        bail!("taxas diferentes: vocal {} Hz, instrumental {} Hz", vocals.rate, inst.rate);
    }
    if vocals.frames != inst.frames {
        println!("AVISO: numero de amostras diferente (vocal {}, instrumental {}); usando o menor", vocals.frames, inst.frames);
    }
    let src_rate = vocals.rate;
    let frames = vocals.frames.min(inst.frames);
    let dur_secs = frames as f64 / src_rate as f64;
    println!("Stems: {} Hz, {} amostras ({:.3} s)", src_rate, frames, dur_secs);
    let stems = Arc::new(Stems { vocals, inst, frames });

    let host = cpal::default_host();
    let device = match &args.device {
        Some(name) => {
            let needle = name.to_lowercase();
            host.output_devices()?
                .find(|d| d.name().map(|n| n.to_lowercase().contains(&needle)).unwrap_or(false))
                .ok_or_else(|| anyhow!("dispositivo '{name}' nao encontrado (use --list)"))?
        }
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow!("nenhum dispositivo de saida padrao"))?,
    };
    let supported = device.default_output_config()?;
    if supported.sample_format() != SampleFormat::F32 {
        bail!("formato {:?} nao suportado neste spike (esperado F32)", supported.sample_format());
    }
    let mut config = supported.config();
    let dev_rate = config.sample_rate.0;
    let channels = config.channels as usize;

    if let Some(req) = args.buffer {
        let size = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => req.clamp(*min, *max),
            SupportedBufferSize::Unknown => req,
        };
        config.buffer_size = BufferSize::Fixed(size);
        println!("Buffer solicitado: {req} amostras (aplicado: {size})");
    }

    println!(
        "Host: {} | Saida: {} | {} Hz, {} canais",
        host.id().name(),
        device.name().unwrap_or_else(|_| "?".into()),
        dev_rate,
        channels
    );
    if dev_rate != src_rate {
        println!("Reamostragem linear {} Hz -> {} Hz (so neste spike)", src_rate, dev_rate);
    }

    let sh = Arc::new(Shared {
        vocal_vol: AtomicU32::new(args.vocal_vol.to_bits()),
        inst_vol: AtomicU32::new(args.inst_vol.to_bits()),
        master_vol: AtomicU32::new(1.0f32.to_bits()),
        playing: AtomicBool::new(true),
        looping: AtomicBool::new(args.looping),
        finished: AtomicBool::new(false),
        quit: AtomicBool::new(false),
        seek_to: AtomicU64::new(NO_SEEK),
        seeks: AtomicU64::new(0),
        pos_frames: AtomicU64::new(0),
        out_frames: AtomicU64::new(0),
        max_cb_us: AtomicU64::new(0),
        last_cb_frames: AtomicU64::new(0),
        peak_bits: AtomicU32::new(0),
    });

    let mut state = CbState {
        pos: 0.0,
        step: src_rate as f64 / dev_rate as f64,
        ramp: 1.0 / (RAMP_SECONDS * dev_rate as f32),
        play_gain: 0.0,
        cur_v: args.vocal_vol,
        cur_i: args.inst_vol,
        cur_m: 1.0,
    };

    let stems_cb = Arc::clone(&stems);
    let sh_cb = Arc::clone(&sh);
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| render(data, channels, &mut state, &stems_cb, &sh_cb),
        |err| eprintln!("erro no stream de audio: {err}"),
        None,
    )?;
    stream.play()?;

    println!("\nTocando. Comandos (digite e Enter):");
    println!("  v <0-2>  volume do vocal      i <0-2>  volume do instrumental   m <0-2>  volume master");
    println!("  seek <s> pula para o segundo  p  pausa/retoma                   l  liga/desliga repeticao");
    println!("  q        sai\n");

    {
        let sh = Arc::clone(&sh);
        thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                let mut parts = line.split_whitespace();
                let cmd = parts.next().unwrap_or("");
                let arg = parts.next().and_then(|a| a.parse::<f32>().ok());
                match (cmd, arg) {
                    ("v", Some(x)) => setf(&sh.vocal_vol, x.clamp(0.0, 2.0)),
                    ("i", Some(x)) => setf(&sh.inst_vol, x.clamp(0.0, 2.0)),
                    ("m", Some(x)) => setf(&sh.master_vol, x.clamp(0.0, 2.0)),
                    ("seek", Some(s)) => {
                        let target = (s.max(0.0) as f64 * src_rate as f64) as u64;
                        sh.finished.store(false, Relaxed);
                        sh.seek_to.store(target, Relaxed);
                        sh.seeks.fetch_add(1, Relaxed);
                    }
                    ("p", _) => {
                        let now = !sh.playing.load(Relaxed);
                        sh.playing.store(now, Relaxed);
                        println!("{}", if now { "retomado" } else { "pausado" });
                    }
                    ("l", _) => {
                        let now = !sh.looping.load(Relaxed);
                        sh.looping.store(now, Relaxed);
                        println!("repeticao {}", if now { "ligada" } else { "desligada" });
                    }
                    ("q", _) => {
                        sh.quit.store(true, Relaxed);
                        break;
                    }
                    _ => println!("comandos: v|i|m <valor>, seek <s>, p, l, q"),
                }
            }
        });
    }

    if args.seek_stress {
        let sh = Arc::clone(&sh);
        let max_start_ms = ((dur_secs - 5.0).max(1.0) * 1000.0) as u64;
        thread::spawn(move || {
            let mut x: u64 = 0x2545_F491_4F6C_DD1D;
            while !sh.quit.load(Relaxed) {
                thread::sleep(Duration::from_millis(300));
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let ms = x % max_start_ms;
                sh.seek_to.store(ms * src_rate as u64 / 1000, Relaxed);
                sh.seeks.fetch_add(1, Relaxed);
            }
        });
        println!("Modo seek-stress: um seek aleatorio a cada 300 ms.\n");
    }

    let start = Instant::now();
    let mut next_report = 5.0f64;
    let mut baseline: Option<(f64, u64)> = None;
    let mut worst_load = 0.0f64;
    let mut last_drift_ms = 0.0f64;

    while !sh.quit.load(Relaxed) {
        thread::sleep(Duration::from_millis(100));
        let t = start.elapsed().as_secs_f64();
        let out = sh.out_frames.load(Relaxed);
        if baseline.is_none() && t >= 2.0 {
            baseline = Some((t, out));
        }
        if let Some(d) = args.duration {
            if t >= d {
                break;
            }
        }
        if sh.finished.load(Relaxed) && !sh.looping.load(Relaxed) {
            println!("fim da musica");
            break;
        }
        if t >= next_report {
            next_report += 5.0;
            let drift = baseline
                .map(|(t0, o0)| ((out - o0) as f64 / dev_rate as f64 - (t - t0)) * 1000.0)
                .unwrap_or(0.0);
            last_drift_ms = drift;
            let buf = sh.last_cb_frames.load(Relaxed).max(1);
            let buf_ms = buf as f64 / dev_rate as f64 * 1000.0;
            let cb_ms = sh.max_cb_us.swap(0, Relaxed) as f64 / 1000.0;
            let load = cb_ms / buf_ms;
            worst_load = worst_load.max(load);
            let peak = f32::from_bits(sh.peak_bits.swap(0, Relaxed));
            let pos = sh.pos_frames.load(Relaxed) as f64 / src_rate as f64;
            println!(
                "t={:6.1}s | pos={:7.2}s | deriva do dispositivo vs relogio: {:+6.1} ms | callback max {:.3} ms de {:.1} ms ({:.0}%) | pico {:.2} | seeks {}",
                t, pos, drift, cb_ms, buf_ms, load * 100.0, peak, sh.seeks.load(Relaxed)
            );
        }
    }

    drop(stream);
    println!("\n== Resumo");
    println!("  tempo total: {:.1} s", start.elapsed().as_secs_f64());
    println!("  deriva final do dispositivo vs relogio (desde 2 s): {:+.1} ms", last_drift_ms);
    println!("  pior uso do callback: {:.0}% do tempo do buffer", worst_load * 100.0);
    println!("  seeks executados: {}", sh.seeks.load(Relaxed));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stems(frames: usize, rate: u32, vocal: f32, inst: f32) -> Stems {
        let mk = |v: f32| Stem { samples: vec![v; frames * 2], frames, rate };
        Stems { vocals: mk(vocal), inst: mk(inst), frames }
    }

    fn shared(vocal_vol: f32, inst_vol: f32, looping: bool) -> Shared {
        Shared {
            vocal_vol: AtomicU32::new(vocal_vol.to_bits()),
            inst_vol: AtomicU32::new(inst_vol.to_bits()),
            master_vol: AtomicU32::new(1.0f32.to_bits()),
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(looping),
            finished: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            seek_to: AtomicU64::new(NO_SEEK),
            seeks: AtomicU64::new(0),
            pos_frames: AtomicU64::new(0),
            out_frames: AtomicU64::new(0),
            max_cb_us: AtomicU64::new(0),
            last_cb_frames: AtomicU64::new(0),
            peak_bits: AtomicU32::new(0),
        }
    }

    fn state(src: u32, dev: u32, v: f32, i: f32) -> CbState {
        CbState {
            pos: 0.0,
            step: src as f64 / dev as f64,
            ramp: 1.0 / (RAMP_SECONDS * dev as f32),
            play_gain: 0.0,
            cur_v: v,
            cur_i: i,
            cur_m: 1.0,
        }
    }

    #[test]
    fn mixes_both_stems_with_independent_volumes() {
        let s = stems(44_100, 44_100, 0.5, 0.25);
        // karaoke tradicional: vocal 0, instrumental 1
        let sh = shared(0.0, 1.0, false);
        let mut st = state(44_100, 44_100, 0.0, 1.0);
        let mut buf = vec![0.0f32; 2 * 4_410];
        render(&mut buf, 2, &mut st, &s, &sh);
        assert!((buf[buf.len() - 2] - 0.25).abs() < 1e-4);
        // cantar junto: vocal 1, instrumental 1
        let sh = shared(1.0, 1.0, false);
        let mut st = state(44_100, 44_100, 1.0, 1.0);
        render(&mut buf, 2, &mut st, &s, &sh);
        assert!((buf[buf.len() - 1] - 0.75).abs() < 1e-4);
    }

    #[test]
    fn position_tracks_source_time_when_resampling() {
        // 1 s de saida a 48 kHz deve avancar ~1 s (44100 amostras) na fonte, em varios buffers
        let s = stems(10 * 44_100, 44_100, 0.1, 0.1);
        let sh = shared(0.0, 1.0, false);
        let mut st = state(44_100, 48_000, 0.0, 1.0);
        let mut buf = vec![0.0f32; 2 * 480];
        for _ in 0..100 {
            render(&mut buf, 2, &mut st, &s, &sh);
        }
        let pos = sh.pos_frames.load(Relaxed) as i64;
        assert!((pos - 44_100).abs() <= 1, "pos = {pos}");
        assert_eq!(sh.out_frames.load(Relaxed), 48_000);
    }

    #[test]
    fn seek_jumps_and_fades_in() {
        let s = stems(10 * 44_100, 44_100, 0.0, 0.5);
        let sh = shared(0.0, 1.0, false);
        let mut st = state(44_100, 44_100, 0.0, 1.0);
        let mut buf = vec![0.0f32; 2 * 4_410];
        render(&mut buf, 2, &mut st, &s, &sh);
        sh.seek_to.store(5 * 44_100, Relaxed);
        render(&mut buf, 2, &mut st, &s, &sh);
        assert!(buf[0].abs() < 0.01, "o primeiro quadro apos o seek deve ser quase mudo");
        assert!((buf[buf.len() - 2] - 0.5).abs() < 1e-4, "o fim do buffer ja esta no ganho cheio");
        let pos = sh.pos_frames.load(Relaxed) as i64;
        assert!((pos - (5 * 44_100 + 4_410)).abs() <= 1, "pos = {pos}");
    }

    #[test]
    fn stops_at_end_and_loops_when_asked() {
        let s = stems(10_000, 44_100, 0.0, 0.5);
        let sh = shared(0.0, 1.0, false);
        let mut st = state(44_100, 44_100, 0.0, 1.0);
        let mut buf = vec![0.0f32; 2 * 20_000];
        render(&mut buf, 2, &mut st, &s, &sh);
        assert!(sh.finished.load(Relaxed));
        assert!(buf[buf.len() - 2..].iter().all(|&x| x == 0.0));

        let sh = shared(0.0, 1.0, true);
        let mut st = state(44_100, 44_100, 0.0, 1.0);
        render(&mut buf, 2, &mut st, &s, &sh);
        assert!(!sh.finished.load(Relaxed));
        assert!((buf[buf.len() - 2] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn loads_wav_float_and_pcm16() {
        let dir = std::env::temp_dir().join("spike_audio_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p32 = dir.join("f32.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&p32, spec).unwrap();
        for _ in 0..100 {
            w.write_sample(0.5f32).unwrap();
            w.write_sample(-0.5f32).unwrap();
        }
        w.finalize().unwrap();
        let s = load_wav(&p32).unwrap();
        assert_eq!((s.frames, s.rate), (100, 44_100));
        assert!((s.samples[0] - 0.5).abs() < 1e-6 && (s.samples[1] + 0.5).abs() < 1e-6);

        let p16 = dir.join("i16.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&p16, spec).unwrap();
        for _ in 0..50 {
            w.write_sample(16_384i16).unwrap();
        }
        w.finalize().unwrap();
        let s = load_wav(&p16).unwrap();
        assert_eq!((s.frames, s.rate, s.samples.len()), (50, 48_000, 100));
        assert!((s.samples[0] - 0.5).abs() < 1e-4);
    }
}
