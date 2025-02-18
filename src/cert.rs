use openssl::{
    asn1::Asn1Time,
    bn::{BigNum, MsbOption},
    error::ErrorStack,
    hash::MessageDigest,
    pkey::{PKey, PKeyRef, Private},
    rsa::Rsa,
    x509::{
        extension::{
            AuthorityKeyIdentifier, KeyUsage, SubjectAlternativeName, SubjectKeyIdentifier,
        },
        X509NameBuilder, X509Ref, X509Req, X509ReqBuilder, X509VerifyResult, X509,
    },
};
use std::{fs::File, io::Write, path::Path};

pub const CERTIFICATE: &str = "root.ca";
pub const PRIVATE_KEY: &str = "privkey.pem";

/// Make a CA certificate and private key
fn mk_ca_cert(org: &str, cn: &str) -> Result<(X509, PKey<Private>), ErrorStack> {
    let rsa = Rsa::generate(2048)?;
    let key_pair = PKey::from_rsa(rsa)?;

    let mut x509_name = X509NameBuilder::new()?;
    x509_name.append_entry_by_text("O", org)?;
    x509_name.append_entry_by_text("CN", cn)?;
    let x509_name = x509_name.build();

    let mut cert_builder = X509::builder()?;
    cert_builder.set_version(2)?;
    let serial_number = {
        let mut serial = BigNum::new()?;
        serial.rand(159, MsbOption::MAYBE_ZERO, false)?;
        serial.to_asn1_integer()?
    };
    cert_builder.set_serial_number(&serial_number)?;
    cert_builder.set_subject_name(&x509_name)?;
    cert_builder.set_issuer_name(&x509_name)?;
    cert_builder.set_pubkey(&key_pair)?;
    let not_before = Asn1Time::days_from_now(0)?;
    cert_builder.set_not_before(&not_before)?;
    let not_after = Asn1Time::days_from_now(365)?;
    cert_builder.set_not_after(&not_after)?;

    cert_builder.append_extension(
        openssl::x509::extension::BasicConstraints::new()
            .critical()
            .ca()
            .build()?,
    )?;
    cert_builder.append_extension(
        KeyUsage::new()
            .critical()
            .key_cert_sign()
            .crl_sign()
            .build()?,
    )?;

    let subject_key_identifier =
        SubjectKeyIdentifier::new().build(&cert_builder.x509v3_context(None, None))?;
    cert_builder.append_extension(subject_key_identifier)?;

    cert_builder.sign(&key_pair, MessageDigest::sha256())?;
    let cert = cert_builder.build();

    Ok((cert, key_pair))
}

/// Make a X509 request with the given private key
fn mk_request(key_pair: &PKey<Private>, org: &str, cn: &str) -> Result<X509Req, ErrorStack> {
    let mut req_builder = X509ReqBuilder::new()?;
    req_builder.set_pubkey(key_pair)?;

    let mut x509_name = X509NameBuilder::new()?;
    x509_name.append_entry_by_text("O", org)?;
    x509_name.append_entry_by_text("CN", cn)?;
    let x509_name = x509_name.build();
    req_builder.set_subject_name(&x509_name)?;

    req_builder.sign(key_pair, MessageDigest::sha256())?;
    let req = req_builder.build();
    Ok(req)
}

/// Make a certificate and private key signed by the given CA cert and private key
fn mk_ca_signed_cert(
    ca_cert: &X509Ref,
    ca_key_pair: &PKeyRef<Private>,
    org: &str,
    cn: &str,
) -> Result<(X509, PKey<Private>), ErrorStack> {
    let rsa = Rsa::generate(2048)?;
    let key_pair = PKey::from_rsa(rsa)?;

    let req = mk_request(&key_pair, org, cn)?;

    let mut cert_builder = X509::builder()?;
    cert_builder.set_version(2)?;
    let serial_number = {
        let mut serial = BigNum::new()?;
        serial.rand(159, MsbOption::MAYBE_ZERO, false)?;
        serial.to_asn1_integer()?
    };
    cert_builder.set_serial_number(&serial_number)?;
    cert_builder.set_subject_name(req.subject_name())?;
    cert_builder.set_issuer_name(ca_cert.subject_name())?;
    cert_builder.set_pubkey(&key_pair)?;
    let not_before = Asn1Time::days_from_now(0)?;
    cert_builder.set_not_before(&not_before)?;
    let not_after = Asn1Time::days_from_now(365)?;
    cert_builder.set_not_after(&not_after)?;

    cert_builder.append_extension(openssl::x509::extension::BasicConstraints::new().build()?)?;

    cert_builder.append_extension(
        KeyUsage::new()
            .critical()
            .non_repudiation()
            .digital_signature()
            .key_encipherment()
            .build()?,
    )?;

    let subject_key_identifier =
        SubjectKeyIdentifier::new().build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(subject_key_identifier)?;

    let auth_key_identifier = AuthorityKeyIdentifier::new()
        .keyid(false)
        .issuer(false)
        .build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(auth_key_identifier)?;

    let subject_alt_name = SubjectAlternativeName::new()
        .dns(org)
        .build(&cert_builder.x509v3_context(Some(ca_cert), None))?;
    cert_builder.append_extension(subject_alt_name)?;

    cert_builder.sign(ca_key_pair, MessageDigest::sha256())?;
    let cert = cert_builder.build();

    Ok((cert, key_pair))
}

pub fn generate_cert_and_keypair(org: &str, cn: &str, path: &Path) -> anyhow::Result<()> {
    let (ca_cert, ca_key_pair) = mk_ca_cert(org, cn)?;
    let (cert, key_pair) = mk_ca_signed_cert(&ca_cert, &ca_key_pair, org, cn)?;

    // Verify that this cert was issued by this ca
    match ca_cert.issued(&cert) {
        X509VerifyResult::OK => {
            let mut ca_file = File::create(path.join(CERTIFICATE))?;
            let mut key_file = File::create(path.join(PRIVATE_KEY))?;

            ca_file.write_all(&cert.to_pem()?)?;
            key_file.write_all(&key_pair.private_key_to_pem_pkcs8()?)?;
        }
        ver_err => println!("Failed to verify certificate: {}", ver_err),
    };

    Ok(())
}
