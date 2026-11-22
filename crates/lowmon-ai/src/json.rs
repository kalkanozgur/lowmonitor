//! Salt-okunur, bayt aralığı tabanlı JSON gezinme (ai-usage-bar `json_lite` portu).
//! DOM kurulmaz: küçük API yanıtlarından birkaç alan okunur ve Claude Code kimlik dosyasında
//! üç değer, diğer baytlara dokunmadan değiştirilir (`creds::patch`). serde_json bunu veremez.

/// Belgedeki bir değerin bayt aralığı `[begin, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub begin: usize,
    pub end: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

pub fn skip_ws(doc: &[u8], mut pos: usize) -> usize {
    while pos < doc.len() && is_ws(doc[pos]) {
        pos += 1;
    }
    pos
}

/// `pos`'taki (`"`) dizginin kapanış tırnağından bir sonrası.
fn string_end(doc: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos + 1;
    while i < doc.len() {
        match doc[i] {
            b'\\' => i += 1,
            b'"' => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// `pos`'ta (boşluk atlanarak) başlayan değerin bir sonrası; bozuksa `None`.
pub fn value_end(doc: &[u8], pos: usize) -> Option<usize> {
    let pos = skip_ws(doc, pos);
    let c = *doc.get(pos)?;
    match c {
        b'"' => string_end(doc, pos),
        b'{' | b'[' => {
            let mut depth = 0i32;
            let mut i = pos;
            while i < doc.len() {
                match doc[i] {
                    b'"' => {
                        i = string_end(doc, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            // sayı / true / false / null
            let mut i = pos;
            while i < doc.len() && !matches!(doc[i], b',' | b'}' | b']') && !is_ws(doc[i]) {
                i += 1;
            }
            (i != pos).then_some(i)
        }
    }
}

pub fn root(doc: &[u8]) -> Option<Span> {
    let b = skip_ws(doc, 0);
    value_end(doc, b).map(|e| Span { begin: b, end: e })
}

pub fn is_object(doc: &[u8], v: Span) -> bool {
    v.end - v.begin >= 2 && doc[v.begin] == b'{'
}

pub fn is_array(doc: &[u8], v: Span) -> bool {
    v.end - v.begin >= 2 && doc[v.begin] == b'['
}

/// `obj` nesnesinin doğrudan `key` üyesi (iç nesnelerde aranmaz).
pub fn member(doc: &[u8], obj: Span, key: &str) -> Option<Span> {
    if !is_object(doc, obj) {
        return None;
    }
    let mut i = skip_ws(doc, obj.begin + 1);
    while i < obj.end && doc[i] == b'"' {
        let key_end = string_end(doc, i).filter(|&e| e <= obj.end)?;
        let raw_key = &doc[i + 1..key_end - 1];
        i = skip_ws(doc, key_end);
        if i >= obj.end || doc[i] != b':' {
            return None;
        }
        i = skip_ws(doc, i + 1);
        let v_end = value_end(doc, i).filter(|&e| e <= obj.end)?;
        if raw_key == key.as_bytes() {
            return Some(Span {
                begin: i,
                end: v_end,
            });
        }
        i = skip_ws(doc, v_end);
        if i < obj.end && doc[i] == b',' {
            i = skip_ws(doc, i + 1);
        } else {
            return None;
        }
    }
    None
}

/// Dizi elemanları; bozuk elemanda durur.
pub fn elements(doc: &[u8], arr: Span) -> impl Iterator<Item = Span> + '_ {
    let mut i = if is_array(doc, arr) {
        skip_ws(doc, arr.begin + 1)
    } else {
        arr.end
    };
    if i < arr.end && doc[i] == b']' {
        i = arr.end;
    }
    std::iter::from_fn(move || {
        if i >= arr.end {
            return None;
        }
        let e = value_end(doc, i).filter(|&e| e <= arr.end)?;
        let span = Span { begin: i, end: e };
        let next = skip_ws(doc, e);
        i = if next < arr.end && doc[next] == b',' {
            skip_ws(doc, next + 1)
        } else {
            arr.end
        };
        Some(span)
    })
}

fn hex4(s: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(s.get(..4)?).ok()?;
    u32::from_str_radix(s, 16).ok()
}

/// Çözülmüş dizgi (kaçışlar ve `\uXXXX` → UTF-8).
pub fn as_string(doc: &[u8], v: Span) -> Option<String> {
    if v.end - v.begin < 2 || doc[v.begin] != b'"' || doc[v.end - 1] != b'"' {
        return None;
    }
    let s = &doc[v.begin + 1..v.end - 1];
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let e = *s.get(i)?;
        match e {
            b'"' | b'\\' | b'/' => out.push(e),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => {
                let mut cp = hex4(&s[i + 1..])?;
                i += 4;
                // UTF-16 vekil çifti
                if (0xD800..=0xDBFF).contains(&cp)
                    && s.get(i + 1..i + 3) == Some(b"\\u")
                    && let Some(lo) = s
                        .get(i + 3..)
                        .and_then(hex4)
                        .filter(|lo| (0xDC00..=0xDFFF).contains(lo))
                {
                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    i += 6;
                }
                let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            _ => return None,
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

pub fn as_number(doc: &[u8], v: Span) -> Option<f64> {
    std::str::from_utf8(&doc[v.begin..v.end]).ok()?.parse().ok()
}

/// `obj.key` dizgi ise değeri.
pub fn str_member(doc: &[u8], obj: Span, key: &str) -> Option<String> {
    as_string(doc, member(doc, obj, key)?)
}

/// `obj.key` sayı ise değeri; yoksa 0.
pub fn num_member(doc: &[u8], obj: Span, key: &str) -> f64 {
    member(doc, obj, key)
        .and_then(|m| as_number(doc, m))
        .unwrap_or(0.0)
}

/// `s`'yi tırnaklı JSON dizgisi olarak kodlar.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_at(doc: &str, key: &str) -> Option<String> {
        let d = doc.as_bytes();
        as_string(d, member(d, root(d)?, key)?)
    }

    #[test]
    fn members_and_nesting() {
        let doc = br#" { "a" : {"b": [1, {"c":"}"}]}, "b" : 2.5e1 , "s":"x\"y" } "#;
        let r = root(doc).unwrap();
        assert!(is_object(doc, r));
        assert_eq!(as_number(doc, member(doc, r, "b").unwrap()), Some(25.0));
        assert_eq!(
            as_string(doc, member(doc, r, "s").unwrap()).as_deref(),
            Some("x\"y")
        );
        let a = member(doc, r, "a").unwrap();
        assert!(member(doc, r, "c").is_none(), "iç nesnede aranmaz");
        let arr = member(doc, a, "b").unwrap();
        assert_eq!(elements(doc, arr).count(), 2);
    }

    #[test]
    fn escapes_and_unicode() {
        assert_eq!(str_at(r#"{"k":"ç😀\n"}"#, "k").as_deref(), Some("ç😀\n"));
        assert_eq!(str_at(r#"{"k":"\q"}"#, "k"), None);
    }

    #[test]
    fn malformed() {
        assert!(root(b"{\"a\":").is_none());
        assert!(root(b"").is_none());
        let d = br#"{"a" 1}"#;
        assert!(member(d, root(d).unwrap(), "a").is_none());
        let empty = b"[]";
        assert_eq!(elements(empty, root(empty).unwrap()).count(), 0);
    }

    #[test]
    fn quote_roundtrip() {
        let s = "a\"b\\c\n\u{1}ç";
        let q = quote(s);
        assert_eq!(q, "\"a\\\"b\\\\c\\n\\u0001ç\"");
        let d = q.as_bytes();
        assert_eq!(as_string(d, root(d).unwrap()).as_deref(), Some(s));
    }
}
