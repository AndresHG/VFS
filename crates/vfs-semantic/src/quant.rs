//! Vector quantization for `vectors.db` (VFS-29 R6, R11; research §3): a 1-bit row (the sign
//! of each dimension) scanned by Hamming distance, and an int8 row with one scale per vector
//! for the rescore.

/// Scales `v` to unit length (a zero vector stays zero).
pub fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// The sign bits of `v`, eight dimensions per byte, most significant bit first.
pub fn bits(v: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; v.len().div_ceil(8)];
    for (i, &x) in v.iter().enumerate() {
        if x > 0.0 {
            out[i / 8] |= 0x80 >> (i % 8);
        }
    }
    out
}

/// The sign bits of `v - mean` (see [`crate::store::Store::insert`]).
pub fn centred_bits(v: &[f32], mean: &[f32]) -> Vec<u8> {
    let d: Vec<f32> = v.iter().zip(mean).map(|(a, m)| a - m).collect();
    bits(&d)
}

/// `v` as int8 values and the scale that maps them back (`value ≈ q × scale`).
pub fn int8(v: &[f32]) -> (Vec<i8>, f32) {
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    if max == 0.0 {
        return (vec![0; v.len()], 0.0);
    }
    let scale = max / 127.0;
    let q = v
        .iter()
        .map(|x| (x / scale).round().clamp(-127.0, 127.0) as i8)
        .collect();
    (q, scale)
}

/// Hamming distance between two bit rows of the same length.
pub fn hamming(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// Dot product of a float query with an int8 row and its scale.
pub fn dot_int8(q: &[f32], row: &[i8], scale: f32) -> f32 {
    q.iter().zip(row).map(|(a, &b)| a * b as f32).sum::<f32>() * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-29 R6: the 1-bit row keeps the signs, eight per byte.
    #[test]
    fn bits_keep_signs() {
        let v = [0.5, -0.1, 0.0, 0.2, -0.3, 0.1, 0.1, -0.2, 0.7];
        assert_eq!(bits(&v), vec![0b1001_0110, 0b1000_0000]);
        assert_eq!(hamming(&bits(&v), &bits(&v)), 0);
        let w: Vec<f32> = v.iter().map(|x| -x).collect();
        assert_eq!(hamming(&bits(&v), &bits(&w)), 8); // the zero is 0 in both
    }

    /// VFS-29 R6, R11: the int8 row and its scale give back the dot product closely.
    #[test]
    fn int8_rescore_is_close_to_float() {
        let mut v: Vec<f32> = (0..384)
            .map(|i| ((i * 37 % 101) as f32 - 50.0) / 50.0)
            .collect();
        normalise(&mut v);
        let mut q: Vec<f32> = (0..384)
            .map(|i| ((i * 13 % 89) as f32 - 44.0) / 44.0)
            .collect();
        normalise(&mut q);
        let exact: f32 = q.iter().zip(&v).map(|(a, b)| a * b).sum();
        let (row, scale) = int8(&v);
        assert!((dot_int8(&q, &row, scale) - exact).abs() < 0.01);
        assert_eq!(int8(&[0.0, 0.0]), (vec![0, 0], 0.0));
    }
}
