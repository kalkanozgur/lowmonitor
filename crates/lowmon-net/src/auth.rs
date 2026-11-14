//! Eşleştirme + token deposu + deneme koruması (spek §16.2, ADR-0024).
//! Dosyalar `%APPDATA%\LowMonitor\` altında, DPAPI kullanıcı kapsamı:
//! - `pair.bin`  = SHA-256(kod) ‖ bitiş ms (u64 LE); tek kullanımlık, 120 s.
//! - `tokens.bin` = SHA-256(token) dizisi; düz token hiçbir yerde saklanmaz.
use lowmon_core::dpapi::dpapi;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs, io,
    net::IpAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

pub type Hash = [u8; 32];

pub const PAIR_TTL_MS: u64 = 120_000;
const THROTTLE: Duration = Duration::from_secs(1);
const BAN_AFTER: u32 = 10;
const BAN: Duration = Duration::from_secs(600);

pub fn sha(b: &[u8]) -> Hash {
    Sha256::digest(b).into()
}

/// Uzunluk sabitken içerikten bağımsız süre (kod/token karşılaştırması).
fn ct_eq(a: &Hash, b: &Hash) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(b)
}

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.dir.join(name)) {
            Ok(b) => dpapi(&b, false).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, name: &str, data: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        // shortcut: atomik değil (tmp + rename yok); yarım yazılmış dosya DPAPI'de reddedilir → token'lar yeniden eşleştirilir
        fs::write(self.dir.join(name), dpapi(data, true)?)
    }

    /// Yeni 6 haneli kod; öncekini geçersiz kılar.
    pub fn start_pair(&self, now_ms: u64) -> io::Result<String> {
        // shortcut: u32 % 10^6 modulo yanlılığı ~%0.02; 120 s + IP kısıtı altında önemsiz
        let code = format!("{:06}", u32::from_le_bytes(random()?) % 1_000_000);
        let mut rec = sha(code.as_bytes()).to_vec();
        rec.extend_from_slice(&(now_ms + PAIR_TTL_MS).to_le_bytes());
        self.write("pair.bin", &rec)?;
        Ok(code)
    }

    /// Doğru ve süresi geçmemiş kod → yeni token (hex, 256 bit). Kod tek kullanımlık.
    pub fn redeem(&self, code: &str, now_ms: u64) -> io::Result<Option<String>> {
        let Some(rec) = self.read("pair.bin")? else {
            return Ok(None);
        };
        let (Ok(h), Ok(exp)) = (
            Hash::try_from(rec.get(..32).unwrap_or_default()),
            <[u8; 8]>::try_from(rec.get(32..40).unwrap_or_default()),
        ) else {
            return Ok(None);
        };
        if now_ms >= u64::from_le_bytes(exp) {
            let _ = fs::remove_file(self.dir.join("pair.bin"));
            return Ok(None);
        }
        if !ct_eq(&h, &sha(code.as_bytes())) {
            return Ok(None);
        }
        fs::remove_file(self.dir.join("pair.bin"))?;
        let token: String = random::<32>()?.iter().map(|b| format!("{b:02x}")).collect();
        let mut all = self.read("tokens.bin")?.unwrap_or_default();
        all.extend_from_slice(&sha(token.as_bytes()));
        self.write("tokens.bin", &all)?;
        Ok(Some(token))
    }

    /// Kayıtlı token hash'leri; dosya yoksa boş. Her el sıkışmada okunur → `unpair` hemen etkili.
    pub fn hashes(&self) -> io::Result<Vec<Hash>> {
        let raw = self.read("tokens.bin")?.unwrap_or_default();
        Ok(raw.as_chunks::<32>().0.to_vec())
    }

    pub fn check(&self, token: &str) -> Option<Hash> {
        let h = sha(token.as_bytes());
        self.hashes()
            .ok()?
            .iter()
            .any(|k| ct_eq(k, &h))
            .then_some(h)
    }

    pub fn clear(&self) -> io::Result<()> {
        for f in ["tokens.bin", "pair.bin"] {
            match fs::remove_file(self.dir.join(f)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Entry {
    fails: u32,
    last_fail: Option<Instant>,
    banned_until: Option<Instant>,
}

/// IP başına: hatalı denemeden sonra 1 s yeni bağlantı yok, 10 hata → 10 dk ban.
#[derive(Default)]
pub struct Guard(HashMap<IpAddr, Entry>);

impl Guard {
    pub fn allowed(&self, ip: IpAddr, now: Instant) -> bool {
        self.0.get(&ip).is_none_or(|e| {
            e.banned_until.is_none_or(|t| now >= t)
                && e.last_fail.is_none_or(|t| now >= t + THROTTLE)
        })
    }

    pub fn fail(&mut self, ip: IpAddr, now: Instant) {
        // Süresi dolan kayıtlar atılır: harita LAN'daki aktif saldırganlarla sınırlı kalır
        self.0.retain(|_, e| {
            e.banned_until.is_some_and(|t| now < t) || e.last_fail.is_some_and(|t| now < t + BAN)
        });
        let e = self.0.entry(ip).or_default();
        e.fails += 1;
        e.last_fail = Some(now);
        if e.fails >= BAN_AFTER {
            e.fails = 0;
            e.banned_until = Some(now + BAN);
        }
    }

    pub fn ok(&mut self, ip: IpAddr) {
        self.0.remove(&ip);
    }
}

#[cfg(test)]
pub fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("lowmon-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    Store { dir }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_once_expire_and_clear() {
        let s = temp_store("auth");
        assert_eq!(s.redeem("000000", 0).unwrap(), None, "kod yokken");
        let code = s.start_pair(1_000).unwrap();
        assert_eq!(code.len(), 6);
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert_eq!(s.redeem(wrong, 2_000).unwrap(), None);
        let tok = s.redeem(&code, 2_000).unwrap().expect("doğru kod");
        assert_eq!(tok.len(), 64);
        assert_eq!(s.redeem(&code, 2_000).unwrap(), None, "tek kullanımlık");
        assert!(s.check(&tok).is_some());
        assert!(s.check("yanlis").is_none());
        // Düz token diskte yok
        let raw = fs::read(s.dir.join("tokens.bin")).unwrap();
        assert!(!raw.windows(8).any(|w| w == &tok.as_bytes()[..8]));

        let code = s.start_pair(0).unwrap();
        assert_eq!(s.redeem(&code, PAIR_TTL_MS).unwrap(), None, "süre doldu");

        s.clear().unwrap();
        assert!(s.check(&tok).is_none(), "unpair --all");
        let _ = fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn throttle_and_ban() {
        let (mut g, ip, t0) = (
            Guard::default(),
            IpAddr::from([10, 0, 0, 2]),
            Instant::now(),
        );
        let other = IpAddr::from([10, 0, 0, 3]);
        g.fail(ip, t0);
        assert!(!g.allowed(ip, t0 + Duration::from_millis(500)));
        assert!(g.allowed(other, t0), "başka IP etkilenmez");
        assert!(g.allowed(ip, t0 + THROTTLE));
        for i in 1..BAN_AFTER {
            g.fail(ip, t0 + THROTTLE * (i + 1));
        }
        let t = t0 + THROTTLE * (BAN_AFTER + 2);
        assert!(!g.allowed(ip, t), "10 hata → ban");
        assert!(g.allowed(ip, t0 + THROTTLE * BAN_AFTER + BAN));
        g.fail(other, t0);
        g.ok(other);
        assert!(g.allowed(other, t0), "başarı sayacı sıfırlar");
    }
}
