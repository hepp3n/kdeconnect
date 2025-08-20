use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn generate_device_uuid() -> String {
    uuid::Uuid::new_v4().to_string().replace("-", "")
}

pub(crate) fn default_hostname() -> String {
    if let Ok(host) = hostname::get() {
        return host.to_str().unwrap_or_default().to_owned();
    }

    String::from("localhost")
}

pub(crate) fn pair_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
