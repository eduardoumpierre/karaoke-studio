//! Parte Windows do spike 3b: abre saida e entrada em WASAPI (compartilhado, IAudioClient3 ou
//! exclusivo), dirigidas por eventos, e mede a latencia de ida e volta com o chirp.

use std::sync::atomic::Ordering::{Acquire, Relaxed};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use wasapi::*;

use crate::dsp::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Shared,
    LowLat,
    Exclusive,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Shared => "compartilhado (periodo padrao do motor de audio)",
            Mode::LowLat => "compartilhado de baixa latencia (IAudioClient3)",
            Mode::Exclusive => "exclusivo",
        }
    }
}

#[derive(Clone)]
struct Args {
    mode: Mode,
    input: Option<String>,
    output: Option<String>,
    period_ms: Option<f64>,
    clicks: usize,
    level: f32,
    min_ncc: f32,
    list: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        mode: Mode::Exclusive,
        input: None,
        output: None,
        period_ms: None,
        clicks: 20,
        level: 0.5,
        min_ncc: 0.3,
        list: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = |what: &str| it.next().ok_or_else(|| format!("{arg} precisa de {what}"));
        match arg.as_str() {
            "--list" => a.list = true,
            "--mode" => {
                a.mode = match next("shared, lowlat ou exclusive")?.as_str() {
                    "shared" => Mode::Shared,
                    "lowlat" => Mode::LowLat,
                    "exclusive" => Mode::Exclusive,
                    other => return Err(format!("modo desconhecido: {other} (use shared, lowlat ou exclusive)")),
                }
            }
            "--input" => a.input = Some(next("um nome")?),
            "--output" => a.output = Some(next("um nome")?),
            "--period-ms" => a.period_ms = Some(next("milissegundos")?.parse::<f64>().map_err(|e| e.to_string())?),
            "--clicks" => a.clicks = next("um numero")?.parse::<usize>().map_err(|e| e.to_string())?,
            "--level" => a.level = next("um valor")?.parse::<f32>().map_err(|e| e.to_string())?,
            "--min-ncc" => a.min_ncc = next("um valor")?.parse::<f32>().map_err(|e| e.to_string())?,
            _ => return Err(format!("argumento inesperado: {arg}")),
        }
    }
    Ok(a)
}

/// Mensagem de erro com dica para os codigos HRESULT mais comuns do modo exclusivo.
fn describe(e: &WasapiError) -> String {
    if let WasapiError::Windows(w) = e {
        let code = w.code().0 as u32;
        let hint = match code {
            0x8889_000A | 0x8889_000E => {
                " -> exclusivo negado ou dispositivo em uso: feche outros apps de audio e, nas Propriedades do dispositivo (aba Avancado), marque 'Permitir que aplicativos assumam o controle exclusivo deste dispositivo'"
            }
            0x8889_0008 => " -> formato nao suportado pelo dispositivo",
            0x8889_0019 => " -> buffer nao alinhado",
            0x8889_0016 => " -> tamanho de buffer invalido",
            _ => "",
        };
        return format!("{e} (HRESULT {code:#010x}){hint}");
    }
    format!("{e}")
}

fn pick_device(en: &DeviceEnumerator, dir: Direction, name: &Option<String>) -> Result<Device, String> {
    match name {
        None => en.get_default_device(dir).map_err(|e| describe(&e)),
        Some(n) => {
            let needle = n.to_lowercase();
            let coll = en.get_device_collection(dir).map_err(|e| describe(&e))?;
            let count = coll.get_nbr_devices().map_err(|e| describe(&e))?;
            for i in 0..count {
                let d = coll.get_device_at_index(i).map_err(|e| describe(&e))?;
                if let Ok(fname) = d.get_friendlyname() {
                    if fname.to_lowercase().contains(&needle) {
                        return Ok(d);
                    }
                }
            }
            Err(format!("dispositivo '{n}' nao encontrado (use --list)"))
        }
    }
}

fn sample_fmt_of(f: &WaveFormat) -> Result<SampleFmt, String> {
    let bits = f.get_bitspersample();
    let st = f.get_subformat().map_err(|e| describe(&e))?;
    match (bits, st) {
        (16, SampleType::Int) => Ok(SampleFmt::I16),
        (24, SampleType::Int) => Ok(SampleFmt::I24),
        (32, SampleType::Int) => Ok(SampleFmt::I32),
        (32, SampleType::Float) => Ok(SampleFmt::F32),
        (b, t) => Err(format!("formato de amostra nao suportado neste spike: {b} bits, {t:?}")),
    }
}

struct Opened {
    client: AudioClient,
    fmt: WaveFormat,
    desc: String,
}

/// Inicializa o cliente no modo de exclusivo; se o driver pedir buffer alinhado, repete uma vez
/// com o tamanho que ele indicou (procedimento da documentacao do IAudioClient::Initialize).
fn init_exclusive(
    device: &Device,
    dir: Direction,
    mut client: AudioClient,
    fmt: &WaveFormat,
    period: i64,
) -> Result<AudioClient, String> {
    match client.initialize_client(fmt, dir, StreamMode::EventsExclusive { period_hns: period }, None) {
        Ok(()) => Ok(client),
        Err(WasapiError::Windows(werr)) if werr.code().0 as u32 == 0x8889_0019 => {
            let frames = client.get_buffer_size().map_err(|e| describe(&e))?;
            let aligned = calculate_period_100ns(frames as i64, fmt.get_samplespersec() as i64);
            let mut second = device.get_iaudioclient().map_err(|e| describe(&e))?;
            second
                .initialize_client(fmt, dir, StreamMode::EventsExclusive { period_hns: aligned }, None)
                .map_err(|e| describe(&e))?;
            Ok(second)
        }
        Err(e) => Err(describe(&e)),
    }
}

fn init_stream(device: &Device, dir: Direction, mode: Mode, period_ms: Option<f64>) -> Result<Opened, String> {
    let mut client = device.get_iaudioclient().map_err(|e| describe(&e))?;
    match mode {
        Mode::Shared => {
            let fmt = WaveFormat::new(32, 32, SampleType::Float, 48_000, 2, None);
            let (def, _min) = client.get_device_period().map_err(|e| describe(&e))?;
            let sm = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: def };
            client.initialize_client(&fmt, dir, sm, None).map_err(|e| describe(&e))?;
            let desc = format!("periodo padrao do motor: {:.2} ms", def as f64 / 1e4);
            Ok(Opened { client, fmt, desc })
        }
        Mode::LowLat => {
            let fmt = client.get_mixformat().map_err(|e| describe(&e))?;
            let p = client.get_shared_mode_engine_period(&fmt).map_err(|e| describe(&e))?;
            let rate = fmt.get_samplespersec() as f64;
            client
                .initialize_shared_audio_stream(&fmt, TimingMode::Events, p.min)
                .map_err(|e| describe(&e))?;
            let desc = format!(
                "IAudioClient3: periodo pedido = minimo {} amostras ({:.2} ms); padrao {}, fundamental {}, maximo {}",
                p.min,
                p.min as f64 / rate * 1000.0,
                p.default,
                p.fundamental,
                p.max
            );
            Ok(Opened { client, fmt, desc })
        }
        Mode::Exclusive => {
            let mut chosen: Option<WaveFormat> = None;
            'search: for rate in [48_000usize, 44_100] {
                for ch in [2usize, 1] {
                    for (store, valid, st) in [
                        (24usize, 24usize, SampleType::Int),
                        (32, 24, SampleType::Int),
                        (16, 16, SampleType::Int),
                        (32, 32, SampleType::Float),
                    ] {
                        let f = WaveFormat::new(store, valid, st, rate, ch, None);
                        if let Ok(ok) = client.is_supported_exclusive_with_quirks(&f) {
                            chosen = Some(ok);
                            break 'search;
                        }
                    }
                }
            }
            let fmt = chosen.ok_or_else(|| {
                "nenhum formato aceito em modo exclusivo (testados: 24/32/16 bits, 48 e 44,1 kHz, 1 e 2 canais)".to_string()
            })?;
            let (def, min) = client.get_device_period().map_err(|e| describe(&e))?;
            let desired = period_ms.map(|ms| (ms * 10_000.0) as i64).unwrap_or(min);
            let period = client
                .calculate_aligned_period_near(desired, Some(128), &fmt)
                .map_err(|e| describe(&e))?;
            let client = init_exclusive(device, dir, client, &fmt, period)?;
            let desc = format!(
                "periodo pedido {:.2} ms; minimo do dispositivo {:.2} ms; padrao {:.2} ms",
                period as f64 / 1e4,
                min as f64 / 1e4,
                def as f64 / 1e4
            );
            Ok(Opened { client, fmt, desc })
        }
    }
}

struct StreamInfo {
    rate: u32,
    text: String,
}

fn stream_text(device_name: &str, o: &Opened, sfmt: SampleFmt, buffer_frames: u32) -> String {
    let rate = o.fmt.get_samplespersec();
    let latency = match o.client.get_stream_latency() {
        Ok(v) => format!("{:.2} ms", v as f64 / 1e4),
        Err(_) => "indisponivel".to_string(),
    };
    format!(
        "{device_name}\n      formato: {}, {} Hz, {} canais\n      {}\n      buffer efetivo: {} amostras = {:.2} ms | latencia reportada pelo driver: {}",
        sfmt.label(),
        rate,
        o.fmt.get_nchannels(),
        o.desc,
        buffer_frames,
        buffer_frames as f64 / rate as f64 * 1000.0,
        latency
    )
}

fn run_render(args: &Args, sh: &Shared, epoch: Instant, tx: &mpsc::Sender<Result<StreamInfo, String>>) -> Result<(), String> {
    let en = DeviceEnumerator::new().map_err(|e| describe(&e))?;
    let device = pick_device(&en, Direction::Render, &args.output)?;
    let name = device.get_friendlyname().unwrap_or_default();
    let opened = init_stream(&device, Direction::Render, args.mode, args.period_ms)?;
    let sfmt = sample_fmt_of(&opened.fmt)?;
    let channels = opened.fmt.get_nchannels() as usize;
    let rate = opened.fmt.get_samplespersec();
    let client = &opened.client;

    let h_event = client.set_get_eventhandle().map_err(|e| describe(&e))?;
    let render = client.get_audiorenderclient().map_err(|e| describe(&e))?;
    let buffer_frames = client.get_buffer_size().map_err(|e| describe(&e))?;

    let burst: Vec<f32> = make_chirp(rate).iter().map(|x| x * args.level).collect();
    let mut st = OutState { burst_pos: None };
    let mut mono = vec![0.0f32; buffer_frames as usize];
    let mut bytes: Vec<u8> = Vec::with_capacity(buffer_frames as usize * sfmt.bytes() * channels);

    // enche o buffer com silencio antes de iniciar, como na documentacao de renderizacao
    let avail = client.get_available_space_in_frames().map_err(|e| describe(&e))? as usize;
    mono.resize(mono.len().max(avail), 0.0);
    mono[..avail].fill(0.0);
    encode_frames(&mono[..avail], sfmt, channels, &mut bytes);
    render.write_to_device(&bytes, None).map_err(|e| describe(&e))?;

    client.start_stream().map_err(|e| describe(&e))?;
    let _ = tx.send(Ok(StreamInfo { rate, text: stream_text(&name, &opened, sfmt, buffer_frames) }));

    while !sh.quit.load(Relaxed) {
        if h_event.wait_for_event(1000).is_err() {
            if sh.quit.load(Relaxed) {
                break;
            }
            return Err("a saida parou de gerar eventos (timeout de 1 s)".to_string());
        }
        let cb_ns = epoch.elapsed().as_nanos() as u64;
        let avail = client.get_available_space_in_frames().map_err(|e| describe(&e))? as usize;
        if avail == 0 {
            continue;
        }
        if mono.len() < avail {
            mono.resize(avail, 0.0);
        }
        fill_output(&mut mono[..avail], &mut st, &burst, sh, cb_ns);
        encode_frames(&mono[..avail], sfmt, channels, &mut bytes);
        render.write_to_device(&bytes, None).map_err(|e| describe(&e))?;
    }
    let _ = client.stop_stream();
    Ok(())
}

fn run_capture(args: &Args, sh: &Shared, epoch: Instant, tx: &mpsc::Sender<Result<StreamInfo, String>>) -> Result<(), String> {
    let en = DeviceEnumerator::new().map_err(|e| describe(&e))?;
    let device = pick_device(&en, Direction::Capture, &args.input)?;
    let name = device.get_friendlyname().unwrap_or_default();
    let opened = init_stream(&device, Direction::Capture, args.mode, args.period_ms)?;
    let sfmt = sample_fmt_of(&opened.fmt)?;
    let channels = opened.fmt.get_nchannels() as usize;
    let rate = opened.fmt.get_samplespersec();
    let blockalign = opened.fmt.get_blockalign() as usize;
    let client = &opened.client;

    let h_event = client.set_get_eventhandle().map_err(|e| describe(&e))?;
    let capture = client.get_audiocaptureclient().map_err(|e| describe(&e))?;
    let buffer_frames = client.get_buffer_size().map_err(|e| describe(&e))?;
    let mut buf = vec![0u8; buffer_frames as usize * blockalign];
    let mut mono: Vec<f32> = Vec::with_capacity(buffer_frames as usize);

    client.start_stream().map_err(|e| describe(&e))?;
    let _ = tx.send(Ok(StreamInfo { rate, text: stream_text(&name, &opened, sfmt, buffer_frames) }));

    while !sh.quit.load(Relaxed) {
        if h_event.wait_for_event(1000).is_err() {
            if sh.quit.load(Relaxed) {
                break;
            }
            return Err("a entrada parou de gerar eventos (timeout de 1 s)".to_string());
        }
        let now = epoch.elapsed().as_nanos() as u64;
        loop {
            let (frames, _info) = capture.read_from_device(&mut buf).map_err(|e| describe(&e))?;
            if frames == 0 {
                break;
            }
            decode_mono(&buf[..frames as usize * blockalign], sfmt, channels, &mut mono);
            capture_mono(&mono, rate, now, sh);
            if args.mode == Mode::Exclusive {
                break; // em exclusivo ha um bloco por evento
            }
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

fn spawn_stream(
    dir: Direction,
    args: Args,
    sh: Arc<Shared>,
    epoch: Instant,
    tx: mpsc::Sender<Result<StreamInfo, String>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let _ = initialize_mta();
        let res = match dir {
            Direction::Render => run_render(&args, &sh, epoch, &tx),
            Direction::Capture => run_capture(&args, &sh, epoch, &tx),
        };
        if let Err(e) = res {
            eprintln!("[{dir:?}] erro: {e}");
            let _ = tx.send(Err(e));
        }
    })
}

fn wait_info(label: &str, rx: &mpsc::Receiver<Result<StreamInfo, String>>) -> Result<StreamInfo, String> {
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(r) => r.map_err(|e| format!("{label}: {e}")),
        Err(_) => Err(format!("{label}: a abertura nao respondeu em 15 s")),
    }
}

fn list_devices() {
    let en = match DeviceEnumerator::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("erro: {}", describe(&e));
            return;
        }
    };
    for (dir, label) in [(Direction::Render, "Saidas"), (Direction::Capture, "Entradas")] {
        println!("{label}:");
        if let Ok(coll) = en.get_device_collection(dir) {
            let n = coll.get_nbr_devices().unwrap_or(0);
            for i in 0..n {
                if let Ok(d) = coll.get_device_at_index(i) {
                    println!("  - {}", d.get_friendlyname().unwrap_or_else(|_| "?".into()));
                }
            }
        }
    }
}

pub fn run() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("erro: {e}");
            return;
        }
    };
    let _ = initialize_mta();
    if args.list {
        list_devices();
        return;
    }

    let epoch = Instant::now();
    let sh = Arc::new(Shared::new(192_000));
    let (tx_o, rx_o) = mpsc::channel();
    let (tx_i, rx_i) = mpsc::channel();
    let h_out = spawn_stream(Direction::Render, args.clone(), Arc::clone(&sh), epoch, tx_o);
    let h_in = spawn_stream(Direction::Capture, args.clone(), Arc::clone(&sh), epoch, tx_i);

    let (out_res, in_res) = (wait_info("saida", &rx_o), wait_info("entrada", &rx_i));
    let (out_info, in_info) = match (out_res, in_res) {
        (Ok(o), Ok(i)) => (o, i),
        (o, i) => {
            for msg in [o.err(), i.err()].into_iter().flatten() {
                eprintln!("ERRO: {msg}");
            }
            sh.quit.store(true, Relaxed);
            let _ = h_out.join();
            let _ = h_in.join();
            return;
        }
    };
    sh.cap_target.store((in_info.rate as f64 * WINDOW_SECONDS) as u64, Relaxed);

    println!("Modo: {}", args.mode.label());
    println!("Saida:   {}", out_info.text);
    println!("Entrada: {}", in_info.text);

    thread::sleep(Duration::from_millis(1000));
    sh.in_peak_bits.swap(0, Relaxed);
    thread::sleep(Duration::from_millis(1000));
    let noise = f32::from_bits(sh.in_peak_bits.swap(0, Relaxed));
    println!("\nRuido de fundo na entrada (pico, em silencio): {noise:.4}");
    println!("Nivel do chirp: {:.2} | confianca minima (ncc): {:.2}\n", args.level, args.min_ncc);

    let template = make_chirp(in_info.rate);
    let max_lag = (in_info.rate as f64 * SEARCH_SECONDS) as usize;
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
            println!("  teste {n:2}: a gravacao nao terminou (a entrada parou de entregar dados?)");
            rejected += 1;
            continue;
        }
        let target = (sh.cap_target.load(Relaxed) as usize).min(sh.cap.len());
        let rec: Vec<f32> = sh.cap[..target].iter().map(|a| f32::from_bits(a.load(Relaxed))).collect();
        let t_start = sh.cap_t0_ns.load(Relaxed) as f64;
        let emit = sh.emit_ns.load(Relaxed) as f64;
        match best_match(&rec, &template, max_lag) {
            Some(m) if m.ncc >= args.min_ncc => {
                let ms = (t_start - emit) / 1e6 + m.lag as f64 / in_info.rate as f64 * 1000.0;
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

    sh.quit.store(true, Relaxed);
    let _ = h_out.join();
    let _ = h_in.join();

    println!("\n== Resumo ({})", args.mode.label());
    println!("  medidas aceitas: {} de {} (rejeitadas: {rejected})", accepted.len(), args.clicks);
    match summarize(&mut accepted) {
        Some((min, median, mean, max)) => {
            println!("  ida e volta: minimo {min:.1} ms | mediana {median:.1} ms | media {mean:.1} ms | maximo {max:.1} ms");
            println!("  (acustico: cada metro entre a caixa e o microfone soma cerca de 3 ms)");
        }
        None => println!("  nenhuma medida valida: aproxime o microfone, aumente --level ou reduza o ruido do ambiente."),
    }
}
