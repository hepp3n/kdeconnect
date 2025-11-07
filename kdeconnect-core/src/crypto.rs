use std::{fs::File, io, path::PathBuf};

use rcgen::{Certificate, CertificateParams, DnType, Error, KeyPair};
use time::Duration;

use crate::config::CONFIG_DIR;

pub struct KeyStore {
    certificate: String,
    _cert_path: PathBuf,
    keypair: String,
    _keys_path: PathBuf,
}

impl KeyStore {
    pub fn new(device_uuid: &str) -> anyhow::Result<Self> {
        let config_dir = dirs::config_dir()
            .expect("cant find config directory")
            .join(CONFIG_DIR);

        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir).expect("failed to create config directory");
        }

        let cert_path = config_dir.join(format!("device_cert@{}.pem", device_uuid));
        let keys_path = config_dir.join(format!("device_key@{}.pem", device_uuid));

        let keypair = match keys_path.exists() {
            true => {
                let mut key_file = File::open(&keys_path)?;
                let mut key_pem = String::new();

                use io::Read as _;
                key_file.read_to_string(&mut key_pem)?;

                key_pem
            }
            false => {
                let key = KeyPair::generate()?.serialize_pem();

                let mut key_file = File::create(&keys_path)?;

                use io::Write as _;
                key_file.write_all(key.as_bytes())?;

                key
            }
        };

        let certificate = match cert_path.exists() {
            true => {
                let mut cert_file = File::open(&cert_path)?;
                let mut cert_pem = String::new();

                use io::Read as _;
                cert_file.read_to_string(&mut cert_pem)?;

                cert_pem
            }
            false => {
                let cert = certificate_generator(&keypair, device_uuid)?.pem();

                let mut cert_file = File::create(&cert_path)?;

                use io::Write as _;
                cert_file.write_all(cert.as_bytes())?;

                cert
            }
        };

        Ok(Self {
            certificate,
            _cert_path: cert_path,
            keypair,
            _keys_path: keys_path,
        })
    }

    pub fn get_certificate(&self) -> &String {
        &self.certificate
    }

    pub fn get_keypair(&self) -> &String {
        &self.keypair
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
