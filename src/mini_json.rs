//! Minimal, zero-dependency, RFC 8259-compliant JSON parser for Qsirch 7 API responses.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(HashMap<String, JsonValue>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            JsonValue::Number(n) if *n >= 0.0 => Some(*n as u64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(arr) => Some(arr.as_slice()),
            _ => None,
        }
    }
}

pub fn parse_json(input: &str) -> Result<JsonValue, String> {
    let mut p = Parser {
        input,
        bytes: input.as_bytes(),
        pos: 0,
    };
    let val = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("Trailing characters at byte {}", p.pos));
    }
    Ok(val)
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let Some(b) = self.peek() else {
            return Err("Unexpected end of JSON".into());
        };
        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(JsonValue::String),
            b't' => self.expect_literal(b"true", JsonValue::Bool(true)),
            b'f' => self.expect_literal(b"false", JsonValue::Bool(false)),
            b'n' => self.expect_literal(b"null", JsonValue::Null),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(format!("Unexpected byte '{}' at {}", b as char, self.pos)),
        }
    }

    fn expect_literal(&mut self, lit: &[u8], val: JsonValue) -> Result<JsonValue, String> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("Invalid literal at byte {}", self.pos))
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        let mut map = HashMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("Expected string key at byte {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("Expected ':' at byte {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(map));
                }
                _ => return Err(format!("Expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        let mut arr = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(arr));
        }
        loop {
            let val = self.parse_value()?;
            arr.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(arr));
                }
                _ => return Err(format!("Expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // '"'
        let mut out = String::new();
        let mut chunk_start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                out.push_str(&self.input[chunk_start..self.pos]);
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                out.push_str(&self.input[chunk_start..self.pos]);
                self.pos += 1;
                let Some(&esc) = self.bytes.get(self.pos) else {
                    return Err("Unterminated escape sequence".into());
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0C'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code)
                            && self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u")
                        {
                            self.pos += 2;
                            let low = self.parse_hex4()?;
                            if (0xDC00..=0xDFFF).contains(&low) {
                                let cp = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                if let Some(ch) = char::from_u32(cp) {
                                    out.push(ch);
                                }
                                chunk_start = self.pos;
                                continue;
                            }
                        }
                        if let Some(ch) = char::from_u32(code as u32) {
                            out.push(ch);
                        }
                    }
                    _ => return Err(format!("Invalid escape '\\{}'", esc as char)),
                }
                chunk_start = self.pos;
            } else {
                self.pos += 1;
            }
        }
        Err("Unterminated JSON string".into())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("Incomplete \\u escape".into());
        }
        let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| "Invalid hex in \\u escape".to_string())?;
        self.pos += 4;
        u16::from_str_radix(hex, 16).map_err(|_| format!("Invalid hex digits '{hex}'"))
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "Invalid number".to_string())?;
        let n: f64 = slice
            .parse()
            .map_err(|_| format!("Failed parsing number '{slice}'"))?;
        Ok(JsonValue::Number(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qsirch_like_payload() {
        let raw = r#"{"total":2,"items":[{"id":"1","name":"a","size":42,"preview":{"info":[{"key":"path","value":"/Multimedia/a.pdf"}]}}]}"#;
        let v = parse_json(raw).unwrap();
        assert_eq!(v.get("total").and_then(|x| x.as_u64()), Some(2));
        let items = v.get("items").and_then(|x| x.as_array()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("size").and_then(|x| x.as_u64()), Some(42));
    }
}
