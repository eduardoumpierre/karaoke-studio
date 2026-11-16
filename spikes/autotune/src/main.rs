//! Spike do autotune (Fase 0): corrige a altura de uma gravacao de voz e mede custo e atraso.
//! Uso: spike-autotune ENTRADA.wav SAIDA.wav [--key C] [--scale chromatic|major|minor|pent-major|pent-minor]
//!        [--speed-ms 20] [--strength 1.0] [--block 128]

mod tune;

use std::time::Instant;
use tune::*;

fn read_mono(path: &str) -> Result<(Vec<f32>, u32, u16), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{path}: {e}"))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>().map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>().map_err(|e| e.to_string())?
        }
    };
    let mono = raw.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect();
    Ok((mono, spec.sample_rate, spec.channels))
}

fn usage() -> String {
    "uso: spike-autotune ENTRADA.wav SAIDA.wav [--key C] [--scale chromatic|major|minor|pent-major|pent-minor] [--speed-ms 20] [--strength 1.0] [--block 128] [--gain-db 0] [--gate 0.0005]".into()
}

fn run() -> Result<(), String> {
    let mut pos: Vec<String> = Vec::new();
    let mut cfg = Config::default();
    let mut block = 128usize;
    let mut gain_db = 0.0f32;
    let mut gate = 0.0005f32;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = |n: &str| it.next().ok_or_else(|| format!("{a} precisa de {n}"));
        match a.as_str() {
            "--key" => cfg.key = parse_key(&val("uma nota")?).ok_or("tom invalido (use C, C#, Db, ...)")?,
            "--scale" => cfg.scale = Scale::parse(&val("uma escala")?).ok_or("escala invalida")?,
            "--speed-ms" => cfg.speed_ms = val("ms")?.parse().map_err(|e| format!("{e}"))?,
            "--strength" => cfg.strength = val("0 a 1")?.parse().map_err(|e| format!("{e}"))?,
            "--gain-db" => gain_db = val("dB")?.parse().map_err(|e| format!("{e}"))?,
            "--gate" => gate = val("rms")?.parse().map_err(|e| format!("{e}"))?,
            "--block" => block = val("amostras")?.parse().map_err(|e| format!("{e}"))?,
            s if s.starts_with("--") => return Err(format!("argumento desconhecido: {s}\n{}", usage())),
            _ => pos.push(a),
        }
    }
    if pos.len() != 2 {
        return Err(usage());
    }
    let (mut x, rate, channels) = read_mono(&pos[0])?;
    if gain_db != 0.0 {
        let g = 10f32.powf(gain_db / 20.0);
        x.iter_mut().for_each(|v| *v *= g);
    }
    let in_peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let in_rms = (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    let fs = rate as f32;
    let secs = x.len() as f32 / fs;
    println!("Entrada: {:.1} s, {rate} Hz, {channels} canal(is) (processado em mono)", secs);
    println!(
        "Configuracao: tom {} | escala {:?} | velocidade {} ms | forca {} | bloco {block} amostras",
        NOTE_NAMES[cfg.key as usize], cfg.scale, cfg.speed_ms, cfg.strength
    );

    println!("Volume da entrada: pico {in_peak:.3}, RMS {in_rms:.4} (ganho aplicado {gain_db} dB)");
    let mut at = AutoTune::new(fs, &cfg);
    at.set_gate(gate);
    let mut y = vec![0.0f32; x.len()];
    let block_ms = block as f32 / fs * 1000.0;
    let (mut total, mut worst) = (0.0f64, 0.0f64);
    let mut blocks = 0u64;
    for (i, o) in x.chunks(block).zip(y.chunks_mut(block)) {
        let t0 = Instant::now();
        at.process(i, o);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        total += dt;
        worst = worst.max(dt);
        blocks += 1;
    }

    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(&pos[1], spec).map_err(|e| e.to_string())?;
    for v in &y {
        w.write_sample(*v).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())?;

    let dry = pitch_track(&x, fs, cfg.fmin, cfg.fmax);
    let wet = pitch_track(&y, fs, cfg.fmin, cfg.fmax);
    let s = &at.stats;
    println!("\n== Resultado");
    println!("  trechos com voz: {:.0}% | correcao media aplicada: {:.1} cents", 100.0 * s.voiced_hops as f64 / s.hops.max(1) as f64, s.sum_abs_corr_cents / s.voiced_hops.max(1) as f64);
    match (scale_deviation(&dry, cfg.key, cfg.scale), scale_deviation(&wet, cfg.key, cfg.scale)) {
        (Some((dm, da)), Some((wm, wa))) => {
            println!("  desvio ate a nota da escala (mediana | media):");
            println!("    antes:  {dm:5.1} | {da:5.1} cents");
            println!("    depois: {wm:5.1} | {wa:5.1} cents");
        }
        _ => println!("  nao foi detectada voz suficiente para medir o desvio"),
    }
    let (g, u, v) = at.detector_counts();
    let tot = (g + u + v).max(1) as f64;
    println!(
        "  quadros do detector: com voz {:.0}% | abaixo do volume minimo {:.0}% | sem altura clara {:.0}%",
        100.0 * v as f64 / tot,
        100.0 * g as f64 / tot,
        100.0 * u as f64 / tot
    );
    if s.d_count > 0 {
        let k = 1000.0 / fs as f64;
        println!(
            "  atraso do audio pelo autotune: medio {:.1} ms (minimo {:.1}, maximo {:.1})",
            s.d_sum / s.d_count as f64 * k,
            s.d_min * k,
            s.d_max * k
        );
    }
    println!(
        "  a deteccao de altura enxerga {:.0} ms de passado (janela) e atualiza a cada {:.0} ms",
        at.detector_window_ms(),
        at.hop_ms()
    );
    println!(
        "  CPU: {:.2}% de um nucleo (media por bloco {:.3} ms de {:.2} ms; pior bloco {:.3} ms)",
        100.0 * total / blocks as f64 / block_ms as f64,
        total / blocks as f64,
        block_ms,
        worst
    );
    println!("  pico da saida: {peak:.2}{}", if peak > 1.0 { "  (ATENCAO: passa de 1,0)" } else { "" });
    println!("\nGravado: {}", pos[1]);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("erro: {e}");
        std::process::exit(1);
    }
}
