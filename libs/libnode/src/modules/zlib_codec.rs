use alloc::vec::Vec;

pub fn deflate_raw(input: &[u8]) -> Vec<u8> {
    libzip_client::deflate_raw(input).unwrap_or_default()
}

pub fn inflate_raw(input: &[u8]) -> Option<Vec<u8>> {
    libzip_client::inflate_raw(input)
}

pub fn deflate_zlib(input: &[u8]) -> Vec<u8> {
    libzip_client::deflate(input).unwrap_or_default()
}

pub fn inflate_zlib(input: &[u8]) -> Option<Vec<u8>> {
    libzip_client::inflate(input)
}

pub fn gzip(input: &[u8]) -> Vec<u8> {
    libzip_client::gzip(input).unwrap_or_default()
}

pub fn gunzip(input: &[u8]) -> Option<Vec<u8>> {
    libzip_client::gunzip(input)
}

pub fn unzip(input: &[u8]) -> Option<Vec<u8>> {
    libzip_client::unzip(input)
}

#[cfg(test)]
mod tests {
    const EXPECTED_LEN: usize = 44_000;
    const RAW_DYNAMIC: &[u8] = &[
        237, 205, 187, 1, 67, 0, 20, 0, 192, 222, 20, 111, 53, 4, 241, 75, 16, 223, 76, 175, 74,
        151, 70, 237, 110, 129, 75, 211, 171, 34, 251, 39, 242, 159, 120, 68, 17, 101, 84, 207,
        186, 105, 187, 254, 245, 30, 198, 233, 51, 47, 235, 182, 31, 223, 228, 114, 102, 179, 217,
        108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102,
        179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155,
        205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217,
        108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102,
        179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155,
        205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217,
        108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102,
        179, 217, 108, 54, 155, 205, 102, 179, 217, 108, 54, 155, 205, 102, 187, 217, 118, 2,
    ];

    #[test]
    fn delegates_dynamic_huffman_inflate_to_libzip_client() {
        let output = super::inflate_raw(RAW_DYNAMIC).expect("dynamic data");
        assert_eq!(output.len(), EXPECTED_LEN);
    }

    #[test]
    fn roundtrips_libzip_client_wrappers() {
        let input = b"hello from node zlib through libzip_client";
        assert_eq!(
            super::gunzip(&super::gzip(input)).as_deref(),
            Some(input.as_slice())
        );
        assert_eq!(
            super::inflate_zlib(&super::deflate_zlib(input)).as_deref(),
            Some(input.as_slice())
        );
        assert_eq!(
            super::inflate_raw(&super::deflate_raw(input)).as_deref(),
            Some(input.as_slice())
        );
    }
}
