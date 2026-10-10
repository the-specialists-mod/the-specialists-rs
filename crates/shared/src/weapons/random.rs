//! Faithful port of the SDK shared random generator (`dlls/util.cpp` `U_Random`, `U_Srand`,
//! `UTIL_SharedRandomLong`, `UTIL_SharedRandomFloat`). TS 3.0 uses the same seed table (server
//! `seed_table`, client `0x10093168`), so a seeded pellet pattern is identical on both sides.
//!
//! The generator state is local to each call (the SDK keeps it in a global, but every shared
//! random call starts with `U_Srand`, so no state survives between calls).

/// SDK `seed_table[256]`.
pub const SEED_TABLE: [u32; 256] = [
    28985, 27138, 26457, 9451, 17764, 10909, 28790, 8716, 6361, 4853, 17798, 21977, 19643, 20662, 10834, 20103, 27067, 28634, 18623, 25849,
    8576, 26234, 23887, 18228, 32587, 4836, 3306, 1811, 3035, 24559, 18399, 315, 26766, 907, 24102, 12370, 9674, 2972, 10472, 16492, 22683,
    11529, 27968, 30406, 13213, 2319, 23620, 16823, 10013, 23772, 21567, 1251, 19579, 20313, 18241, 30130, 8402, 20807, 27354, 7169, 21211,
    17293, 5410, 19223, 10255, 22480, 27388, 9946, 15628, 24389, 17308, 2370, 9530, 31683, 25927, 23567, 11694, 26397, 32602, 15031, 18255,
    17582, 1422, 28835, 23607, 12597, 20602, 10138, 5212, 1252, 10074, 23166, 19823, 31667, 5902, 24630, 18948, 14330, 14950, 8939, 23540,
    21311, 22428, 22391, 3583, 29004, 30498, 18714, 4278, 2437, 22430, 3439, 28313, 23161, 25396, 13471, 19324, 15287, 2563, 18901, 13103,
    16867, 9714, 14322, 15197, 26889, 19372, 26241, 31925, 14640, 11497, 8941, 10056, 6451, 28656, 10737, 13874, 17356, 8281, 25937, 1661,
    4850, 7448, 12744, 21826, 5477, 10167, 16705, 26897, 8839, 30947, 27978, 27283, 24685, 32298, 3525, 12398, 28726, 9475, 10208, 617,
    13467, 22287, 2376, 6097, 26312, 2974, 9114, 21787, 28010, 4725, 15387, 3274, 10762, 31695, 17320, 18324, 12441, 16801, 27376, 22464,
    7500, 5666, 18144, 15314, 31914, 31627, 6495, 5226, 31203, 2331, 4668, 12650, 18275, 351, 7268, 31319, 30119, 7600, 2905, 13826, 11343,
    13053, 15583, 30055, 31093, 5067, 761, 9685, 11070, 21369, 27155, 3663, 26542, 20169, 12161, 15411, 30401, 7580, 31784, 8985, 29367,
    20989, 14203, 29694, 21167, 10337, 1706, 28578, 887, 3373, 19477, 14382, 675, 7033, 15111, 26138, 12252, 30996, 21409, 25678, 18555,
    13256, 23316, 22407, 16727, 991, 9236, 5373, 29402, 6117, 15241, 27715, 19291, 19888, 19847,
];

/// `U_Srand` + `U_Random` state.
struct Gen(u32);

impl Gen {
    fn srand(seed: u32) -> Self {
        Gen(SEED_TABLE[(seed & 0xff) as usize])
    }

    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(69069);
        self.0 = self.0.wrapping_add(SEED_TABLE[(self.0 & 0xff) as usize]);
        self.0 = self.0.wrapping_add(1);
        self.0 & 0x0fff_ffff
    }
}

/// `UTIL_SharedRandomLong(seed, low, high)`.
pub fn shared_random_long(seed: u32, low: i32, high: i32) -> i32 {
    let mut g = Gen::srand(seed.wrapping_add(low as u32).wrapping_add(high as u32));
    let range = high.wrapping_sub(low).wrapping_add(1) as u32;
    if range.wrapping_sub(1) == 0 {
        return low;
    }
    let rnum = g.next();
    low.wrapping_add((rnum % range) as i32)
}

/// `UTIL_SharedRandomFloat(seed, low, high)`.
///
/// The seed mixes in the raw bit patterns of `low` and `high`. The range is truncated to an
/// integer (TS `ts_i386.so` 0xebc54 truncates through a 64-bit `fistp`, the SDK converts to
/// `unsigned int`), so `(-pi, pi)` really yields `[-pi, -pi + 6)`. The offset is rounded to a
/// float before the multiply, as in the binary; the multiply-add is exact in f64 and rounded once.
pub fn shared_random_float(seed: u32, low: f32, high: f32) -> f32 {
    let mut g = Gen::srand(seed.wrapping_add(low.to_bits()).wrapping_add(high.to_bits()));
    g.next();
    g.next();
    let range = (high as f64 - low as f64).trunc() as i64 as u32;
    if range == 0 {
        return low;
    }
    let tensixrand = g.next() & 65535;
    let offset = (tensixrand as f64 / 65536.0) as f32;
    (offset as f64 * range as f64 + low as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from the SDK C code (util.cpp `UTIL_SharedRandomFloat/Long`, compiled
    /// with `gcc -m32 -O0 -mfpmath=387`): (seed, SR(seed,-pi,pi) bits, SR(seed+1,0,1) bits,
    /// SRL(seed,-5,20)).
    const GOLDEN: &[(u32, u32, u32, i32)] = &[
        (0, 0xbfe52db6, 0x3da1e800, -2),
        (7, 0xbfae79b6, 0x3e42d800, 5),
        (14, 0xbfe341b6, 0x3f264000, 12),
        (21, 0x403670a5, 0x3e5ff800, 3),
        (28, 0x3bf74a00, 0x3f7b7d00, 0),
        (35, 0x403301a5, 0x3c440000, 10),
        (42, 0x40112fa5, 0x3f4d8000, 19),
        (49, 0x3ecba528, 0x3f64c900, 16),
        (56, 0x40306025, 0x3f7d3f00, 10),
        (63, 0x3fd5be4a, 0x3f1d1e00, 5),
        (70, 0xc01b875b, 0x3f365b00, 8),
        (77, 0xbfbb09b6, 0x3dc2c000, -3),
        (84, 0xbfc90ab6, 0x3f6d8500, 4),
        (91, 0xbf45cd6c, 0x3f646d00, 16),
        (98, 0x3f98864a, 0x3f1db200, 10),
        (105, 0xbfbd91b6, 0x3f6ea600, -3),
        (112, 0x3e66c250, 0x3eb2ba00, -2),
        (119, 0x3fe43a4a, 0x3f645000, 17),
        (126, 0xbf68b36c, 0x3f4aa800, 7),
        (133, 0xbf8bbab6, 0x3f055400, 13),
        (140, 0x403430a5, 0x3ee0f800, 6),
        (147, 0xbfe686b6, 0x3e088c00, 20),
        (154, 0x4015ed25, 0x3e7a6c00, 3),
        (161, 0x402a58a5, 0x3f381e00, 4),
        (168, 0x3fa09f4a, 0x3f625900, 4),
        (175, 0xc031625b, 0x3f3d5900, -1),
        (182, 0xc024b15b, 0x3e901a00, -4),
        (189, 0xc039365b, 0x3f3e6100, -3),
        (196, 0xbfd3abb6, 0x3f523600, 13),
        (203, 0xbfe5bab6, 0x3f707e00, 14),
        (210, 0x40050125, 0x3e1d3400, 1),
        (217, 0x3dbd14a0, 0x3f42b800, 19),
        (224, 0xbeb5e6d8, 0x3f17df00, 13),
        (231, 0x40217fa5, 0x3f507600, -4),
        (238, 0x40029a25, 0x3da8a800, 4),
        (245, 0xc004fe5b, 0x3f12ab00, 6),
        (252, 0xc020705b, 0x3ea0b600, 18),
        (259, 0x400b6ea5, 0x3f503a00, 17),
        (266, 0x402875a5, 0x3f71b600, 19),
        (273, 0x3f0c0694, 0x3f338300, 2),
        (280, 0x3f622294, 0x3f21df00, -3),
        (287, 0xc00ed1db, 0x3e443400, 10),
        (294, 0xbfe308b6, 0x3f053f00, 8),
    ];

    #[test]
    fn matches_sdk_c_bit_for_bit() {
        let pi = std::f32::consts::PI; // the SDK test literal 3.1415927f32, same bits
        for &(s, a, b, l) in GOLDEN {
            assert_eq!(shared_random_float(s, -pi, pi).to_bits(), a, "seed {s}");
            assert_eq!(shared_random_float(s + 1, 0.0, 1.0).to_bits(), b, "seed {s}");
            assert_eq!(shared_random_long(s, -5, 20), l, "seed {s}");
        }
    }

    #[test]
    fn long_matches_sdk_formula() {
        // range 1 returns low without consuming the generator
        assert_eq!(shared_random_long(123, 5, 5), 5);
        for seed in 0..512u32 {
            let v = shared_random_long(seed, -3, 9);
            assert!((-3..=9).contains(&v));
        }
        // Hand-computed: seed 0, low 0, high 1 -> srand(1) -> table[1]=27138
        let mut g = 27138u32.wrapping_mul(69069);
        g = g.wrapping_add(SEED_TABLE[(g & 0xff) as usize]).wrapping_add(1);
        assert_eq!(shared_random_long(0, 0, 1), ((g & 0x0fff_ffff) % 2) as i32);
    }

    #[test]
    fn float_range_and_truncation() {
        let pi = std::f32::consts::PI;
        for seed in 0..1024u32 {
            let t = shared_random_float(seed, -pi, pi);
            assert!(t >= -pi && t < -pi + 6.0, "{t}");
            let r = shared_random_float(seed, 0.0, 1.0);
            assert!((0.0..1.0).contains(&r));
        }
        // zero range returns low
        assert_eq!(shared_random_float(7, 0.25, 0.75), 0.25);
    }

    #[test]
    fn float_depends_only_on_low_seed_byte() {
        let a = shared_random_float(0x1234_5601, 0.0, 1.0);
        let b = shared_random_float(0x0000_0001, 0.0, 1.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }
}
