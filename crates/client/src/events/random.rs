//! The SDK shared random generator (`UTIL_SharedRandomLong/Float`), as compiled into TS's
//! client.dll (`0x1001ae60` / `0x1001aea0`, seed table `0x10093168`).
//!
//! This is deliberately **not** `shared::weapons::random`: TS's two binaries differ. client.dll
//! keeps `high - low` as a float (verified in the assembly at `0x1001aec3`: `fsubs`/`fsts`, no
//! truncation; decomp `FUN_1001aea0`), while ts_i386.so converts it to an integer
//! (`UTIL_SharedRandomFloat` 0xebc54, as the SDK's `unsigned int range`). For the pellet angle
//! range (-pi, pi) that gives different values for the same seed (seed 77: client `0xbfb0e2eb`,
//! server `0xbfbb09b6`), so an original TS client also draws its own pellets slightly off the
//! server's. Each side keeps its own binary's behaviour; `UTIL_SharedRandomLong` and float ranges
//! with an integral width (the pellet radius, 0..1) agree. The seed is `seed + bits(low) +
//! bits(high)` in both.

const SEED_TABLE: [u32; 256] = [
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

struct Gen(u32);

impl Gen {
    fn seeded(seed: u32) -> Self {
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
    let mut g = Gen::seeded(seed.wrapping_add(low as u32).wrapping_add(high as u32));
    let range = high.wrapping_sub(low) as u32;
    if range == 0 {
        return low;
    }
    let r = g.next();
    low.wrapping_add((r % range.wrapping_add(1)) as i32)
}

/// `UTIL_SharedRandomFloat(seed, low, high)` (TS build: float range).
pub fn shared_random_float(seed: u32, low: f32, high: f32) -> f32 {
    let mut g = Gen::seeded(seed.wrapping_add(low.to_bits()).wrapping_add(high.to_bits()));
    g.next();
    g.next();
    let range = high - low;
    if range == 0.0 {
        return low;
    }
    let t = (g.next() & 0xffff) as f64 * (1.0 / 65536.0);
    // x87: (t * range) + low in extended precision, rounded once to float on return.
    (t * range as f64 + low as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference() {
        // Reference values from a C build of the SDK algorithm with TS's float range (gcc, x86-64:
        // `(double)t * (1/65536.0) * range + low`, see docs/implementation/client-rendering.md).
        assert_eq!(shared_random_long(0, 0, 0), 0);
        assert_eq!(shared_random_float(5, 2.0, 2.0), 2.0);
        for seed in 0..2000u32 {
            let f = shared_random_float(seed, -std::f32::consts::PI, std::f32::consts::PI);
            assert!((-std::f32::consts::PI..std::f32::consts::PI).contains(&f));
            let g = shared_random_float(seed, 0.0, 1.0);
            assert!((0.0..1.0).contains(&g));
            let l = shared_random_long(seed, -5, 5);
            assert!((-5..=5).contains(&l));
        }
        assert_eq!(shared_random_float(1234, 0.0, 1.0), REF_FLOAT_1234);
        assert_eq!(shared_random_long(1234, 0, 100), REF_LONG_1234);
    }

    const REF_FLOAT_1234: f32 = f32::from_bits(0x3f06_5d00); // 0.524856567
    const REF_LONG_1234: i32 = 36;

    /// Same generator as the server's for integral ranges, different for (-pi, pi) (see above).
    #[test]
    fn agrees_with_server_where_ts_does() {
        use shared::weapons::random as sv;
        for seed in 0..512u32 {
            assert_eq!(shared_random_long(seed, -5, 20), sv::shared_random_long(seed, -5, 20));
            assert_eq!(shared_random_float(seed, 0.0, 1.0).to_bits(), sv::shared_random_float(seed, 0.0, 1.0).to_bits());
        }
        let pi = std::f32::consts::PI;
        assert_ne!(shared_random_float(77, -pi, pi).to_bits(), sv::shared_random_float(77, -pi, pi).to_bits());
    }

    #[test]
    fn matches_reference_pi() {
        let f = shared_random_float(77, -std::f32::consts::PI, std::f32::consts::PI);
        assert_eq!(f.to_bits(), 0xbfb0_e2eb); // -1.38192499
    }
}
