//! `CHudGeiger` (`0x100cfac4`, the third element in TS's `CHud` list): the radiation counter's
//! clicks, `player/geiger1..3.wav` at a rate and volume set by the range the server sends
//! (`Geiger`, the byte times 4). It draws nothing, but runs every frame once a `Geiger` message
//! has turned it on, and each run takes three to five numbers from the CRT `rand` it shares with
//! the HudText flicker.

use super::message::CrtRand;

/// `CHudGeiger::Draw` (`0x1001f4e0`): the sound to play this frame, if any, as (sample, volume).
///
/// Out of 1..=999, nothing (and no `rand`). Otherwise the range picks a chance out of 128, a
/// volume scale and how many `rand` bits pick the sample; then `rand` gives the volume
/// (`(r & 127) * scale / 255 + 0.25`), a click happens when either of two more `rand`s is under the
/// chance, and one or two more choose `geiger1`..`geiger3`. Past 800 the chance is 0, so the
/// scale and bit count (left uninitialised by TS) are never used.
pub fn draw(range: i32, rand: &mut CrtRand) -> Option<(String, f32)> {
    if !(1..=999).contains(&range) {
        return None;
    }
    let (pct, scale, bits): (i32, f32, i32) = match range {
        801.. => (0, 0.0, 0),
        601..=800 => (2, 0.4, 2),
        501..=600 => (4, 0.5, 2),
        401..=500 => (8, 0.6, 3),
        301..=400 => (8, 0.7, 3),
        201..=300 => (28, 0.78, 3),
        151..=200 => (40, 0.8, 3),
        101..=150 => (60, 0.85, 3),
        76..=100 => (80, 0.9, 3),
        51..=75 => (90, 0.95, 2),
        _ => (95, 1.0, 2),
    };
    // x87: the int times the float scale times the float 1/255 (`0x1007cd70`), plus the double
    // 0.25, stored as a float.
    let vol = ((rand.next() & 0x7f) as f64 * scale as f64 * 0.003_921_569_f32 as f64 + 0.25) as f32;
    if rand.next() & 0x7f >= pct && rand.next() & 0x7f >= pct {
        return None;
    }
    let mut j = rand.next() & 1;
    if bits > 2 {
        j += rand.next() & 1;
    }
    Some((format!("player/geiger{}.wav", j + 1), vol))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geiger_clicks() {
        let mut r = CrtRand(1);
        assert_eq!(draw(0, &mut r), None);
        assert_eq!(draw(1000, &mut r), None);
        assert_eq!(r, CrtRand(1), "out of range takes no rand");
        // Far: three rands, never a click.
        assert_eq!(draw(900, &mut r), None);
        let mut three = CrtRand(1);
        (0..3).for_each(|_| {
            three.next();
        });
        assert_eq!(r, three);
        // Seed 1: 41, 18467, 6334, 26500, 19169: 41 & 127 = 41, 18467 & 127 = 35 < 95 (close):
        // a click, sample from 6334 & 1 = 0, volume 41 / 255 + 0.25.
        let mut r = CrtRand(1);
        let (s, v) = draw(10, &mut r).unwrap();
        assert_eq!(s, "player/geiger1.wav");
        assert!((v - (41.0 / 255.0 + 0.25)).abs() < 1e-6);
        // At 450 the chance is 8: 35 and 6334 & 127 = 62 both miss.
        assert_eq!(draw(450, &mut CrtRand(1)), None);
        // Three bits at 250 (chance 28): the third rand, 6334 & 127 = 62, misses too.
        assert_eq!(draw(250, &mut CrtRand(1)), None);
    }
}
