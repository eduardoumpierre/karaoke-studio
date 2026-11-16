// Evita a janela de console extra no Windows em builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    karaoke_studio_lib::run();
}
