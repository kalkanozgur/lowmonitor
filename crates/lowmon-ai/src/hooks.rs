//! Claude Code hook kurulumu (ADR-0034 Parça 4): `~/.claude/settings.json`'a `lowmon-hook`
//! girdilerini ekler / siler. Yalnız komutu `lowmon-hook.exe` olan işleyicilere dokunulur;
//! kullanıcının diğer ayarları ve anahtar sırası korunur (`serde_json` `preserve_order`).
use serde_json::{Map, Value, json};

/// Kurulan olaylar; `Notification` yalnız izin / boşta / soru istemleri (`auth_success` vb. değil).
const EVENTS: [(&str, Option<&str>); 6] = [
    ("UserPromptSubmit", None),
    ("PreToolUse", None),
    ("PostToolUse", None),
    (
        "Notification",
        Some("permission_prompt|idle_prompt|elicitation_dialog"),
    ),
    ("Stop", None),
    ("SessionEnd", None),
];

const EXE: &str = "lowmon-hook.exe";

/// Hook komutu: tırnaklı, ileri eğik çizgili yol (Claude Code Windows'ta Git Bash ile de çalıştırır).
pub fn command(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.to_string_lossy().replace('\\', "/"))
}

fn ours(h: &Value) -> bool {
    h.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.to_ascii_lowercase().contains(EXE))
}

/// Boş metin `{}` sayılır; kök nesne değilse ya da JSON geçersizse dokunulmaz.
fn parse(text: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(text).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Err("kök JSON nesnesi değil".into()),
    }
}

fn to_text(root: &Map<String, Value>) -> String {
    let mut s = serde_json::to_string_pretty(root).unwrap_or_default();
    s.push('\n');
    s
}

/// Bizim işleyicileri siler; boşalan grup / olay / `hooks` anahtarı da gider. Değişti mi döner.
fn strip(root: &mut Map<String, Value>) -> bool {
    let Some(Value::Object(hooks)) = root.get_mut("hooks") else {
        return false;
    };
    let mut changed = false;
    for groups in hooks.values_mut() {
        let Value::Array(groups) = groups else {
            continue;
        };
        for g in groups.iter_mut() {
            if let Some(Value::Array(hs)) = g.get_mut("hooks") {
                let n = hs.len();
                hs.retain(|h| !ours(h));
                changed |= hs.len() != n;
            }
        }
        groups.retain(|g| !matches!(g.get("hooks"), Some(Value::Array(hs)) if hs.is_empty()));
    }
    if changed {
        hooks.retain(|_, v| !matches!(v, Value::Array(a) if a.is_empty()));
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }
    changed
}

/// Kurulu mu: herhangi bir olayda bizim işleyici var mı.
pub fn installed(text: &str) -> bool {
    let Ok(mut root) = parse(text) else {
        return false;
    };
    strip(&mut root)
}

/// Eski girdilerimizi silip güncel komutla yeniden ekler (tekrar çalıştırmak güvenli).
pub fn install(text: &str, cmd: &str) -> Result<String, String> {
    let mut root = parse(text)?;
    strip(&mut root);
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else {
        return Err("\"hooks\" nesne değil".into());
    };
    for (ev, matcher) in EVENTS {
        let groups = hooks.entry(ev).or_insert_with(|| json!([]));
        let Value::Array(groups) = groups else {
            return Err(format!("\"hooks.{ev}\" dizi değil"));
        };
        let h = json!({ "type": "command", "command": cmd, "async": true });
        groups.push(match matcher {
            Some(m) => json!({ "matcher": m, "hooks": [h] }),
            None => json!({ "hooks": [h] }),
        });
    }
    Ok(to_text(&root))
}

/// Bizim girdileri siler; bizden başka bir şey yoksa `None` (dosyaya yazılmaz).
pub fn uninstall(text: &str) -> Result<Option<String>, String> {
    let mut root = parse(text)?;
    Ok(strip(&mut root).then(|| to_text(&root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "\"C:/Program Files/LowMonitor/lowmon-hook.exe\"";

    #[test]
    fn command_quotes_and_slashes() {
        let p = std::path::Path::new(r"C:\Program Files\LowMonitor\lowmon-hook.exe");
        assert_eq!(command(p), CMD);
    }

    #[test]
    fn install_keeps_user_settings_and_order() {
        let src = r#"{
  "model": "opus",
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
  },
  "env": { "A": "1" }
}"#;
        let out = install(src, CMD).unwrap();
        assert!(installed(&out));
        // kullanıcı anahtarları ve sırası yerinde
        let keys: Vec<_> = parse(&out).unwrap().keys().cloned().collect();
        assert_eq!(keys, ["model", "hooks", "env"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["async"], true);
        assert_eq!(
            v["hooks"]["Notification"][0]["matcher"],
            "permission_prompt|idle_prompt|elicitation_dialog"
        );
        assert_eq!(v["hooks"].as_object().unwrap().len(), 6);
    }

    #[test]
    fn install_is_idempotent() {
        let once = install("", CMD).unwrap();
        assert_eq!(install(&once, CMD).unwrap(), once);
    }

    #[test]
    fn uninstall_restores() {
        let src = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#;
        let out = uninstall(&install(src, CMD).unwrap()).unwrap().unwrap();
        assert!(!installed(&out));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, serde_json::from_str::<Value>(src).unwrap());
        // yalnız bizimkiler varsa `hooks` anahtarı da gider
        let out = uninstall(&install("{\"x\":1}", CMD).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            json!({"x": 1})
        );
        // bizden bir şey yoksa yazılmaz
        assert_eq!(uninstall(src).unwrap(), None);
    }

    #[test]
    fn invalid_is_untouched() {
        assert!(install("{ bozuk", CMD).is_err());
        assert!(install("[]", CMD).is_err());
        assert!(install(r#"{"hooks":[]}"#, CMD).is_err());
        assert!(uninstall("{ bozuk").is_err());
        assert!(!installed("{ bozuk"));
    }
}
