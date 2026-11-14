//! Маленький JSON: разобрать, поправить, записать обратно. Нужен, чтобы ставить хуки в настройки
//! Claude Code (`~/.claude/settings.json`) без сторонних крейтов. Порядок ключей сохраняется,
//! числа хранятся текстом как были.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Json::Obj(v));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    v.push((k, self.value(depth + 1)?));
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Json::Obj(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Json::Arr(v));
                }
                loop {
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match *self.b.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Json::Arr(v));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'n' => self.word("null", Json::Null),
            _ => {
                let start = self.i;
                while matches!(self.b.get(self.i), Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')) {
                    self.i += 1;
                }
                (self.i > start).then(|| Json::Num(String::from_utf8_lossy(&self.b[start..self.i]).into_owned()))
            }
        }
    }
    fn word(&mut self, w: &str, v: Json) -> Option<Json> {
        self.b.get(self.i..self.i + w.len()).filter(|s| *s == w.as_bytes())?;
        self.i += w.len();
        Some(v)
    }
    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            match *self.b.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return String::from_utf8(out).ok();
                }
                b'\\' => {
                    self.i += 1;
                    match *self.b.get(self.i)? {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = |at: usize| u32::from_str_radix(std::str::from_utf8(self.b.get(at..at + 4)?).ok()?, 16).ok();
                            let hi = hex(self.i + 1)?;
                            self.i += 4;
                            let mut c = hi;
                            if (0xD800..0xDC00).contains(&hi) && self.b.get(self.i + 1) == Some(&b'\\') && self.b.get(self.i + 2) == Some(&b'u') {
                                let lo = hex(self.i + 3)?;
                                self.i += 6;
                                c = 0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0u8; 4]).as_bytes());
                        }
                        c => out.push(c),
                    }
                }
                c => out.push(c),
            }
            self.i += 1;
        }
    }
}

/// Разобрать текст целиком. None — это не JSON (или после значения что-то лишнее).
pub fn parse(s: &str) -> Option<Json> {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    let mut p = P { b: s.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

impl Json {
    pub fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Obj(v) => v.iter().find(|(key, _)| key == k).map(|(_, x)| x),
            _ => None,
        }
    }
    /// Значение по ключу; если его нет, вставить `or`. Работает только для объекта.
    pub fn entry(&mut self, k: &str, or: Json) -> Option<&mut Json> {
        let Json::Obj(v) = self else { return None };
        let i = match v.iter().position(|(key, _)| key == k) {
            Some(i) => i,
            None => {
                v.push((k.to_string(), or));
                v.len() - 1
            }
        };
        Some(&mut v[i].1)
    }
    pub fn remove(&mut self, k: &str) {
        if let Json::Obj(v) = self {
            v.retain(|(key, _)| key != k);
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Текст с отступами в два пробела (как пишет сам Claude Code).
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, o: &mut String, ind: usize) {
        let pad = |o: &mut String, n: usize| o.extend(std::iter::repeat_n(' ', n * 2));
        match self {
            Json::Null => o.push_str("null"),
            Json::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => o.push_str(n),
            Json::Str(s) => o.push_str(&crate::feat::agents::json_quote(s)),
            Json::Arr(v) if v.is_empty() => o.push_str("[]"),
            Json::Obj(v) if v.is_empty() => o.push_str("{}"),
            Json::Arr(v) => {
                o.push_str("[\n");
                for (i, x) in v.iter().enumerate() {
                    pad(o, ind + 1);
                    x.write(o, ind + 1);
                    o.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                pad(o, ind);
                o.push(']');
            }
            Json::Obj(v) => {
                o.push_str("{\n");
                for (i, (k, x)) in v.iter().enumerate() {
                    pad(o, ind + 1);
                    o.push_str(&crate::feat::agents::json_quote(k));
                    o.push_str(": ");
                    x.write(o, ind + 1);
                    o.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                pad(o, ind);
                o.push('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"model":"opus","n":-1.5e3,"ok":true,"none":null,"list":[1,"два",{"x":"\"q\"\n"}],"empty":{},"e":[]}"#;
        let v = parse(src).unwrap();
        assert_eq!(v.get("model").and_then(|x| x.as_str()), Some("opus"));
        let again = parse(&v.dump()).unwrap();
        assert_eq!(again, v);
        assert!(v.dump().contains("\n  \"list\": [\n    1,"));
        assert!(parse("{\"a\":1} лишнее").is_none());
        assert!(parse("{\"a\":}").is_none());
        assert_eq!(parse("\u{FEFF}{}"), Some(Json::Obj(vec![])));
    }
}
