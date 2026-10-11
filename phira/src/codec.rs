//! Phira 内置资源解密。
//!
//! 磁盘格式 = 8字节块变换( zstd(原始数据) + 填充 )，无密钥、可逆。
//! 算法逆向自 https://github.com/wiorigami/phira-customizer (GPL-3.0)。

fn decrypt_block(v: &mut [u8; 8]) {
    v.swap(1, 4);
    v.swap(2, 6);
    // _perm: 交换 (1,4) 与 (3,6)
    v.swap(1, 4);
    v.swap(3, 6);
    let mut i = 0;
    while i < 8 {
        v[i | 1] = v[i | 1].wrapping_sub(v[i]);
        i += 2;
    }
    let mut i = 0;
    while i < 8 {
        v[i | 2] = v[i | 2].wrapping_sub(v[i]);
        v[i | 3] = v[i | 3].wrapping_sub(v[i | 1]);
        i += 4;
    }
    v[4] = v[4].wrapping_sub(v[0]);
    v[5] = v[5].wrapping_sub(v[1]);
    v[6] = v[6].wrapping_sub(v[2]);
    v[7] = v[7].wrapping_sub(v[3]);
    v.swap(3, 5);
}

/// 把 Phira 资源文件字节还原为原始文件字节。
pub fn phira_decrypt(data: &[u8]) -> Vec<u8> {
    let n = data.len() / 8 * 8;
    let mut out = data[..n].to_vec();
    let mut i = 0;
    while i < n {
        let mut blk = [0u8; 8];
        blk.copy_from_slice(&out[i..i + 8]);
        decrypt_block(&mut blk);
        out[i..i + 8].copy_from_slice(&blk);
        i += 8;
    }
    if data.len() >= 8 && !out.is_empty() {
        let last = *out.last().unwrap() as usize;
        let new_size = data.len() + last - 8;
        if new_size > 0 && new_size <= out.len() {
            out.truncate(new_size);
        }
    }
    zstd::stream::decode_all(&out[..]).unwrap_or_default()
}
