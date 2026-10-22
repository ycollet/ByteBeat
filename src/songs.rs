use serde::{Deserialize, Serialize};

use crate::engine::Mode;

#[derive(Clone, Serialize, Deserialize)]
pub struct Song {
    pub title: String,
    pub author: String,
    pub code: String,
    pub rate: f64,
    pub mode: Mode,
}

pub fn demos() -> Vec<Song> {
    [
        ("Copper staircase", "((t * ((t >> 12 & 3) + 2)) & 127) + ((t >> 4) & 63)", Mode::Bytebeat, 8_000),
        ("Parallel circuits", "[t * ((t >> 13 & 7) + 1) & 255, t * ((t >> 12 & 3) + 3) & 255]", Mode::Bytebeat, 8_000),
        ("Glass orbit", "0.35 * sin(2 * PI * 220 * t / sampleRate + 2 * sin(2 * PI * 0.7 * t / sampleRate))", Mode::Floatbeat, 44_100),
        ("Binary rain", "((t * 5 & t >> 7) | (t * 3 & t >> 10)) ^ (t >> 5)", Mode::Bytebeat, 8_000),
        ("Signed steps", "(t * ((t >> 11 & 3) + 1) & 255) - 128", Mode::Signed, 11_025),
        ("Soft satellites", "return t => [0.3 * sin(2 * PI * 165 * t) * sin(PI * t / 3), 0.3 * sin(2 * PI * 220 * t) * sin(PI * t / 4)];", Mode::Funcbeat, 44_100),
        ("Pulse garden", "((t * ((t >> 10 & 7) + 1) & 128) >> 1) + (t * 3 & 63)", Mode::Bytebeat, 8_000),
        ("Low tide", "0.3 * sin(2 * PI * (110 + 27.5 * (t >> 15 & 3)) * t / sampleRate) * exp(-4 * (t % 16000) / 16000)", Mode::Floatbeat, 32_000),
    ]
    .into_iter()
    .map(|(title, code, mode, rate)| Song {
        title: title.into(),
        author: "Built-in demo".into(),
        code: code.into(),
        rate: f64::from(rate),
        mode,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_demo_renders() {
        for song in super::demos() {
            let mut engine = crate::engine::Engine::new(&song.code, song.mode, song.rate)
                .unwrap_or_else(|e| panic!("{}: {e}", song.title));
            let frames = engine
                .render(2048, 48_000)
                .unwrap_or_else(|e| panic!("{}: {e}", song.title));
            assert_eq!(frames.len(), 2048);
            assert!(
                frames
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() <= 1.0)
            );
        }
    }
}
