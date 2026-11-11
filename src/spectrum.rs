//! The audio spectrum scenes listen to: what's playing, as 64 bands a side from low to high,
//! each about 0 to 1. How Wallpaper Engine scales its bands isn't documented; these are spread
//! evenly in pitch from 30 Hz to 16 kHz, on a 60 dB scale, and fall back smoothly.

pub const BANDS: usize = 64;
/// Samples a second, a side.
pub const RATE: u32 = 48_000;
/// Samples each analysis looks at: about 43 ms.
const SIZE: usize = 2048;
const LOW: f32 = 30.0;
const HIGH: f32 = 16_000.0;
/// The quietest level above silence, in decibels below full scale.
const FLOOR: f32 = 60.0;
/// How fast a band falls back once its sound stops, a second.
const FALL: f32 = 8.0;

pub type Bands = [[f32; BANDS]; 2];

pub struct Analyzer {
    /// The last `SIZE` samples of each side, oldest at `at`.
    window: [Vec<f32>; 2],
    at: usize,
    hann: Vec<f32>,
    /// Where each band's frequencies start, and where the last ends.
    edges: [usize; BANDS + 1],
    level: Bands,
}

impl Analyzer {
    pub fn new() -> Self {
        let hann = (0..SIZE)
            .map(|i| {
                let turn = std::f32::consts::TAU * i as f32 / SIZE as f32;
                0.5 - 0.5 * turn.cos()
            })
            .collect();
        let bin = |hz: f32| (hz * SIZE as f32 / RATE as f32).round() as usize;
        let mut edges = [0; BANDS + 1];
        for (band, edge) in edges.iter_mut().enumerate() {
            let hz = LOW * (HIGH / LOW).powf(band as f32 / BANDS as f32);
            *edge = bin(hz).max(1);
        }
        // Low bands narrower than a frequency step each get one of their own.
        for band in 1..=BANDS {
            edges[band] = edges[band].max(edges[band - 1] + 1);
        }
        Analyzer {
            window: [vec![0.0; SIZE], vec![0.0; SIZE]],
            at: 0,
            hann,
            edges,
            level: [[0.0; BANDS]; 2],
        }
    }

    /// Takes samples, left and right in turn.
    pub fn push(&mut self, samples: &[f32]) {
        for frame in samples.as_chunks::<2>().0 {
            for (window, &sample) in self.window.iter_mut().zip(frame) {
                window[self.at] = if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
            self.at = (self.at + 1) % SIZE;
        }
    }

    /// The bands now, `seconds` after they were last asked for: they rise at once and fall
    /// back smoothly.
    pub fn bands(&mut self, seconds: f32) -> Bands {
        let keep = (-FALL * seconds.max(0.0)).exp();
        for side in 0..2 {
            let mut re: Vec<f32> = (0..SIZE)
                .map(|i| self.window[side][(self.at + i) % SIZE] * self.hann[i])
                .collect();
            let mut im = vec![0.0; SIZE];
            fft(&mut re, &mut im);
            for band in 0..BANDS {
                let peak = (self.edges[band]..self.edges[band + 1].min(SIZE / 2))
                    .map(|k| re[k].hypot(im[k]))
                    .fold(0.0, f32::max);
                // A full-scale sine reads 1: the window halves it, and half its power is in the
                // mirrored half.
                let amplitude = peak * 4.0 / SIZE as f32;
                let loud = (1.0 + 20.0 * amplitude.max(1e-9).log10() / FLOOR).clamp(0.0, 1.0);
                let level = &mut self.level[side][band];
                *level = if loud >= *level {
                    loud
                } else {
                    loud + (*level - loud) * keep
                };
            }
        }
        self.level
    }
}

/// Fourier transform in place, of a length that is a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let turn = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (sin, cos) = (turn * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cos - im[b] * sin, re[b] * sin + im[b] * cos);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amplitude: f32, left_only: bool) -> Vec<f32> {
        (0..SIZE)
            .flat_map(|i| {
                let sample =
                    amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
                [sample, if left_only { 0.0 } else { sample }]
            })
            .collect()
    }

    fn loudest(bands: &[f32; BANDS]) -> usize {
        (0..BANDS)
            .max_by(|&a, &b| bands[a].total_cmp(&bands[b]))
            .unwrap()
    }

    #[test]
    fn silence_is_zero_and_a_tone_lights_its_band_on_its_side() {
        let mut analyzer = Analyzer::new();
        assert_eq!(analyzer.bands(0.0), [[0.0; BANDS]; 2]);
        analyzer.push(&tone(1000.0, 1.0, true));
        let [left, right] = analyzer.bands(0.0);
        assert!(left[loudest(&left)] > 0.95, "{left:?}");
        assert!(right.iter().all(|&band| band == 0.0));
        // Higher sounds land in higher bands.
        let mut high = Analyzer::new();
        high.push(&tone(8000.0, 1.0, false));
        assert!(loudest(&high.bands(0.0)[0]) > loudest(&left) + 10);
    }

    #[test]
    fn quieter_reads_lower_and_a_band_falls_back_smoothly() {
        let mut loud = Analyzer::new();
        loud.push(&tone(440.0, 1.0, false));
        let mut quiet = Analyzer::new();
        quiet.push(&tone(440.0, 0.03, false));
        let (loud, quiet) = (loud.bands(0.0)[0], quiet.bands(0.0)[0]);
        let band = loudest(&loud);
        // 30 dB down on a 60 dB scale.
        assert!(
            (loud[band] - quiet[band] - 0.5).abs() < 0.05,
            "{} {}",
            loud[band],
            quiet[band]
        );
        let mut fading = Analyzer::new();
        fading.push(&tone(440.0, 1.0, false));
        let was = fading.bands(0.0)[0][band];
        fading.push(&vec![0.0; 2 * SIZE]);
        let soon = fading.bands(0.05)[0][band];
        assert!(soon < was && soon > 0.5 * was, "{was} {soon}");
    }

    #[test]
    fn every_band_has_frequencies_of_its_own() {
        let analyzer = Analyzer::new();
        assert!(analyzer.edges.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(analyzer.edges[BANDS] <= SIZE / 2);
    }
}
