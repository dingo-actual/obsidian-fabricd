//! Process-wide TLS provider installation.
//!
//! `reqwest` is built without a default TLS provider, because the default one
//! (`aws-lc-rs`) needs CMake on the targets this service ships to. The ring
//! provider costs nothing extra and builds everywhere, but it must be installed
//! once before the first HTTPS client is constructed.

/// Installs the ring provider for the whole process.
///
/// Idempotent: a second call reports that a provider is already installed and is
/// ignored, so startup and tests can both call it.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::install_crypto_provider;

    #[test]
    fn installing_twice_is_harmless() {
        install_crypto_provider();
        install_crypto_provider();
    }

    #[test]
    fn an_https_client_can_be_built_afterwards() {
        install_crypto_provider();

        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("a client with a configured TLS provider");

        drop(client);
    }
}
