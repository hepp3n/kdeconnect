use std::time::SystemTime;

pub fn generate_device_uuid() -> String {
    uuid::Uuid::new_v4().to_string().replace("-", "")
}

pub fn default_hostname() -> String {
    if let Ok(host) = hostname::get() {
        return host.to_str().unwrap_or_default().to_owned();
    }

    String::from("localhost")
}

pub(crate) fn packet_id() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis()
}
