//! Kullanıcı ayar dosyasına yalnız değişen anahtarı yazar (`toml_edit`: yorum + sıra korunur);
//! sonuç dosyanın kendi ayrıştırıcısıyla doğrulanmadan diske gitmez. lowmon-ui Ayarlar ve LowDeck (ADR-0010) ortak.
use toml_edit::{DocumentMut, Item, Table, value};

/// Yazılacak değer (form alanları yalnız bunlar).
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Anahtarı sil (varsayılana dön; ör. blok monitörü "ortak")
    Remove,
}

/// `path` = tablo yolu + anahtar (`["sys", "warn_pct"]`); eksik tablolar oluşturulur.
pub fn set(text: &str, path: &[&str], v: &Val) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let (key, tables) = path.split_last().ok_or("boş yol")?;
    let mut t: &mut Table = doc.as_table_mut();
    for name in tables {
        let item = t.entry(name).or_insert_with(|| Item::Table(Table::new()));
        t = item
            .as_table_mut()
            .ok_or_else(|| format!("{name} tablo değil"))?;
    }
    // Var olan değerin süsü (satır sonu yorumu) korunur
    let decor = t
        .get(key)
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    let mut new = match v {
        Val::Remove => {
            t.remove(key);
            return Ok(doc.to_string());
        }
        Val::Bool(b) => value(*b),
        Val::Int(i) => value(*i),
        Val::Float(f) => value(*f),
        Val::Str(s) => value(s.as_str()),
    };
    if let (Some(d), Some(nv)) = (decor, new.as_value_mut()) {
        *nv.decor_mut() = d;
    }
    // Yerinde değiştir: anahtarın süsü (üstündeki yorum satırları) korunur; `insert` onu siler
    match t.get_mut(key) {
        Some(item) => *item = new,
        None => {
            t.insert(key, new);
        }
    }
    Ok(doc.to_string())
}

/// Değişiklikleri sırayla uygular, sonucu `check` ile (dosyanın kendi ayrıştırıcısı) doğrular.
pub fn apply(
    text: &str,
    changes: &[(&[&str], Val)],
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    let mut out = text.to_owned();
    for (p, v) in changes {
        out = set(&out, p, v)?;
    }
    check(&out)?;
    Ok(out)
}

/// Dosyayı değişikliklerle yazar (yoksa oluşturur); geçici dosya + rename.
pub fn write(
    path: &std::path::Path,
    changes: &[(&[&str], Val)],
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let out = apply(&text, changes, check)?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_key() {
        let out = set(
            "[ai]
monitor = \"left\" # yorum
bubble = true
",
            &["ai", "monitor"],
            &Val::Remove,
        )
        .unwrap();
        assert_eq!(
            out,
            "[ai]
bubble = true
"
        );
    }

    #[test]
    fn keeps_comments_and_order() {
        let src =
            "# baş yorum\nbind = \"192.168.1.5\"\nport = 47810 # port yorumu\ntoken = \"x\"\n";
        let out = set(src, &["port"], &Val::Int(47820)).unwrap();
        assert_eq!(
            out,
            "# baş yorum\nbind = \"192.168.1.5\"\nport = 47820 # port yorumu\ntoken = \"x\"\n"
        );
    }

    #[test]
    fn keeps_comment_above_first_key() {
        let out = set(
            "# baş
refresh_sec = 180
",
            &["refresh_sec"],
            &Val::Int(60),
        )
        .unwrap();
        assert_eq!(
            out,
            "# baş
refresh_sec = 60
"
        );
    }

    #[test]
    fn creates_tables() {
        let out = set("", &["sys", "cpu_temp"], &Val::Bool(false)).unwrap();
        assert_eq!(out, "[sys]\ncpu_temp = false\n");
        let out = set(&out, &["ai", "bubble"], &Val::Bool(true)).unwrap();
        let out = set(&out, &["font_pt"], &Val::Float(9.5)).unwrap();
        let v: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(v["sys"]["cpu_temp"].as_bool(), Some(false));
        assert_eq!(v["ai"]["bubble"].as_bool(), Some(true));
        assert_eq!(v["font_pt"].as_float(), Some(9.5));
    }

    #[test]
    fn check_blocks_bad_result() {
        let ok = |_: &str| Ok(());
        let deny = |_: &str| Err("geçersiz".to_string());
        assert!(apply("a = 1", &[(&["a"], Val::Int(2))], ok).is_ok());
        assert_eq!(
            apply("a = 1", &[(&["a"], Val::Int(2))], deny),
            Err("geçersiz".into())
        );
        assert!(set("{ bozuk", &["a"], &Val::Int(1)).is_err());
        assert!(set("x = 1", &["x", "y"], &Val::Int(1)).is_err());
    }
}
