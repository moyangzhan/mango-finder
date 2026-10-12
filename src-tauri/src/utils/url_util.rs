/// URL encoding utility functions
///
/// Provides URL encoding functionality without external dependencies

/// Encode a string for use in a URL path segment
///
/// This function encodes characters that are not allowed in URL paths,
/// following RFC 3986 guidelines with some practical adjustments for file paths.
///
/// # Arguments
/// * `s` - The string to encode
///
/// # Returns
/// The URL-encoded string
///
/// # Examples
/// ```
/// let encoded = url_encode("hello world.txt");
/// assert_eq!(encoded, "hello%20world.txt");
/// ```
pub fn url_encode(s: &str) -> String {
    s.chars()
        .map(|c| {
            // Unreserved characters (RFC 3986) plus some common path characters
            match c {
                // Alphanumeric characters
                'A'..='Z' | 'a'..='z' | '0'..='9' => c.to_string(),
                // Unreserved special characters
                '-' | '_' | '.' | '~' => c.to_string(),
                // Common path separators (keep as-is for readability)
                '/' | '\\' => c.to_string(),
                // Colon for Windows drive letters (e.g., C:)
                ':' => c.to_string(),
                // Percent-encode everything else
                _ => format!("%{:02X}", c as u32),
            }
        })
        .collect()
}

/// Decode a URL-encoded string
///
/// # Arguments
/// * `s` - The URL-encoded string to decode
///
/// # Returns
/// The decoded string, or None if the encoding is invalid
pub fn url_decode(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Try to read two hex digits
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            return None; // Invalid encoding
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }

    Some(result)
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
        assert_eq!(url_decode("hello+world"), Some("hello world".to_string()));
    }

    #[test]
    fn test_url_roundtrip() {
        let original = "test file @#$%^&.txt";
        let encoded = url_encode(original);
        let decoded = url_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
