//! Segment başlığı, blok şifreleme ve hash zinciri (spek §5.1–5.3).
//!
//! Zincir: blok i'nin AAD'si = `önceki || i (u64 LE)`; önceki = blok 0 için 64 B başlık, sonra bir önceki tag.
//! Böylece başlık değişimi, blok silme / yer değiştirme / araya ekleme ve bit flip doğrulamada yakalanır.
//! Bilinen sınır: mühürsüz segmentin *son* blokları kesilirse tek başına anlaşılmaz (spek §5.6, state.bin + Seal).
// shortcut: anahtar baytları bellekte sıfırlanmıyor (zeroize yok); servis tek süreç, M2 sonrası gözden geçir
use chacha20poly1305::{
    AeadInOut, Key, KeyInit, Tag, XChaCha20Poly1305, XNonce, aead::inout::InOutBuf,
};
use hkdf::Hkdf;
use sha2::Sha256;

pub const MAGIC: [u8; 4] = *b"LWMN";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Blok uzunluk alanı üst sınırı; bozuk `len` ile dev ayırma olmasın.
pub const MAX_BLOCK: usize = 1 << 20;

pub type MasterKey = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    BadHeader,
    /// Dosya bir bloğun ortasında bitiyor (çökme sırasında yarım yazma).
    Truncated {
        block: u64,
    },
    /// Tag doğrulanmadı: kurcalama, yanlış anahtar ya da zincir kopukluğu.
    Auth {
        block: u64,
    },
    Rng,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub flags: u16,
    pub segment_id: u64,
    pub created_unix: u64,
    pub interval_ms: u32,
    pub schema_hash: u32,
    pub salt: [u8; 16],
}

impl Header {
    /// Yeni segment başlığı; salt rastgele.
    pub fn new(
        segment_id: u64,
        created_unix: u64,
        interval_ms: u32,
        schema_hash: u32,
    ) -> Result<Self, CryptoError> {
        let mut salt = [0; 16];
        getrandom::fill(&mut salt).map_err(|_| CryptoError::Rng)?;
        Ok(Self {
            flags: 0,
            segment_id,
            created_unix,
            interval_ms,
            schema_hash,
            salt,
        })
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..6].copy_from_slice(&VERSION.to_le_bytes());
        b[6..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.segment_id.to_le_bytes());
        b[16..24].copy_from_slice(&self.created_unix.to_le_bytes());
        b[24..28].copy_from_slice(&self.interval_ms.to_le_bytes());
        b[28..32].copy_from_slice(&self.schema_hash.to_le_bytes());
        b[32..48].copy_from_slice(&self.salt);
        b // 48..64 reserved = 0
    }

    pub fn parse(b: &[u8]) -> Result<Self, CryptoError> {
        let b: &[u8; HEADER_LEN] = b
            .get(..HEADER_LEN)
            .and_then(|s| s.try_into().ok())
            .ok_or(CryptoError::BadHeader)?;
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        if b[0..4] != MAGIC || u16_at(4) != VERSION {
            return Err(CryptoError::BadHeader);
        }
        Ok(Self {
            flags: u16_at(6),
            segment_id: u64_at(8),
            created_unix: u64_at(16),
            interval_ms: u32_at(24),
            schema_hash: u32_at(28),
            salt: b[32..48].try_into().unwrap(),
        })
    }
}

fn segment_cipher(master: &MasterKey, salt: &[u8; 16]) -> XChaCha20Poly1305 {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), master)
        .expand(b"lwm-seg-v1", &mut okm)
        .expect("32 B HKDF çıktısı her zaman geçerli");
    XChaCha20Poly1305::new(&Key::from(okm))
}

/// Zincir durumu; yazıcı ve okuyucu aynı kuralı paylaşır.
struct Chain {
    cipher: XChaCha20Poly1305,
    prev: Vec<u8>, // blok 0: başlık baytları, sonra son tag
    index: u64,
    aad: Vec<u8>,
}

impl Chain {
    /// `raw`: dosyadaki başlık baytları aynen (reserved dahil) → her başlık biti AAD'de.
    fn new(master: &MasterKey, salt: &[u8; 16], raw: &[u8]) -> Self {
        Self {
            cipher: segment_cipher(master, salt),
            prev: raw.to_vec(),
            index: 0,
            aad: Vec::with_capacity(HEADER_LEN + 8),
        }
    }

    fn aad(&mut self) -> &[u8] {
        self.aad.clear();
        self.aad.extend_from_slice(&self.prev);
        self.aad.extend_from_slice(&self.index.to_le_bytes());
        &self.aad
    }

    fn advance(&mut self, tag: &[u8; TAG_LEN]) {
        self.prev.clear();
        self.prev.extend_from_slice(tag);
        self.index += 1;
    }
}

/// Yazıcı tarafı: düz metin bloğu → `len | nonce | ciphertext | tag`.
pub struct Sealer {
    chain: Chain,
}

impl Sealer {
    pub fn new(master: &MasterKey, header: &Header) -> Self {
        Self {
            chain: Chain::new(master, &header.salt, &header.to_bytes()),
        }
    }

    /// Çerçeveli bloğu `out`'a ekler.
    pub fn seal(&mut self, plain: &[u8], out: &mut Vec<u8>) -> Result<(), CryptoError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| CryptoError::Rng)?;
        let len = (NONCE_LEN + plain.len() + TAG_LEN) as u32;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nonce);
        let start = out.len();
        out.extend_from_slice(plain);
        let nonce = XNonce::from(nonce);
        let aad = self.chain.aad().to_vec();
        let tag: [u8; TAG_LEN] = self
            .chain
            .cipher
            .encrypt_inout_detached(&nonce, &aad, InOutBuf::from(&mut out[start..]))
            .expect("1 MB altı blokta XChaCha20-Poly1305 hata vermez")
            .into();
        out.extend_from_slice(&tag);
        self.chain.advance(&tag);
        Ok(())
    }

    /// state.bin için zincir başı: (yazılan blok sayısı, son tag ya da başlık).
    pub fn head(&self) -> (u64, &[u8]) {
        (self.chain.index, &self.chain.prev)
    }
}

/// Okuyucu: segment dosyasını baştan sona doğrular ve blokları çözer.
pub struct Opener<'a> {
    chain: Chain,
    rest: &'a [u8],
    good_end: usize,
}

impl<'a> Opener<'a> {
    pub fn new(master: &MasterKey, file: &'a [u8]) -> Result<(Header, Self), CryptoError> {
        let header = Header::parse(file)?;
        let opener = Self {
            chain: Chain::new(master, &header.salt, &file[..HEADER_LEN]),
            rest: &file[HEADER_LEN..],
            good_end: HEADER_LEN,
        };
        Ok((header, opener))
    }

    /// Doğrulanan blok sayısı ve zincir başı.
    pub fn head(&self) -> (u64, &[u8]) {
        (self.chain.index, &self.chain.prev)
    }

    /// Son doğrulanmış bloğun bittiği dosya ofseti (yarım kuyruk buraya kırpılır).
    pub fn good_end(&self) -> usize {
        self.good_end
    }

    /// Doğrulanmış zincirin sonundan yazmaya devam et.
    pub fn into_sealer(self) -> Sealer {
        Sealer { chain: self.chain }
    }
}

impl Iterator for Opener<'_> {
    type Item = Result<Vec<u8>, CryptoError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let block = self.chain.index;
        let r = (|| {
            let truncated = CryptoError::Truncated { block };
            let len = u32::from_le_bytes(self.rest.get(..4).ok_or(truncated)?.try_into().unwrap())
                as usize;
            if !(NONCE_LEN + TAG_LEN..=MAX_BLOCK).contains(&len) {
                return Err(CryptoError::Auth { block }); // bozuk uzunluk alanı
            }
            let body = self.rest.get(4..4 + len).ok_or(truncated)?;
            let (nonce, rest) = body.split_at(NONCE_LEN);
            let (ct, tag) = rest.split_at(rest.len() - TAG_LEN);
            let mut plain = ct.to_vec();
            let tag: [u8; TAG_LEN] = tag.try_into().unwrap();
            let nonce = XNonce::from(<[u8; NONCE_LEN]>::try_from(nonce).unwrap());
            let aad = self.chain.aad().to_vec();
            self.chain
                .cipher
                .decrypt_inout_detached(
                    &nonce,
                    &aad,
                    InOutBuf::from(plain.as_mut_slice()),
                    &Tag::from(tag),
                )
                .map_err(|_| CryptoError::Auth { block })?;
            self.chain.advance(&tag);
            self.rest = &self.rest[4 + len..];
            self.good_end += 4 + len;
            Ok(plain)
        })();
        if r.is_err() {
            self.rest = &[]; // ilk hatadan sonra zincire güvenilmez
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: MasterKey = [7; 32];

    /// 3 bloklu segment + blok sınırları (dosya içi ofset aralıkları).
    fn segment() -> (Vec<u8>, Vec<std::ops::Range<usize>>) {
        let h = Header::new(1, 1_763_128_800, 1000, 0xABCD).unwrap();
        let mut file = h.to_bytes().to_vec();
        let mut s = Sealer::new(&KEY, &h);
        let mut ranges = Vec::new();
        for i in 0..3u8 {
            let start = file.len();
            s.seal(&[i; 100], &mut file).unwrap();
            ranges.push(start..file.len());
        }
        (file, ranges)
    }

    fn open_all(key: &MasterKey, file: &[u8]) -> Result<Vec<Vec<u8>>, CryptoError> {
        let (_, o) = Opener::new(key, file)?;
        o.collect()
    }

    #[test]
    fn round_trip() {
        let (file, _) = segment();
        let blocks = open_all(&KEY, &file).unwrap();
        assert_eq!(blocks, vec![vec![0; 100], vec![1; 100], vec![2; 100]]);
    }

    #[test]
    fn header_round_trip_and_magic() {
        let h = Header::new(42, 5, 1000, 9).unwrap();
        assert_eq!(Header::parse(&h.to_bytes()), Ok(h));
        let mut b = h.to_bytes();
        b[0] = b'X';
        assert_eq!(Header::parse(&b), Err(CryptoError::BadHeader));
    }

    #[test]
    fn every_bit_flip_is_detected() {
        let (file, _) = segment();
        // Başlık magic/version dışındaki her bayt ve tüm bloklar: tek bit değişimi → hata
        for i in 0..file.len() {
            let mut f = file.clone();
            f[i] ^= 0x01;
            assert!(open_all(&KEY, &f).is_err(), "bayt {i} değişimi yakalanmadı");
        }
    }

    #[test]
    fn deleting_middle_block_is_detected() {
        let (file, r) = segment();
        let f = [&file[..r[1].start], &file[r[1].end..]].concat();
        assert_eq!(open_all(&KEY, &f), Err(CryptoError::Auth { block: 1 }));
    }

    #[test]
    fn deleting_first_block_is_detected() {
        let (file, r) = segment();
        let f = [&file[..r[0].start], &file[r[0].end..]].concat();
        assert_eq!(open_all(&KEY, &f), Err(CryptoError::Auth { block: 0 }));
    }

    #[test]
    fn swapping_blocks_is_detected() {
        let (file, r) = segment();
        let f = [
            &file[..r[0].start],
            &file[r[1].clone()],
            &file[r[0].clone()],
            &file[r[2].clone()],
        ]
        .concat();
        assert_eq!(open_all(&KEY, &f), Err(CryptoError::Auth { block: 0 }));
    }

    #[test]
    fn duplicating_block_is_detected() {
        let (file, r) = segment();
        let f = [&file[..r[1].end], &file[r[1].clone()], &file[r[2].clone()]].concat();
        assert_eq!(open_all(&KEY, &f), Err(CryptoError::Auth { block: 2 }));
    }

    #[test]
    fn header_swap_between_segments_is_detected() {
        let (a, _) = segment();
        let (b, _) = segment();
        let f = [&b[..HEADER_LEN], &a[HEADER_LEN..]].concat();
        assert!(open_all(&KEY, &f).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let (file, _) = segment();
        assert_eq!(
            open_all(&[8; 32], &file),
            Err(CryptoError::Auth { block: 0 })
        );
    }

    #[test]
    fn torn_tail_reports_truncated() {
        let (file, r) = segment();
        let f = &file[..r[2].end - 5];
        let (_, o) = Opener::new(&KEY, f).unwrap();
        let res: Vec<_> = o.collect();
        assert_eq!(res.len(), 3);
        assert_eq!(res[2], Err(CryptoError::Truncated { block: 2 }));
    }

    #[test]
    fn resume_continues_chain() {
        let (mut file, _) = segment();
        let (_, mut o) = Opener::new(&KEY, &file).unwrap();
        while o.next().is_some() {}
        assert_eq!(o.good_end(), file.len());
        let mut s = o.into_sealer();
        s.seal(&[9; 10], &mut file).unwrap();
        assert_eq!(open_all(&KEY, &file).unwrap().len(), 4);
    }

    /// Bilinen sınır (spek §5.6): tam blok sınırında kuyruk kesimi tek başına fark edilmez.
    #[test]
    fn tail_cut_at_block_boundary_is_not_self_evident() {
        let (file, r) = segment();
        assert_eq!(open_all(&KEY, &file[..r[2].start]).unwrap().len(), 2);
    }
}
