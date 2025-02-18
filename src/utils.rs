use uuid::Uuid;

pub fn generate_device_id() -> String {
    let id = Uuid::new_v4();
    format!("_{}_", id.to_string().replace("-", "_"))
}

pub fn get_default_devicename() -> String {
    if let Ok(host) = hostname::get() {
        return host.to_str().unwrap_or_default().to_owned();
    }

    String::from("localhost")
}
