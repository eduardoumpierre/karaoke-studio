//! Spike 3b (Fase 0): latencia de ida e volta com WASAPI em tres modos
//! (compartilhado, compartilhado de baixa latencia via IAudioClient3, e exclusivo).
//!
//! A medicao e a mesma do spike 3a: um chirp (1 a 8 kHz, 50 ms) sai pela saida, 1 s da entrada
//! e gravado e o atraso e achado por correlacao cruzada normalizada. Veja SPIKE-WASAPI.md.
//!
//! A logica pura (chirp, correlacao, formatos) fica em `dsp` e roda em qualquer sistema com
//! `cargo test`; a parte WASAPI (`win`) so existe no Windows.

#![cfg_attr(not(windows), allow(dead_code))]

mod dsp;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Este spike usa WASAPI e so roda no Windows. Em outros sistemas, use `cargo test` para a logica pura.");
}
