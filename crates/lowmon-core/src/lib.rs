//! Tüm exe'lerin paylaştığı tipler (spek §2): metrikler, kayıt kodlaması, şifreleme.
pub mod codec;
pub mod config;
pub mod crypto;
pub mod csv;
pub mod dpapi;
pub mod time;
// Eski yollar (lowmon_core::metrics / ::proto) korunur
pub use lowmon_proto::{metrics, proto};
