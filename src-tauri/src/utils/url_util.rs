//! URL encoding utility functions
//!
//! Provides URL encoding functionality without external dependencies

/// Encode a string for use in a URL path segment
///
/// This function encodes characters that are not allowed in URL paths, following RFC 3986 guidelines with some practical adjustments for file paths.
///
/// # Arguments
/// * `s` - The string to encode
///
/// # Returns
/// The URL-encoded string
///
/// # Examples
/// ```
/// use mango_finder_lib::utils::url_util::url_encode;
/// let encoded = url_encode("hello world.txt");
/// assert_eq!(encoded, "hello%20world.txt");
/// ```
pub fn url_encode(s: &str) -> String {
    // Encode UTF-8 bytes, not code points: non-ASCII expands to multi-byte %XX
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if is_kept_raw(b) {
            out.push(b as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

/// 保留字符:RFC 3986 unreserved + 为路径可读性保留的分隔符与盘符冒号('\' 并非严格合法的 URL 路径字符,仅为 Windows 路径可读性保留)
/// Kept raw: RFC 3986 unreserved + separators and drive colon for readable file paths ('\' is kept for Windows readability, not strict URL validity)
fn is_kept_raw(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b'\\' | b':')
}

/// Decode a URL-encoded string
///
/// Strict RFC 3986: `+` stays a literal plus (form-style `+`-as-space is not applied, matching `url_encode` which emits `%20` for spaces).
///
/// # Arguments
/// * `s` - The URL-encoded string to decode
///
/// # Returns
/// The decoded string, or None if the encoding is invalid (bad escape or decoded bytes not forming valid UTF-8)
pub fn url_decode(s: &str) -> Option<String> {
    // Collect bytes and validate UTF-8 once at the end: assembling multi-byte sequences char-by-char would produce mojibake
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Try to read two hex digits
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2
                && let Ok(byte) = u8::from_str_radix(&hex, 16)
            {
                bytes.push(byte);
            } else {
                return None; // Invalid encoding
            }
        } else {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }

    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode_simple() {
        assert_eq!(url_encode("hello"), "hello");
    }

    #[test]
    fn test_url_encode_space() {
        assert_eq!(url_encode("hello world"), "hello%20world");
    }

    #[test]
    fn test_url_encode_special_chars() {
        assert_eq!(url_encode("file@name.txt"), "file%40name.txt");
    }

    #[test]
    fn test_url_encode_path() {
        assert_eq!(
            url_encode("C:/Users/test/file.txt"),
            "C:/Users/test/file.txt"
        );
    }

    #[test]
    fn test_url_encode_unicode() {
        assert_eq!(url_encode("中文"), "%E4%B8%AD%E6%96%87");
        // 4-byte emoji covers the longest UTF-8 sequence
        assert_eq!(url_encode("😀"), "%F0%9F%98%80");
    }

    #[test]
    fn test_url_decode_simple() {
        assert_eq!(url_decode("hello"), Some("hello".to_string()));
    }

    #[test]
    fn test_url_decode_space() {
        assert_eq!(url_decode("hello%20world"), Some("hello world".to_string()));
    }

    #[test]
    fn test_url_decode_plus() {
        // is a literal plus, not a space
        assert_eq!(url_decode("hello+world"), Some("hello+world".to_string()));
        assert_eq!(url_decode("a%2Bb"), Some("a+b".to_string()));
    }

    #[test]
    fn test_url_decode_invalid_escape() {
        assert_eq!(url_decode("%G1"), None); // not a hex digit
        assert_eq!(url_decode("abc%2"), None); // truncated escape
    }

    #[test]
    fn test_url_decode_unicode() {
        assert_eq!(url_decode("%E4%B8%AD%E6%96%87"), Some("中文".to_string()));
        // Lone continuation byte is invalid UTF-8
        assert_eq!(url_decode("%80"), None);
    }

    #[test]
    fn test_url_roundtrip() {
        let original = "test file @#$%^&.txt 中文 😀 C:\\temp\\a+b.txt";
        let encoded = url_encode(original);
        let decoded = url_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
