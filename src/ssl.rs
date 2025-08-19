use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

use rcgen::{Certificate, CertificateParams, DnType, Error, KeyPair};
use time::Duration;
use tracing::info;

use crate::config::{CERTIFICATE, CONFIG_DIR, KEY_PAIR};

pub(crate) fn certificate_generator(
    keypair: &KeyPair,
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

        return params.self_signed(keypair);
    }

    Err(Error::PemError("CertificateParams failed.".into()))
}

pub(crate) fn store_certificate_files(
    cert: &Certificate,
    cert_path: &Path,
    keys: &KeyPair,
    keys_path: &Path,
) -> io::Result<()> {
    let cert_data = cert.pem();
    let keys_data = keys.serialize_pem();

    match File::create(cert_path) {
        Ok(mut f) => f.write_all(cert_data.as_bytes()).unwrap(),
        Err(e) => info!("{e:?}"),
    }

    match File::create(keys_path) {
        Ok(mut f) => f.write_all(keys_data.as_bytes()).unwrap(),
        Err(e) => info!("{e:?}"),
    }

    Ok(())
}

pub(crate) fn read_certificate() -> Vec<u8> {
    let path = dirs::config_dir()
        .expect("cant find config directory")
        .join(CONFIG_DIR)
        .join(CERTIFICATE);

    let mut buffer = Vec::new();

    match File::open(path) {
        Ok(mut f) => {
            f.read_to_end(&mut buffer).unwrap();
        }
        Err(e) => info!("{e:?}"),
    };

    buffer
}

pub(crate) fn read_keypair() -> Vec<u8> {
    let path = dirs::config_dir()
        .expect("cant find config directory")
        .join(CONFIG_DIR)
        .join(KEY_PAIR);

    let mut buffer = Vec::new();

    match File::open(path) {
        Ok(mut f) => {
            f.read_to_end(&mut buffer).unwrap();
        }
        Err(e) => info!("{e:?}"),
    };

    buffer
}
