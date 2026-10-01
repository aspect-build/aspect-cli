//! The TLS client behind `ctx.std.net.tls.connect`.

use std::io;
use std::net::TcpStream;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::{connect_tcp, host_of, remaining, timed};

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// A client config trusting `roots`, on the same `ring` provider `ctx.http`
/// uses, so the process never depends on which provider was installed.
fn client_config(roots: RootCertStore) -> io::Result<Arc<ClientConfig>> {
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| io::Error::other(e))?
            .with_root_certificates(roots)
            .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The config trusting the operating system's certificate store, loaded once:
/// reading the store takes long enough to notice.
static NATIVE_CONFIG: LazyLock<Result<Arc<ClientConfig>, String>> = LazyLock::new(|| {
    let mut roots = RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(loaded.certs);
    client_config(roots).map_err(|e| e.to_string())
});

/// The config trusting only the certificates in `pem`.
fn pem_config(pem: &[u8]) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(pem) {
        // The PEM parser's errors carry raw bytes; the position adds nothing.
        let cert =
            cert.map_err(|_| invalid_input("ca_pem is not a valid PEM bundle".to_owned()))?;
        roots
            .add(cert)
            .map_err(|e| invalid_input(format!("ca_pem: {e}")))?;
    }
    if roots.is_empty() {
        return Err(invalid_input("ca_pem holds no certificate".to_owned()));
    }
    client_config(roots)
}

/// Connect to `addr` and finish the TLS handshake, all within `timeout`.
pub(super) fn connect(
    addr: &str,
    timeout: Option<Duration>,
    server_name: Option<&str>,
    ca_pem: Option<&[u8]>,
) -> io::Result<StreamOwned<ClientConnection, TcpStream>> {
    let config = match ca_pem {
        Some(pem) => pem_config(pem)?,
        None => NATIVE_CONFIG.clone().map_err(io::Error::other)?,
    };
    let name = server_name.unwrap_or_else(|| host_of(addr));
    let name = ServerName::try_from(name.to_owned())
        .map_err(|e| invalid_input(format!("server name {name:?}: {e}")))?;

    let deadline = timeout.map(|t| Instant::now() + t);
    let mut sock = connect_tcp(addr, timeout)?;
    let mut conn = ClientConnection::new(config, name).map_err(io::Error::other)?;
    while conn.is_handshaking() {
        if let Some(d) = deadline {
            let left = remaining(d)?;
            sock.set_read_timeout(Some(left))?;
            sock.set_write_timeout(Some(left))?;
        }
        conn.complete_io(&mut sock).map_err(timed)?;
    }
    if deadline.is_some() {
        sock.set_read_timeout(None)?;
        sock.set_write_timeout(None)?;
    }
    Ok(StreamOwned::new(conn, sock))
}
