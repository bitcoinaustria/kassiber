//! Decode persisted JSON into Python's value model, including its extensions.
//! Iterative container parsing avoids using the Rust stack for stored input.
use kassiber_pyjson::{BigInt, Key, PyStr, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    Invalid,
    Limit,
}
type Parsed<T> = std::result::Result<T, ParseError>;
#[derive(Clone, Copy)]
enum State {
    First,
    Next,
    After,
    Colon,
    Value,
}
enum Frame {
    List(Vec<Value>, State),
    Dict(Vec<(Key, Value)>, Option<PyStr>, State),
}
struct Parser<'a> {
    text: &'a str,
    position: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.position += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }
    fn take(&mut self, byte: u8) -> Parsed<()> {
        if self.peek() != Some(byte) {
            return Err(ParseError::Invalid);
        }
        self.position += 1;
        Ok(())
    }
    fn keyword(&mut self, word: &str, value: Value) -> Parsed<Value> {
        if !self.text[self.position..].starts_with(word) {
            return Err(ParseError::Invalid);
        }
        self.position += word.len();
        Ok(value)
    }
    fn hex4(&mut self) -> Parsed<u32> {
        let end = self.position.checked_add(4).ok_or(ParseError::Invalid)?;
        let bytes = self
            .text
            .as_bytes()
            .get(self.position..end)
            .ok_or(ParseError::Invalid)?;
        let mut value = 0;
        for byte in bytes {
            value = value * 16
                + match byte {
                    b'0'..=b'9' => u32::from(byte - b'0'),
                    b'a'..=b'f' => u32::from(byte - b'a' + 10),
                    b'A'..=b'F' => u32::from(byte - b'A' + 10),
                    _ => return Err(ParseError::Invalid),
                };
        }
        self.position = end;
        Ok(value)
    }
    fn string(&mut self) -> Parsed<PyStr> {
        self.take(b'"')?;
        let mut points = Vec::new();
        loop {
            let byte = self.peek().ok_or(ParseError::Invalid)?;
            match byte {
                b'"' => {
                    self.position += 1;
                    return PyStr::from_code_points(points).map_err(|_| ParseError::Invalid);
                }
                0..=0x1f => return Err(ParseError::Invalid),
                b'\\' => {
                    self.position += 1;
                    let escaped = self.peek().ok_or(ParseError::Invalid)?;
                    self.position += 1;
                    let point = match escaped {
                        b'"' => 34,
                        b'\\' => 92,
                        b'/' => 47,
                        b'b' => 8,
                        b'f' => 12,
                        b'n' => 10,
                        b'r' => 13,
                        b't' => 9,
                        b'u' => {
                            let high = self.hex4()?;
                            if (0xd800..=0xdbff).contains(&high)
                                && self.text[self.position..].starts_with("\\u")
                            {
                                let previous = self.position;
                                self.position += 2;
                                if let Ok(low) = self.hex4() {
                                    if (0xdc00..=0xdfff).contains(&low) {
                                        0x10000 + (high - 0xd800) * 0x400 + low - 0xdc00
                                    } else {
                                        self.position = previous;
                                        high
                                    }
                                } else {
                                    self.position = previous;
                                    high
                                }
                            } else {
                                high
                            }
                        }
                        _ => return Err(ParseError::Invalid),
                    };
                    points.push(point);
                }
                _ => {
                    let c = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or(ParseError::Invalid)?;
                    points.push(u32::from(c));
                    self.position += c.len_utf8();
                }
            }
        }
    }
    fn scalar(&mut self) -> Parsed<Value> {
        match self.peek() {
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.keyword("true", Value::Bool(true)),
            Some(b'f') => self.keyword("false", Value::Bool(false)),
            Some(b'n') => self.keyword("null", Value::Null),
            Some(b'N') => self.keyword("NaN", Value::Float(f64::NAN)),
            Some(b'I') => self.keyword("Infinity", Value::Float(f64::INFINITY)),
            Some(b'-') if self.text[self.position..].starts_with("-Infinity") => {
                self.keyword("-Infinity", Value::Float(f64::NEG_INFINITY))
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(ParseError::Invalid),
        }
    }
    fn digits(&mut self) -> usize {
        let start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        self.position - start
    }
    fn number(&mut self) -> Parsed<Value> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        let integer_start = self.position;
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(ParseError::Invalid),
        }
        let digits = self.position - integer_start;
        let mut float = false;
        if self.peek() == Some(b'.') {
            float = true;
            self.position += 1;
            if self.digits() == 0 {
                return Err(ParseError::Invalid);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            float = true;
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if self.digits() == 0 {
                return Err(ParseError::Invalid);
            }
        }
        let text = &self.text[start..self.position];
        if float {
            text.parse()
                .map(Value::Float)
                .map_err(|_| ParseError::Invalid)
        } else if digits > kassiber_pyjson::INT_MAX_STR_DIGITS {
            Err(ParseError::Invalid)
        } else {
            BigInt::parse_bytes(text.as_bytes(), 10)
                .map(Value::Int)
                .ok_or(ParseError::Invalid)
        }
    }
}
fn feed(stack: &mut [Frame], result: &mut Option<Value>, value: Value) -> Parsed<()> {
    match stack.last_mut() {
        Some(Frame::List(values, state)) => {
            values.push(value);
            *state = State::After;
        }
        Some(Frame::Dict(values, key, state)) => {
            let key = Key::Str(key.take().ok_or(ParseError::Invalid)?);
            if let Some((_, old)) = values.iter_mut().find(|(existing, _)| existing == &key) {
                *old = value;
            } else {
                values.push((key, value));
            }
            *state = State::After;
        }
        None => {
            if result.is_some() {
                return Err(ParseError::Invalid);
            }
            *result = Some(value);
        }
    }
    Ok(())
}
pub(crate) fn loads(text: &str) -> Parsed<Value> {
    let mut parser = Parser { text, position: 0 };
    let mut stack = Vec::new();
    let mut result = None;
    let mut need_value = true;
    loop {
        parser.whitespace();
        if need_value {
            match parser.peek() {
                Some(b'[') => {
                    parser.position += 1;
                    stack.push(Frame::List(Vec::new(), State::First));
                }
                Some(b'{') => {
                    parser.position += 1;
                    stack.push(Frame::Dict(Vec::new(), None, State::First));
                }
                _ => {
                    let value = parser.scalar()?;
                    feed(&mut stack, &mut result, value)?;
                }
            }
            // Python's default JSON decoder hits its recursion guard near 1000.
            // Refuse deeper data explicitly, never treat it as an empty object.
            if stack.len() > 1000 {
                return Err(ParseError::Limit);
            }
            need_value = false;
            continue;
        }
        let Some(frame) = stack.last_mut() else {
            return if parser.position == text.len() {
                result.ok_or(ParseError::Invalid)
            } else {
                Err(ParseError::Invalid)
            };
        };
        let mut close = false;
        match frame {
            Frame::List(_, state) => match state {
                State::First if parser.peek() == Some(b']') => {
                    parser.position += 1;
                    close = true;
                }
                State::First | State::Next => need_value = true,
                State::After => match parser.peek() {
                    Some(b',') => {
                        parser.position += 1;
                        *state = State::Next;
                    }
                    Some(b']') => {
                        parser.position += 1;
                        close = true;
                    }
                    _ => return Err(ParseError::Invalid),
                },
                _ => return Err(ParseError::Invalid),
            },
            Frame::Dict(_, key, state) => match state {
                State::First if parser.peek() == Some(b'}') => {
                    parser.position += 1;
                    close = true;
                }
                State::First | State::Next => {
                    *key = Some(parser.string()?);
                    *state = State::Colon;
                }
                State::Colon => {
                    parser.take(b':')?;
                    *state = State::Value;
                }
                State::Value => need_value = true,
                State::After => match parser.peek() {
                    Some(b',') => {
                        parser.position += 1;
                        *state = State::Next;
                    }
                    Some(b'}') => {
                        parser.position += 1;
                        close = true;
                    }
                    _ => return Err(ParseError::Invalid),
                },
            },
        }
        if close {
            let value = match stack.pop().expect("nonempty stack") {
                Frame::List(items, _) => Value::List(items),
                Frame::Dict(items, _, _) => Value::Dict(items),
            };
            feed(&mut stack, &mut result, value)?;
        }
    }
}
pub(super) fn get<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Dict(items) => items.iter().find_map(|(key, value)| {
            if key == &Key::from(name) {
                Some(value)
            } else {
                None
            }
        }),
        _ => None,
    }
}
pub(crate) fn dumps(value: &Value, ascii: bool, compact: bool) -> crate::Result<String> {
    kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options {
            ensure_ascii: ascii,
            sort_keys: true,
            separators: if compact {
                Some((",".into(), ":".into()))
            } else {
                None
            },
            ..Default::default()
        },
    )
    .map_err(|error| {
        use crate::stored_values::{refusal, Shape};
        refusal(match error {
            kassiber_pyjson::Error::Surrogate => Shape::LoneSurrogate,
            kassiber_pyjson::Error::OutOfRangeFloat | kassiber_pyjson::Error::NanKeyOrder => {
                Shape::NonFinite
            }
            _ => Shape::Other,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_extensions_round_trip_without_loss() {
        let raw = r#"{"nan":NaN,"pos":Infinity,"neg":-Infinity,"overflow":1e400,"surrogate":"\ud800","pair":"\ud83d\ude00","large":123456789012345678901234567890,"duplicate":1,"duplicate":2}"#;
        let value = loads(raw).unwrap();
        let encoded = dumps(&value, true, true).unwrap();
        assert_eq!(
            encoded,
            r#"{"duplicate":2,"large":123456789012345678901234567890,"nan":NaN,"neg":-Infinity,"overflow":Infinity,"pair":"\ud83d\ude00","pos":Infinity,"surrogate":"\ud800"}"#
        );
        assert!(dumps(&value, false, true).is_err());
    }
    #[test]
    fn malformed_json_is_distinct_from_an_unsupported_depth() {
        for text in [
            "",
            "[1,]",
            "{\"a\":1,}",
            "{a:1}",
            "01",
            "1.",
            "1e",
            "--1",
            "[InfinityX]",
            "\u{feff}{}",
            "\"a\nb\"",
            "\"\\x00\"",
            "\"\\ud800\\uXX00\"",
        ] {
            assert!(loads(text).is_err(), "{text}");
        }
        assert_eq!(
            loads(&format!("{}0{}", "[".repeat(1001), "]".repeat(1001))),
            Err(ParseError::Limit)
        );
        assert_eq!(
            loads(&format!("{}0{}", "[".repeat(100), "]".repeat(100))).unwrap(),
            loads(&format!("{}0{}", "[".repeat(100), "]".repeat(100))).unwrap()
        );
    }
}
