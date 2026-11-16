//! Bir Claude Code oturum dökümünün (`<config>/projects/*/<oturum>.jsonl`) token dağılımı
//! (ai-usage-bar `usage_breakdown` portu). Yalnız sayılar ve araç adları okunur; mesaj metni
//! boyutuyla ölçülür, saklanmaz.
//!
//! Bir isteğin efektif maliyeti: `input + 0,1 × cache_read + 2 × cache_creation + 5 × output`.
//! Paylaştırma:
//! - sabit taban (sistem istemi + araç listesi) = ilk isteğin bağlamı − o ana dek görülen içerik
//! - isteğin girdi maliyeti, taban ile o ana dek biriken içerik grupları arasında (tahmini)
//!   token oranında bölünür
//! - çıktı maliyeti, o isteğin çağırdığı araçların grubuna gider
//!
//! Tahmin: metin ≈ bayt / 4 token, görsel ≈ 800 token.
use crate::api::parse_iso8601;
use crate::json::{self, Span};
use lowmon_core::time::civil;
use std::collections::HashMap;

const CACHE_READ_WEIGHT: f64 = 0.1;
const CACHE_WRITE_WEIGHT: f64 = 2.0;
const OUTPUT_WEIGHT: f64 = 5.0;
const IMAGE_TOKENS: f64 = 800.0;
const BYTES_PER_TOKEN: f64 = 4.0;

// Grup adları veridir (önbellek ve bulut özet dosyalarında böyle durur); çevrilmez.
pub const INSTRUCTIONS: &str = "Claude talimatları";
pub const CONVERSATION: &str = "Konuşma/diğer";
pub const CHROME: &str = "Claude in Chrome";
pub const SCREEN: &str = "Ekran kontrolü";
pub const BROWSER: &str = "Yerleşik tarayıcı";
pub const WEB: &str = "Web";
pub const FILES: &str = "Dosya/shell";
pub const SKILLS: &str = "Beceriler";
pub const TOOL_LOAD: &str = "Araç yükleme";
pub const SUBAGENTS: &str = "Subagent'lar";
const CONNECTION_PREFIX: &str = "Bağlantı: ";

fn id_like(s: &str) -> bool {
    s.len() >= 16 && s.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
}

/// Araç çağrısının yazıldığı grup. Diğer MCP araçları → "Bağlantı: <sunucu>"; rastgele
/// kimlik görünümlü sunucular aracın adıyla anılır.
pub fn group_for_tool(n: &str) -> String {
    const FILE_TOOLS: [&str; 11] = [
        "Read",
        "Write",
        "Edit",
        "MultiEdit",
        "Glob",
        "Grep",
        "Bash",
        "BashOutput",
        "KillShell",
        "NotebookEdit",
        "LS",
    ];
    let fixed = if n.contains("Claude_Browser") {
        BROWSER
    } else if n.starts_with("mcp__claude-in-chrome__") {
        CHROME
    } else if n.contains("computer") {
        SCREEN
    } else if n == "WebSearch" || n == "WebFetch" {
        WEB
    } else if FILE_TOOLS.contains(&n) || n.starts_with("mcp__remote-devices__device_") {
        FILES
    } else if n == "Skill" {
        SKILLS
    } else if n == "ToolSearch" {
        TOOL_LOAD
    } else if n == "Agent" || n == "Task" {
        SUBAGENTS
    } else if let Some(rest) = n.strip_prefix("mcp__") {
        let (server, tool) = rest.split_once("__").unwrap_or((rest, ""));
        let name = if id_like(server) && !tool.is_empty() {
            tool
        } else {
            server
        };
        return format!("{CONNECTION_PREFIX}{name}");
    } else {
        CONVERSATION
    };
    fixed.to_string()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupCost {
    pub name: String,
    pub effective: f64,
    /// 0..1
    pub share: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub session_id: String,
    pub project: String,
    /// Unix s
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub request_count: usize,
    pub total_effective: f64,
    /// sıfır olmayan gruplar, büyükten küçüğe
    pub groups: Vec<GroupCost>,
}

fn str_member(doc: &[u8], obj: Span, key: &str) -> Option<String> {
    json::as_string(doc, json::member(doc, obj, key)?)
}

fn num_member(doc: &[u8], obj: Span, key: &str) -> f64 {
    json::member(doc, obj, key)
        .and_then(|m| json::as_number(doc, m))
        .unwrap_or(0.0)
}

fn text_tokens(s: Span) -> f64 {
    (s.end - s.begin) as f64 / BYTES_PER_TOKEN
}

/// tool_use / tool_result sarmalayıcısı olmayan içerik bloğunun tahmini token'ı.
fn block_tokens(doc: &[u8], block: Span) -> f64 {
    if json::is_object(doc, block) {
        if str_member(doc, block, "type").as_deref() == Some("image") {
            return IMAGE_TOKENS;
        }
        if let Some(text) = json::member(doc, block, "text") {
            return text_tokens(text);
        }
    }
    text_tokens(block)
}

/// "content" değeri: dizgi ya da blok dizisi.
fn content_tokens(doc: &[u8], content: Span) -> f64 {
    if !json::is_array(doc, content) {
        return text_tokens(content);
    }
    json::elements(doc, content)
        .map(|b| block_tokens(doc, b))
        .sum()
}

struct Pending {
    id: String,
    input: f64,
    cache_read: f64,
    cache_write: f64,
    output: f64,
    /// istek gönderildiği andaki grup başına içerik
    weights: Vec<f64>,
    content_total: f64,
    out_groups: Vec<usize>,
}

/// Akış analizörü: dökümü satır satır besle, sonra bir kez `finish`.
pub struct Analyzer {
    names: Vec<String>,
    /// bağlamdaki tahmini token, grup başına
    content: Vec<f64>,
    /// yazılan efektif maliyet, grup başına
    cost: Vec<f64>,
    content_total: f64,
    base: Option<f64>,
    pending: Option<Pending>,
    /// tool_use id → grup
    tool_groups: HashMap<String, usize>,
    meta: Session,
}

impl Default for Analyzer {
    fn default() -> Self {
        let mut a = Self {
            names: Vec::new(),
            content: Vec::new(),
            cost: Vec::new(),
            content_total: 0.0,
            base: None,
            pending: None,
            tool_groups: HashMap::new(),
            meta: Session::default(),
        };
        a.group_index(INSTRUCTIONS); // 0: sabit tabanı alır
        a.group_index(CONVERSATION); // 1
        a
    }
}

impl Analyzer {
    fn group_index(&mut self, name: &str) -> usize {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i;
        }
        self.names.push(name.to_string());
        self.content.push(0.0);
        self.cost.push(0.0);
        self.names.len() - 1
    }

    fn add_content(&mut self, g: usize, tokens: f64) {
        self.content[g] += tokens;
        self.content_total += tokens;
    }

    pub fn feed_line(&mut self, line: &[u8]) {
        let Some(root) = json::root(line).filter(|&r| json::is_object(line, r)) else {
            return;
        };
        if self.meta.session_id.is_empty()
            && let Some(sid) = str_member(line, root, "sessionId")
        {
            self.meta.session_id = sid;
        }
        if let Some(t) = str_member(line, root, "timestamp").and_then(|s| parse_iso8601(&s)) {
            self.meta.started_at = Some(self.meta.started_at.map_or(t, |s| s.min(t)));
            self.meta.ended_at = Some(self.meta.ended_at.map_or(t, |e| e.max(t)));
        }
        let Some(msg) = json::member(line, root, "message").filter(|&m| json::is_object(line, m))
        else {
            return;
        };
        match str_member(line, root, "type").as_deref() {
            Some("assistant") => self.on_assistant(line, msg),
            Some("user") => {
                self.flush();
                let compact = json::member(line, root, "isCompactSummary")
                    .is_some_and(|m| &line[m.begin..m.end] == b"true");
                if compact {
                    // Bağlam sıkıştırıldı: önceki içerik gitti, yerini özet aldı.
                    self.content.fill(0.0);
                    self.content_total = 0.0;
                }
                self.on_user(line, msg);
            }
            _ => {}
        }
    }

    fn on_assistant(&mut self, line: &[u8], msg: Span) {
        let Some(id) = str_member(line, msg, "id") else {
            return;
        };
        let (mut inp, mut cr, mut cw, mut out) = (0.0, 0.0, 0.0, 0.0);
        if let Some(u) = json::member(line, msg, "usage") {
            inp = num_member(line, u, "input_tokens");
            cr = num_member(line, u, "cache_read_input_tokens");
            cw = num_member(line, u, "cache_creation_input_tokens");
            out = num_member(line, u, "output_tokens");
        }
        if self.pending.as_ref().is_none_or(|p| p.id != id) {
            self.flush();
            if inp + cr + cw + out <= 0.0 {
                return; // sentetik / hata girdilerinde kullanım yok
            }
            self.pending = Some(Pending {
                id,
                input: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                output: 0.0,
                weights: self.content.clone(),
                content_total: self.content_total,
                out_groups: Vec::new(),
            });
        }
        // Bir istek birkaç satıra akar; görülen en büyük değerler tutulur.
        let p = self.pending.as_mut().expect("yukarıda kuruldu");
        p.input = p.input.max(inp);
        p.cache_read = p.cache_read.max(cr);
        p.cache_write = p.cache_write.max(cw);
        p.output = p.output.max(out);

        let Some(content) = json::member(line, msg, "content") else {
            return;
        };
        if !json::is_array(line, content) {
            self.add_content(1, text_tokens(content));
            return;
        }
        for b in json::elements(line, content) {
            match str_member(line, b, "type").as_deref() {
                Some("tool_use") => {
                    let name = str_member(line, b, "name").unwrap_or_default();
                    let g = self.group_index(&group_for_tool(&name));
                    if let Some(tid) = str_member(line, b, "id") {
                        self.tool_groups.insert(tid, g);
                    }
                    let p = self.pending.as_mut().expect("yukarıda kuruldu");
                    if !p.out_groups.contains(&g) {
                        p.out_groups.push(g);
                    }
                    if let Some(input) = json::member(line, b, "input") {
                        self.add_content(g, text_tokens(input));
                    }
                }
                Some("text") => self.add_content(1, block_tokens(line, b)),
                // thinking blokları çıktı olarak faturalanır ama bağlamda kalmaz
                _ => {}
            }
        }
    }

    fn on_user(&mut self, line: &[u8], msg: Span) {
        let Some(content) = json::member(line, msg, "content") else {
            return;
        };
        if !json::is_array(line, content) {
            self.add_content(1, text_tokens(content));
            return;
        }
        for b in json::elements(line, content) {
            if str_member(line, b, "type").as_deref() == Some("tool_result") {
                let g = str_member(line, b, "tool_use_id")
                    .and_then(|tid| self.tool_groups.get(&tid).copied())
                    .unwrap_or(1);
                if let Some(c) = json::member(line, b, "content") {
                    self.add_content(g, content_tokens(line, c));
                }
            } else {
                self.add_content(1, block_tokens(line, b));
            }
        }
    }

    fn flush(&mut self) {
        let Some(mut p) = self.pending.take() else {
            return;
        };
        let context = p.input + p.cache_read + p.cache_write;
        let base = *self
            .base
            .get_or_insert((context - p.content_total).max(0.0));
        let eff_in =
            p.input + CACHE_READ_WEIGHT * p.cache_read + CACHE_WRITE_WEIGHT * p.cache_write;
        let eff_out = OUTPUT_WEIGHT * p.output;

        p.weights.resize(self.names.len(), 0.0);
        p.weights[0] += base;
        let sum: f64 = p.weights.iter().sum();
        if sum > 0.0 {
            for (c, w) in self.cost.iter_mut().zip(&p.weights) {
                *c += eff_in * w / sum;
            }
        } else {
            self.cost[0] += eff_in;
        }
        if p.out_groups.is_empty() {
            self.cost[1] += eff_out;
        } else {
            let part = eff_out / p.out_groups.len() as f64;
            for &g in &p.out_groups {
                self.cost[g] += part;
            }
        }
        self.meta.request_count += 1;
        self.meta.total_effective += eff_in + eff_out;
    }

    /// Bir subagent dökümünün tüm maliyetini "Subagent'lar"a yazar.
    pub fn add_subagent(&mut self, sub: &Session) {
        let g = self.group_index(SUBAGENTS);
        self.cost[g] += sub.total_effective;
        self.meta.total_effective += sub.total_effective;
    }

    pub fn finish(mut self) -> Session {
        self.flush();
        let mut r = self.meta;
        let total = r.total_effective;
        r.groups = self
            .names
            .into_iter()
            .zip(self.cost)
            .filter(|(_, c)| *c > 0.0)
            .map(|(name, c)| GroupCost {
                name,
                effective: c,
                share: if total > 0.0 { c / total } else { 0.0 },
            })
            .collect();
        r.groups.sort_by(|a, b| b.effective.total_cmp(&a.effective)); // kararlı sıralama
        r
    }
}

fn iso8601_utc(ts: i64) -> String {
    let (y, m, d) = civil(ts.max(0) as u64 / 86_400);
    let rem = ts.max(0) % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Oturum özeti (`<log>_usage.json`; yerel dökümü olmayan bulut oturumları ve önbellek):
/// `{"session","proje","baslangic","bitis","istek_sayisi","toplam_efektif","gruplar":[{"ad","efektif","pay"}]}`
pub fn to_summary_json(r: &Session) -> String {
    let time = |t: Option<i64>| t.map_or("null".to_string(), |t| json::quote(&iso8601_utc(t)));
    let groups: Vec<String> = r
        .groups
        .iter()
        .map(|g| {
            format!(
                "{{\"ad\":{},\"efektif\":{:.1},\"pay\":{:.4}}}",
                json::quote(&g.name),
                g.effective,
                g.share
            )
        })
        .collect();
    format!(
        "{{\"session\":{},\"proje\":{},\"baslangic\":{},\"bitis\":{},\"istek_sayisi\":{},\"toplam_efektif\":{:.1},\"gruplar\":[{}]}}",
        json::quote(&r.session_id),
        json::quote(&r.project),
        time(r.started_at),
        time(r.ended_at),
        r.request_count,
        r.total_effective,
        groups.join(",")
    )
}

pub fn parse_summary_json(doc: &[u8]) -> Option<Session> {
    let root = json::root(doc).filter(|&r| json::is_object(doc, r))?;
    let groups = json::member(doc, root, "gruplar").filter(|&g| json::is_array(doc, g))?;
    let time = |key| str_member(doc, root, key).and_then(|s| parse_iso8601(&s));
    Some(Session {
        session_id: str_member(doc, root, "session").unwrap_or_default(),
        project: str_member(doc, root, "proje").unwrap_or_default(),
        started_at: time("baslangic"),
        ended_at: time("bitis"),
        request_count: num_member(doc, root, "istek_sayisi").max(0.0) as usize,
        total_effective: num_member(doc, root, "toplam_efektif"),
        groups: json::elements(doc, groups)
            .map(|g| GroupCost {
                name: str_member(doc, g, "ad").unwrap_or_default(),
                effective: num_member(doc, g, "efektif"),
                share: num_member(doc, g, "pay"),
            })
            .filter(|g| !g.name.is_empty())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Session {
        let mut a = Analyzer::default();
        for l in include_str!("../fixtures/session_sample.jsonl").lines() {
            a.feed_line(l.as_bytes());
        }
        a.finish()
    }

    fn cost_of(r: &Session, name: &str) -> f64 {
        r.groups
            .iter()
            .find(|g| g.name == name)
            .map_or(0.0, |g| g.effective)
    }

    fn near(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn groups_tools_by_name() {
        for (tool, group) in [
            ("Read", FILES),
            ("Bash", FILES),
            ("mcp__remote-devices__device_stage_files", FILES),
            ("WebFetch", WEB),
            ("Skill", SKILLS),
            ("ToolSearch", TOOL_LOAD),
            ("Agent", SUBAGENTS),
            ("mcp__claude-in-chrome__computer", CHROME),
            ("mcp__remote-devices__Claude_Browser__computer", BROWSER),
            ("mcp__remote-devices__computer_screenshot", SCREEN),
            ("mcp__memory__memory_read", "Bağlantı: memory"),
            (
                "mcp__0f3c9a1b2c3d4e5f6a7b__search_issues",
                "Bağlantı: search_issues",
            ),
            ("SomethingElse", CONVERSATION),
        ] {
            assert_eq!(group_for_tool(tool), group, "{tool}");
        }
    }

    #[test]
    fn counts_each_request_once() {
        let r = fixture();
        assert_eq!(r.session_id, "s-123");
        assert_eq!(r.request_count, 3);
        // m1: 10 + 2×1000 + 5×20 = 2110 · m2: 5 + 0,1×1000 + 2×200 + 5×50 = 755 · m3: 183
        assert!(near(r.total_effective, 3048.0, 1e-9));
        assert_eq!(r.started_at, parse_iso8601("2026-10-30T18:00:00Z"));
        assert_eq!(r.ended_at, parse_iso8601("2026-10-30T18:05:00Z"));
    }

    #[test]
    fn attributes_input_and_output() {
        let r = fixture();
        // İçerik (bayt/4): user "abcdefgh" 2,5 + text "ok" 1 → konuşma 3,5
        //   Read input {"p":"x"} 2,25 + sonuç metni 10,5 + görsel 800 → dosya 812,75
        //   chrome input {} 0,5 + sonuç "done" 1,5 → 2,0
        // Taban = ilk bağlam 1010 − 2,5 = 1007,5
        let base = 1007.5;
        let w2 = base + 3.5 + 812.75;
        let w3 = w2 + 2.0;
        let instr = 2010.0 * base / 1010.0 + 505.0 * base / w2 + 133.0 * base / w3;
        let conv = 2010.0 * 2.5 / 1010.0 + 505.0 * 3.5 / w2 + 133.0 * 3.5 / w3 + 50.0;
        let files = 100.0 + 505.0 * 812.75 / w2 + 133.0 * 812.75 / w3;
        let chrome = 250.0 + 133.0 * 2.0 / w3;
        assert!(near(cost_of(&r, INSTRUCTIONS), instr, 1e-6));
        assert!(near(cost_of(&r, CONVERSATION), conv, 1e-6));
        assert!(near(cost_of(&r, FILES), files, 1e-6));
        assert!(near(cost_of(&r, CHROME), chrome, 1e-6));
        assert_eq!(r.groups.len(), 4);
        assert_eq!(r.groups[0].name, INSTRUCTIONS);
        assert!(near(r.groups.iter().map(|g| g.share).sum(), 1.0, 1e-9));
    }

    #[test]
    fn compaction_resets_content() {
        let mut a = Analyzer::default();
        for l in [
            r#"{"type":"user","message":{"content":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#,
            r#"{"type":"assistant","message":{"id":"a","usage":{"input_tokens":100,"output_tokens":0}}}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":"s"}}"#,
            r#"{"type":"assistant","message":{"id":"b","usage":{"input_tokens":100,"output_tokens":0}}}"#,
        ] {
            a.feed_line(l.as_bytes());
        }
        // Taban = 100 − 10 = 90. b isteği taban 90 + özet "s" (3 bayt → 0,75) görür.
        let conv = 100.0 * 10.0 / 100.0 + 100.0 * 0.75 / 90.75;
        assert!(near(cost_of(&a.finish(), CONVERSATION), conv, 1e-9));
    }

    #[test]
    fn subagents_one_group() {
        let sub = Session {
            total_effective: 500.0,
            ..Default::default()
        };
        let mut a = Analyzer::default();
        a.feed_line(br#"{"type":"assistant","message":{"id":"a","usage":{"input_tokens":100,"output_tokens":0}}}"#);
        a.add_subagent(&sub);
        let r = a.finish();
        assert!(near(r.total_effective, 600.0, 1e-9));
        assert!(near(cost_of(&r, SUBAGENTS), 500.0, 1e-9));
    }

    #[test]
    fn summary_json_roundtrip() {
        let mut r = fixture();
        r.project = "Claude Widget".into();
        let js = to_summary_json(&r);
        assert!(js.contains("\"baslangic\":\"2026-10-30T18:00:00Z\""));
        let back = parse_summary_json(js.as_bytes()).unwrap();
        assert_eq!(back.session_id, "s-123");
        assert_eq!(back.project, "Claude Widget");
        assert_eq!(back.request_count, 3);
        assert!(near(back.total_effective, 3048.0, 0.05));
        assert_eq!(back.groups.len(), r.groups.len());
        assert_eq!(back.groups[0].name, r.groups[0].name);
        assert!(near(back.groups[0].share, r.groups[0].share, 1e-4));
        assert!(parse_summary_json(b"{}").is_none());
        assert!(parse_summary_json(b"nope").is_none());
    }
}
