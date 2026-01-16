use std::sync::Arc;

use rcgen::{Certificate, CertificateParams, DnType, Error, KeyPair};
use rustls::{
    ClientConfig, DigitallySignedStruct, ServerConfig,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{
        CryptoProvider, aws_lc_rs::default_provider, verify_tls12_signature, verify_tls13_signature,
    },
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use time::Duration;
use tokio::{fs, io::AsyncWriteExt};

use crate::config::CONFIG_DIR;

#[derive(Debug)]
pub struct KeyStore {
    pub client_config: Arc<ClientConfig>,
    pub server_config: Arc<ServerConfig>,
}

impl KeyStore {
    pub async fn load(device_uuid: &str) -> anyhow::Result<Self> {
        let config_dir = dirs::config_dir()
            .expect("cant find config directory")
            .join(CONFIG_DIR);

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .await
                .expect("failed to create config directory");
        }

        let cert_path = config_dir.join(format!("device_cert@{}.pem", device_uuid));
        let keys_path = config_dir.join(format!("device_key@{}.pem", device_uuid));

        if !keys_path.exists() && !cert_path.exists() {
            let key = KeyPair::generate()?.serialize_pem();
            let mut key_file = fs::File::create(&keys_path).await?;
            key_file.write_all(key.as_bytes()).await?;

            let cert = certificate_generator(&key, device_uuid)?.pem();
            let mut cert_file = fs::File::create(&cert_path).await?;
            cert_file.write_all(cert.as_bytes()).await?;
        };

        let verifier = Arc::new(NoCertificateVerification::new(default_provider()));

        // Read files asynchronously
        let cert_bytes = fs::read(&cert_path)
            .await
            .expect("reading certificate file");
        let keys_bytes = fs::read(&keys_path)
            .await
            .expect("reading private key file");

        let cert = CertificateDer::from_pem_slice(&cert_bytes).expect("decoding certfificate");
        let keys = PrivateKeyDer::from_pem_slice(&keys_bytes).expect("decoding private key");

        let client_config = Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(verifier.clone())
                .with_client_auth_cert(vec![cert.clone()], keys.clone_key())?,
        );

        let server_config = Arc::new(
            rustls::ServerConfig::builder()
                .with_client_cert_verifier(verifier.clone())
                .with_single_cert(vec![cert], keys)
                .expect("building server config"),
        );

        Ok(Self {
            client_config,
            server_config,
        })
    }
}

pub(crate) fn certificate_generator(
    keypair: &str,
    device_uuid: &str,
) -> Result<Certificate, Error> {
    if let Ok(mut params) = CertificateParams::new([device_uuid.to_string()]) {
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - Duration::days(365);
        params
            .distinguished_name
            .push(DnType::CommonName, device_uuid);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "hepp3n");
        params
            .distinguished_name
            .push(DnType::OrganizationalUnitName, "kdeconnect");

        let keypair = KeyPair::from_pem(keypair)?;

        return params.self_signed(&keypair);
    }

    Err(Error::PemError("CertificateParams failed.".into()))
}

#[derive(Debug)]
pub(crate) struct NoCertificateVerification(CryptoProvider);

impl NoCertificateVerification {
    pub fn new(provider: CryptoProvider) -> Self {
        Self(provider)
    }
}

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for NoCertificateVerification {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
}
