//! 与 Go 实现（biliutils/sign.go、utils/hashs/x64hash128.go）保持一致：



use rand::Rng;

pub const WEBVIEW_UA: &str = "Mozilla/5.0 (Linux; Android 12; SM-S9080; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/101.0.4951.61 Safari/537.36 BiliApp/8430300 mobi_app/android isNotchWindow/0 NotchHeight=24 mallVersion/8430300 mVersion/312 disable_rcmd/0 magent/BILI_H5_ANDROID_12_8.43.0_8430300";

pub const SCREEN_INFO: &str = "1699*834*24";

#[inline]
fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51afd7ed558ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
    k ^= k >> 33;
    k
}

fn murmur_x64_128(key: &str, seed: u32) -> (u64, u64) {
    let data = key.as_bytes();
    let nblocks = data.len() / 16;

    let mut h1 = seed as u64;
    let mut h2 = seed as u64;

    let c1: u64 = 0x87c37b91114253d5;
    let c2: u64 = 0x4cf5ad432745937f;

    // 主循环：每次处理 16 字节
    for i in 0..nblocks {
        let mut k1 = u64::from_le_bytes(data[i * 16..i * 16 + 8].try_into().unwrap());
        let mut k2 = u64::from_le_bytes(data[i * 16 + 8..i * 16 + 16].try_into().unwrap());

        k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(31);
        k1 = k1.wrapping_mul(c2);
        h1 ^= k1;

        h1 = h1.rotate_left(27);
        h1 = h1.wrapping_add(h2);
        h1 = h1.wrapping_mul(5).wrapping_add(0x52dce729);

        k2 = k2.wrapping_mul(c2);
        k2 = k2.rotate_left(33);
        k2 = k2.wrapping_mul(c1);
        h2 ^= k2;

        h2 = h2.rotate_left(31);
        h2 = h2.wrapping_add(h1);
        h2 = h2.wrapping_mul(5).wrapping_add(0x38495ab5);
    }

    let tail = &data[nblocks * 16..];
    let len = tail.len();
    let mut k1: u64 = 0;
    let mut k2: u64 = 0;

    if len >= 15 { k2 ^= (tail[14] as u64) << 48; }
    if len >= 14 { k2 ^= (tail[13] as u64) << 40; }
    if len >= 13 { k2 ^= (tail[12] as u64) << 32; }
    if len >= 12 { k2 ^= (tail[11] as u64) << 24; }
    if len >= 11 { k2 ^= (tail[10] as u64) << 16; }
    if len >= 10 { k2 ^= (tail[9] as u64) << 8; }
    if len >= 9 {
        k2 ^= tail[8] as u64;
        k2 = k2.wrapping_mul(c2);
        k2 = k2.rotate_left(33);
        k2 = k2.wrapping_mul(c1);
        h2 ^= k2;
    }
    if len >= 8 { k1 ^= (tail[7] as u64) << 56; }
    if len >= 7 { k1 ^= (tail[6] as u64) << 48; }
    if len >= 6 { k1 ^= (tail[5] as u64) << 40; }
    if len >= 5 { k1 ^= (tail[4] as u64) << 32; }
    if len >= 4 { k1 ^= (tail[3] as u64) << 24; }
    if len >= 3 { k1 ^= (tail[2] as u64) << 16; }
    if len >= 2 { k1 ^= (tail[1] as u64) << 8; }
    if len >= 1 {
        k1 ^= tail[0] as u64;
        k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(31);
        k1 = k1.wrapping_mul(c2);
        h1 ^= k1;
    }

    // 收尾
    h1 ^= data.len() as u64;
    h2 ^= data.len() as u64;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);

    (h1, h2)
}

pub fn get_fe_sign(user_agent: &str, canvas_fp: &str, webgl_fp: &str) -> String {
    let input = format!("{}~{}~{}~{}", canvas_fp, webgl_fp, SCREEN_INFO, user_agent);
    let (h1, h2) = murmur_x64_128(&input, 31);
    format!("{:016x}{:016x}", h1, h2)
}

pub fn random_hex_32() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| std::char::from_digit(rng.gen_range(0..16u32), 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fe_sign_format_is_32_hex() {
        let s = get_fe_sign(WEBVIEW_UA, "abc", "def");
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn murmur_matches_known_vector() {
        assert_eq!(murmur_x64_128("", 0), (0, 0));
    }
}
