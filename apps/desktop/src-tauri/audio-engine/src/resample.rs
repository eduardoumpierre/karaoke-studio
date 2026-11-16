//! Conversão de taxa de amostragem de alta qualidade (rubato, interpolação por sinc).
//! Feita uma vez ao carregar a música, então o callback de áudio só lê amostras prontas.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

const CHUNK: usize = 4096;

/// `input` é estéreo intercalado. Devolve estéreo intercalado na taxa `to`, sem atraso e com a
/// mesma duração (testado com impulsos: o rubato já alinha a saída com a entrada; o
/// `output_delay()` que ele informa NÃO deve ser descontado).
pub fn to_rate(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let frames = input.len() / 2;
    let ratio = to as f64 / from as f64;
    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler =
        SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, 2).expect("parâmetros do resampler");
    let delay = 0usize;
    let expected = (frames as f64 * ratio).round() as usize;

    let left: Vec<f32> = input.iter().step_by(2).copied().collect();
    let right: Vec<f32> = input.iter().skip(1).step_by(2).copied().collect();

    let mut out_l: Vec<f32> = Vec::with_capacity(expected + delay + CHUNK);
    let mut out_r: Vec<f32> = Vec::with_capacity(expected + delay + CHUNK);
    let mut pos = 0;
    while pos + resampler.input_frames_next() <= frames {
        let n = resampler.input_frames_next();
        let out = resampler
            .process(&[&left[pos..pos + n], &right[pos..pos + n]], None)
            .expect("resample");
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
        pos += n;
    }
    // Resto (menor que um bloco) + silêncio para esvaziar o filtro.
    if pos < frames {
        let out = resampler
            .process_partial(Some(&[&left[pos..], &right[pos..]]), None)
            .expect("resample (resto)");
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
    }
    while out_l.len() < expected + delay {
        let out = resampler
            .process_partial::<&[f32]>(None, None)
            .expect("resample (cauda)");
        if out[0].is_empty() {
            break;
        }
        out_l.extend_from_slice(&out[0]);
        out_r.extend_from_slice(&out[1]);
    }

    let mut result = Vec::with_capacity(expected * 2);
    for i in 0..expected {
        let j = i + delay;
        result.push(out_l.get(j).copied().unwrap_or(0.0));
        result.push(out_r.get(j).copied().unwrap_or(0.0));
    }
    result
}
