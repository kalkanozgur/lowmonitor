//! LowMonitor'un dışa açık sözleşmesi (ADR-0022): pipe protokolü ve metrik tipleri.
//! Yalnız serde + postcard; Windows API, şifreleme yok → lowdeck-agent gibi dış istemciler bağımlanabilir.
pub mod metrics;
pub mod proto;
