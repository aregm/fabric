//! A gzip body for the compressed-body fault (14.2), without a compression library: the data
//! goes into "stored" deflate blocks (RFC 1951 3.2.4), which every gzip reader accepts.

/// The bytes of a gzip member (RFC 1952) holding `data`.
pub(crate) fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(u8::from(last));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_of_a_known_string() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn a_member_starts_with_the_gzip_magic_and_ends_with_the_length() {
        let out = compress(b"hello");
        assert_eq!(&out[..3], &[0x1f, 0x8b, 8]);
        assert_eq!(&out[out.len() - 4..], &5u32.to_le_bytes());
    }
}
