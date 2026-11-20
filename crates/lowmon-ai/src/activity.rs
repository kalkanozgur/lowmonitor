//! Claude Code etkinlik durumu (ADR-0034): hook olayları → "çalışıyor / bekliyor / bitti".
//! Saf kod: hook JSON'undan yalnız `hook_event_name`, `session_id`, `tool_name`, `project`, `detail` okunur
//! (`lowmon-hook` güvenli özeti, ADR-0034 eki); komut, tam yol, ileti metni hook'tan hiç gelmez. Zamanlayıcı / G/Ç yok; çağıran (`lowmon-tray`) olay
//! geldikçe `apply`, mevcut 30 s'lik turunda `prune` çağırır. Claude kapalıyken olay gelmez → iş yok.
use crate::json;
use crate::text::{Lang, pick};

/// "Bitti" bu kadar gösterilir, sonra oturum listeden düşer.
pub const DONE_SEC: i64 = 5;
/// Bu kadar olay gelmeyen "çalışıyor" oturum bayattır (çökme / kill).
const WORKING_STALE_SEC: i64 = 10 * 60;
/// "Bekliyor" daha uzun yaşar (kullanıcı uzaklaşmış olabilir) ama sonsuza dek değil.
const WAITING_STALE_SEC: i64 = 60 * 60;
const MAX_SESSIONS: usize = 16;
const LABEL_MAX: usize = 24;
/// Balonun ayrıntı satırı en fazla bu kadar karakter.
const DETAIL_MAX: usize = 32;

/// Öncelik sırası: küçük olan önce gösterilir.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Waiting,
    Working,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label {
    Thinking,
    /// Kısaltılmış araç adı ("Bash", "Edit", MCP'de son parça)
    Tool(String),
    Question,
    Input,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hook {
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Notification,
    Stop,
    SessionEnd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub session: String,
    pub hook: Hook,
    /// `PreToolUse` / `PostToolUse` için araç adı; diğerlerinde boş
    pub tool: String,
    /// Proje klasörünün adı (`cwd` son parçası); yoksa boş
    pub project: String,
    /// Güvenli özet (Bash açıklaması, dosya adı, desen); yoksa boş
    pub detail: String,
}

/// Hook'un stdin JSON'u. Bilinmeyen olay ya da `session_id` yoksa `None` (yok sayılır).
pub fn parse_hook(doc: &[u8]) -> Option<Event> {
    let root = json::root(doc).filter(|&r| json::is_object(doc, r))?;
    let field = |k| json::member(doc, root, k).and_then(|v| json::as_string(doc, v));
    let hook = match field("hook_event_name")?.as_str() {
        "UserPromptSubmit" => Hook::UserPromptSubmit,
        "PreToolUse" => Hook::PreToolUse,
        "PostToolUse" => Hook::PostToolUse,
        "Notification" => Hook::Notification,
        "Stop" => Hook::Stop,
        "SessionEnd" => Hook::SessionEnd,
        _ => return None,
    };
    Some(Event {
        session: field("session_id").filter(|s| !s.is_empty())?,
        hook,
        tool: field("tool_name").unwrap_or_default(),
        project: field("project").unwrap_or_default(),
        detail: field("detail")
            .map(|d| cut(&d, DETAIL_MAX))
            .unwrap_or_default(),
    })
}

/// `max` karakterden uzunsa `…` ile keser; satır sonu boşluğa döner (balon satırı bölünmesin).
fn cut(s: &str, max: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() > max {
        s.chars().take(max - 1).chain(['…']).collect()
    } else {
        s
    }
}

/// Proje adının ilk 2 kelimesi: `lowmonitor-event-notification-3dcb4a` → `lowmonitor-event`.
fn short_project(p: &str) -> &str {
    match p.match_indices(['-', '_', ' ', '.']).nth(1) {
        Some((i, _)) => &p[..i],
        None => p,
    }
}

/// "mcp__sunucu__arac" → "arac"; uzun adlar `LABEL_MAX` karakterde kesilir.
pub fn tool_label(name: &str) -> String {
    cut(name.rsplit("__").next().unwrap_or(name), LABEL_MAX)
}

/// Balon metni.
pub fn text(label: &Label, lang: Lang) -> String {
    match label {
        Label::Thinking => pick(lang, "Düşünüyor…", "Thinking…").into(),
        Label::Tool(t) => format!("{t}…"),
        Label::Question => pick(lang, "Soru bekliyor", "Waiting for answer").into(),
        Label::Input => pick(lang, "Girdi bekliyor", "Waiting for input").into(),
        Label::Done => pick(lang, "Bitti", "Done").into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Session {
    id: String,
    project: String,
    /// Yalnız araç çalışırken dolu
    detail: String,
    kind: Kind,
    label: Label,
    /// Son olayın zamanı (Unix s)
    at: i64,
}

/// Açık Claude Code oturumlarının durumu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    sessions: Vec<Session>,
}

impl Activity {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Olayı işler; gösterilecek durum değişmiş olabilir (`current`).
    pub fn apply(&mut self, ev: &Event, now: i64) {
        let pos = self.sessions.iter().position(|s| s.id == ev.session);
        let (kind, label) = match ev.hook {
            Hook::SessionEnd => {
                if let Some(i) = pos {
                    self.sessions.remove(i);
                }
                return;
            }
            Hook::UserPromptSubmit | Hook::PostToolUse => (Kind::Working, Label::Thinking),
            Hook::PreToolUse if ev.tool == "AskUserQuestion" => (Kind::Waiting, Label::Question),
            Hook::PreToolUse => (Kind::Working, Label::Tool(tool_label(&ev.tool))),
            // İzin istemi çalışan oturumda gelir. Tur bittikten sonraki "boşta" bildirimi (oturum
            // listede yok ya da "bitti") bekleme sayılmaz: balon kullanıcı yokken turuncu kalmasın.
            Hook::Notification => {
                if pos.is_none_or(|i| self.sessions[i].kind != Kind::Working) {
                    return;
                }
                (Kind::Waiting, Label::Input)
            }
            Hook::Stop => (Kind::Done, Label::Done),
        };
        let detail = if matches!(label, Label::Tool(_)) {
            ev.detail.clone()
        } else {
            String::new()
        };
        match pos {
            Some(i) => {
                let s = &mut self.sessions[i];
                (s.kind, s.label, s.at, s.detail) = (kind, label, now, detail);
                if !ev.project.is_empty() {
                    s.project = ev.project.clone();
                }
            }
            None => {
                if self.sessions.len() >= MAX_SESSIONS {
                    // en eski olaylı oturum düşer
                    if let Some(i) = (0..self.sessions.len()).min_by_key(|&i| self.sessions[i].at) {
                        self.sessions.remove(i);
                    }
                }
                self.sessions.push(Session {
                    id: ev.session.clone(),
                    project: ev.project.clone(),
                    detail,
                    kind,
                    label,
                    at: now,
                });
            }
        }
    }

    /// Süresi dolanları düşürür ("bitti" 5 s, bayat "çalışıyor" 10 dk, "bekliyor" 60 dk).
    /// Bir şey düştüyse `true`.
    pub fn prune(&mut self, now: i64) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|s| {
            let ttl = match s.kind {
                Kind::Done => DONE_SEC,
                Kind::Working => WORKING_STALE_SEC,
                Kind::Waiting => WAITING_STALE_SEC,
            };
            now - s.at < ttl
        });
        self.sessions.len() != before
    }

    fn top(&self) -> Option<&Session> {
        self.sessions
            .iter()
            .min_by_key(|s| (s.kind, std::cmp::Reverse(s.at)))
    }

    /// Gösterilecek durum: `bekliyor > çalışıyor > bitti`, eşitlikte en yeni olay.
    pub fn current(&self) -> Option<(Kind, &Label)> {
        self.top().map(|s| (s.kind, &s.label))
    }

    /// Balon metni (kullanıcı, 2026-11-20): `proje (ilk 2 kelime)[ · +N]` / durum / kısa ayrıntı;
    /// boş satırlar atlanır, bitenler sayılmaz.
    pub fn lines(&self, lang: Lang) -> Option<(Kind, String)> {
        let s = self.top()?;
        let others = self
            .sessions
            .iter()
            .filter(|o| o.id != s.id && o.kind != Kind::Done)
            .count();
        let mut head = short_project(&s.project).to_string();
        if others > 0 {
            if !head.is_empty() {
                head.push_str(" · ");
            }
            head.push_str(&format!("+{others} {}", pick(lang, "oturum", "session(s)")));
        }
        let t = [head, text(&s.label, lang), s.detail.clone()]
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Some((s.kind, t))
    }

    /// En az bir oturum kullanıcıdan girdi bekliyor mu (telefon / LM alarmı).
    pub fn any_waiting(&self) -> bool {
        self.sessions.iter().any(|s| s.kind == Kind::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(session: &str, hook: Hook, tool: &str) -> Event {
        Event {
            session: session.into(),
            hook,
            tool: tool.into(),
            project: String::new(),
            detail: String::new(),
        }
    }

    #[test]
    fn lines_show_project_detail_and_others() {
        let mut a = Activity::default();
        let mut e = ev("a", Hook::PreToolUse, "Bash");
        e.project = "lowmonitor-event-notification-3dcb4a".into();
        e.detail = "Release build al".into();
        a.apply(&e, 10);
        assert_eq!(
            a.lines(Lang::Tr),
            Some((
                Kind::Working,
                "lowmonitor-event\nBash…\nRelease build al".into()
            ))
        );
        a.apply(&ev("b", Hook::UserPromptSubmit, ""), 11);
        assert!(!a.any_waiting());
        a.apply(&ev("b", Hook::PreToolUse, "AskUserQuestion"), 12);
        assert!(a.any_waiting());
        // bekleyen öne geçer, diğer oturum sayılır; PostToolUse ayrıntıyı siler
        assert_eq!(
            a.lines(Lang::En),
            Some((Kind::Waiting, "+1 session(s)\nWaiting for answer".into()))
        );
        a.apply(&ev("a", Hook::PostToolUse, "Bash"), 13);
        a.apply(&ev("b", Hook::Stop, ""), 14);
        assert_eq!(
            a.lines(Lang::Tr),
            Some((Kind::Working, "lowmonitor-event\nDüşünüyor…".into()))
        );
    }

    #[test]
    fn parses_hook_json() {
        let doc = br#"{"session_id":"abc","transcript_path":"C:\\x.jsonl","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo sk-ant-secret"}}"#;
        assert_eq!(parse_hook(doc), Some(ev("abc", Hook::PreToolUse, "Bash")));
        let doc = br#"{"hook_event_name":"PreToolUse","session_id":"s","project":"lm","tool_name":"Edit","detail":"main.rs"}"#;
        let e = parse_hook(doc).unwrap();
        assert_eq!((e.project.as_str(), e.detail.as_str()), ("lm", "main.rs"));
        let doc = br#"{"hook_event_name":"Stop","session_id":"s"}"#;
        assert_eq!(parse_hook(doc), Some(ev("s", Hook::Stop, "")));
        // bilinmeyen olay, eksik / boş oturum, bozuk JSON → yok sayılır
        assert_eq!(
            parse_hook(br#"{"hook_event_name":"SubagentStop","session_id":"s"}"#),
            None
        );
        assert_eq!(parse_hook(br#"{"hook_event_name":"Stop"}"#), None);
        assert_eq!(
            parse_hook(br#"{"hook_event_name":"Stop","session_id":""}"#),
            None
        );
        assert_eq!(parse_hook(b"{\"hook_event_name\":"), None);
        assert_eq!(parse_hook(b""), None);
    }

    #[test]
    fn tool_labels_and_text() {
        assert_eq!(tool_label("Bash"), "Bash");
        assert_eq!(tool_label("mcp__claude-in-chrome__navigate"), "navigate");
        let long = tool_label("mcp__x__a_very_long_tool_name_that_never_ends");
        assert_eq!(long.chars().count(), LABEL_MAX);
        assert!(long.ends_with('…'));
        assert_eq!(text(&Label::Tool("Bash".into()), Lang::En), "Bash…");
        assert_eq!(text(&Label::Thinking, Lang::Tr), "Düşünüyor…");
        assert_eq!(text(&Label::Input, Lang::En), "Waiting for input");
        assert_eq!(text(&Label::Done, Lang::Tr), "Bitti");
    }

    #[test]
    fn one_turn() {
        let mut a = Activity::default();
        assert_eq!(a.current(), None);
        a.apply(&ev("s", Hook::UserPromptSubmit, ""), 100);
        assert_eq!(a.current(), Some((Kind::Working, &Label::Thinking)));
        a.apply(&ev("s", Hook::PreToolUse, "Bash"), 101);
        assert_eq!(
            a.current(),
            Some((Kind::Working, &Label::Tool("Bash".into())))
        );
        // izin istemi → bekliyor; onaydan sonra araç biter → yine çalışıyor
        a.apply(&ev("s", Hook::Notification, ""), 102);
        assert_eq!(a.current(), Some((Kind::Waiting, &Label::Input)));
        a.apply(&ev("s", Hook::PostToolUse, "Bash"), 110);
        assert_eq!(a.current(), Some((Kind::Working, &Label::Thinking)));
        a.apply(&ev("s", Hook::PreToolUse, "AskUserQuestion"), 111);
        assert_eq!(a.current(), Some((Kind::Waiting, &Label::Question)));
        a.apply(&ev("s", Hook::PostToolUse, "AskUserQuestion"), 120);
        a.apply(&ev("s", Hook::Stop, ""), 121);
        assert_eq!(a.current(), Some((Kind::Done, &Label::Done)));
        // "bitti" 5 s görünür, sonra düşer
        assert!(!a.prune(121 + DONE_SEC - 1));
        assert!(a.prune(121 + DONE_SEC));
        assert!(a.is_empty());
    }

    #[test]
    fn idle_notification_is_not_waiting() {
        let mut a = Activity::default();
        // oturum hiç görülmedi (tray sonradan açıldı) ya da tur bitti → boşta bildirimi yok sayılır
        a.apply(&ev("s", Hook::Notification, ""), 10);
        assert!(a.is_empty());
        a.apply(&ev("s", Hook::UserPromptSubmit, ""), 11);
        a.apply(&ev("s", Hook::Stop, ""), 12);
        a.apply(&ev("s", Hook::Notification, ""), 13);
        assert_eq!(a.current(), Some((Kind::Done, &Label::Done)));
    }

    #[test]
    fn priority_across_sessions() {
        let mut a = Activity::default();
        a.apply(&ev("a", Hook::UserPromptSubmit, ""), 100);
        a.apply(&ev("b", Hook::PreToolUse, "Edit"), 105);
        // iki çalışan: en yeni olay
        assert_eq!(
            a.current(),
            Some((Kind::Working, &Label::Tool("Edit".into())))
        );
        // bekleyen her zaman öne geçer, daha eski olsa da
        a.apply(&ev("a", Hook::Notification, ""), 106);
        a.apply(&ev("b", Hook::PostToolUse, "Edit"), 107);
        assert_eq!(a.current(), Some((Kind::Waiting, &Label::Input)));
        // biten, çalışanın arkasında kalır
        a.apply(&ev("a", Hook::Stop, ""), 108);
        assert_eq!(a.current(), Some((Kind::Working, &Label::Thinking)));
        a.apply(&ev("b", Hook::SessionEnd, ""), 109);
        assert_eq!(a.current(), Some((Kind::Done, &Label::Done)));
        a.apply(&ev("a", Hook::SessionEnd, ""), 110);
        assert!(a.is_empty());
    }

    #[test]
    fn stale_and_capacity() {
        let mut a = Activity::default();
        a.apply(&ev("w", Hook::UserPromptSubmit, ""), 0);
        a.apply(&ev("q", Hook::UserPromptSubmit, ""), 0);
        a.apply(&ev("q", Hook::Notification, ""), 0);
        // çöken oturum: 10 dk sonra "çalışıyor" düşer, "bekliyor" 60 dk yaşar
        assert!(!a.prune(WORKING_STALE_SEC - 1));
        assert!(a.prune(WORKING_STALE_SEC));
        assert_eq!(a.current(), Some((Kind::Waiting, &Label::Input)));
        assert!(a.prune(WAITING_STALE_SEC));
        assert!(a.is_empty());
        // kapasite: en eski olaylı oturum düşer
        for i in 0..MAX_SESSIONS as i64 + 3 {
            a.apply(&ev(&format!("s{i}"), Hook::UserPromptSubmit, ""), i);
        }
        assert_eq!(a.sessions.len(), MAX_SESSIONS);
        assert!(a.sessions.iter().all(|s| s.id != "s0" && s.id != "s2"));
    }
}
