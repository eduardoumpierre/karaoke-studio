//! Testes com arquivos de áudio de verdade (gerados com o ffmpeg; pulados se ele não existir).

use std::path::{Path, PathBuf};
use std::process::Command;

use audio_engine::{decode, load_track, resample};

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("audio-engine-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Gera um arquivo com um seno (ffmpeg lavfi).
fn make(path: &Path, freq: u32, secs: f32, rate: u32, channels: u32, extra: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency={freq}:duration={secs}:sample_rate={rate}"
        ))
        .args(["-ac", &channels.to_string()])
        .args(extra)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn decodes_flac_24bit_stereo() {
    if !ffmpeg_ok() {
        return;
    }
    let p = tmp("a.flac");
    make(&p, 440, 2.0, 44_100, 2, &["-sample_fmt", "s32"]);
    let d = decode::decode_file(&p).unwrap();
    assert_eq!(d.rate, 44_100);
    assert_eq!(d.frames(), 88_200);
    let peak = d.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    // o ffmpeg gera o seno com amplitude 0,125 e o mono->estéreo tira 3 dB: 0,125 x 0,707
    assert!((peak - 0.0884).abs() < 0.002, "pico {peak}");
    // L e R iguais (seno copiado nos dois canais)
    assert!(d
        .samples
        .chunks(2)
        .take(1000)
        .all(|f| (f[0] - f[1]).abs() < 1e-6));
}

#[test]
fn decodes_mp3_mono_to_stereo_and_wav() {
    if !ffmpeg_ok() {
        return;
    }
    let mp3 = tmp("m.mp3");
    make(&mp3, 330, 1.0, 48_000, 1, &[]);
    let d = decode::decode_file(&mp3).unwrap();
    assert_eq!(d.rate, 48_000);
    assert!(
        d.frames() >= 48_000 && d.frames() < 48_000 + 4_000,
        "frames {}",
        d.frames()
    );
    assert!(d.samples.chunks(2).all(|f| f[0] == f[1]));

    let wav = tmp("w.wav");
    make(&wav, 330, 1.0, 44_100, 2, &[]);
    let d = decode::decode_file(&wav).unwrap();
    assert_eq!((d.rate, d.frames()), (44_100, 44_100));
}

#[test]
fn rejects_garbage_and_missing_files() {
    let p = tmp("lixo.flac");
    std::fs::write(&p, b"isto nao e audio nenhum, so texto").unwrap();
    assert!(decode::decode_file(&p).is_err());
    assert!(decode::decode_file(&tmp("nao-existe.flac")).is_err());
}

#[test]
fn resampling_keeps_duration_pitch_and_phase() {
    // seno de 1 kHz a 44,1 kHz -> 48 kHz: duração igual e as amostras seguem o mesmo seno
    let from = 44_100u32;
    let to = 48_000u32;
    let frames = from as usize * 3;
    let input: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let s =
                0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / from as f64).sin() as f32;
            [s, s]
        })
        .collect();
    let out = resample::to_rate(&input, from, to);
    assert_eq!(out.len() / 2, to as usize * 3);
    let mut worst = 0.0f32;
    for i in 2000..(out.len() / 2 - 2000) {
        let expected =
            0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / to as f64).sin() as f32;
        worst = worst.max((out[i * 2] - expected).abs());
    }
    assert!(worst < 0.01, "erro máximo {worst}");
}

#[test]
fn resampling_does_not_shift_impulses() {
    // o pico do impulso tem de continuar no mesmo instante (em segundos) após a conversão
    let (from, to) = (44_100u32, 48_000u32);
    for pos in [100usize, 5000, 20_000] {
        let mut input = vec![0.0f32; 60_000];
        input[pos * 2] = 1.0;
        input[pos * 2 + 1] = 1.0;
        let out = resample::to_rate(&input, from, to);
        let (idx, _) = out
            .chunks(2)
            .enumerate()
            .max_by(|a, b| a.1[0].partial_cmp(&b.1[0]).unwrap())
            .unwrap();
        let expected = pos as f64 * to as f64 / from as f64;
        assert!(
            (idx as f64 - expected).abs() <= 1.0,
            "impulso em {pos}: pico em {idx}, esperado {expected:.1}"
        );
    }
}

#[test]
fn resampling_is_identity_at_same_rate_and_handles_tiny_input() {
    let v: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4];
    assert_eq!(resample::to_rate(&v, 48_000, 48_000), v);
    let out = resample::to_rate(&v, 44_100, 48_000);
    assert_eq!(out.len(), 4);
    assert!(resample::to_rate(&[], 44_100, 48_000).is_empty());
}

#[test]
fn both_stems_resampled_stay_aligned() {
    if !ffmpeg_ok() {
        return;
    }
    let a = tmp("v.flac");
    let b = tmp("i.flac");
    make(&a, 500, 2.0, 44_100, 2, &[]);
    make(&b, 500, 2.0, 44_100, 2, &[]);
    let va = load_track(&a, 48_000).unwrap();
    let vb = load_track(&b, 48_000).unwrap();
    assert_eq!(va.len(), vb.len());
    assert_eq!(
        va, vb,
        "mesmo processamento, mesmo resultado (sincronia garantida)"
    );
    assert_eq!(va.len() / 2, 96_000);
}
