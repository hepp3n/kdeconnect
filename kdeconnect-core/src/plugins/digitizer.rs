use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    mem,
    os::unix::io::AsRawFd,
    sync::Mutex,
};
use tracing::{debug, warn};

use crate::plugin_interface::Plugin;

static UINPUT_DEVICE: once_cell::sync::Lazy<Mutex<Option<File>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_ABSBIT: u64 = 0x4004_5567;
const UI_SET_PROPBIT: u64 = 0x4004_556E;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;

const BTN_TOUCH: u16 = 0x14A;
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOOL_RUBBER: u16 = 0x141;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

const SYN_REPORT: i32 = 0;

const INPUT_PROP_DIRECT: u16 = 0x01;
const INPUT_PROP_POINTER: u16 = 0x00;

const UI_ABS_SETUP: u64 = 0x4004_5506;

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    ev_type: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DigitizerSession {
    pub action: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    #[serde(rename = "resolutionX")]
    pub resolution_x: Option<i32>,
    #[serde(rename = "resolutionY")]
    pub resolution_y: Option<i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DigitizerEvent {
    pub active: Option<bool>,
    pub touching: Option<bool>,
    pub tool: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub pressure: Option<f64>,
}

impl Plugin for DigitizerSession {
    fn id(&self) -> &'static str {
        "kdeconnect.digitizer.session"
    }
}

impl Plugin for DigitizerEvent {
    fn id(&self) -> &'static str {
        "kdeconnect.digitizer"
    }
}

impl DigitizerSession {
    pub async fn received_packet(&self) {
        match self.action.as_deref() {
            Some("start") => {
                let width = self.width.unwrap_or(1920);
                let height = self.height.unwrap_or(1080);
                let res_x = self.resolution_x.unwrap_or(50);
                let res_y = self.resolution_y.unwrap_or(50);

                debug!(
                    "[digitizer] starting session {}x{} (res {}x{})",
                    width, height, res_x, res_y
                );

                end_session();

                let mut guard = UINPUT_DEVICE.lock().unwrap();
                if guard.is_none() {
                    match create_uinput_device(
                        &format!("KDE Connect Digitizer"),
                        width,
                        height,
                        res_x,
                        res_y,
                    ) {
                        Ok(file) => {
                            *guard = Some(file);
                            debug!("[digitizer] uinput device created");
                        }
                        Err(e) => {
                            warn!("[digitizer] failed to create uinput device: {}", e);
                        }
                    }
                }
            }
            Some("end") => {
                debug!("[digitizer] ending session");
                end_session();
            }
            other => {
                warn!("[digitizer] unknown session action: {:?}", other);
            }
        }
    }
}

impl DigitizerEvent {
    pub async fn received_packet(&self) {
        let is_active = self.active.unwrap_or(true);

        if !is_active {
            clear_event();
            return;
        }

        let x = self.x.unwrap_or(0);
        let y = self.y.unwrap_or(0);
        let pressure = self.pressure.unwrap_or(0.0);
        let touching = self.touching.unwrap_or(false);
        let is_eraser = self.tool.as_deref() == Some("Rubber");

        write_tool_event(x, y, pressure, touching, is_eraser);
    }
}

fn end_session() {
    let mut guard = UINPUT_DEVICE.lock().unwrap();
    if let Some(file) = guard.take() {
        let fd = file.as_raw_fd();
        unsafe { libc::ioctl(fd, UI_DEV_DESTROY) };
        drop(file);
        debug!("[digitizer] uinput device destroyed");
    }
}

fn clear_event() {
    let guard = UINPUT_DEVICE.lock().unwrap();
    let Some(file) = guard.as_ref() else { return };
    let fd = file.as_raw_fd();

    write_event(fd, EV_KEY, BTN_TOUCH, 0);
    write_event(fd, EV_KEY, BTN_TOOL_PEN, 0);
    write_event(fd, EV_KEY, BTN_TOOL_RUBBER, 0);
    write_event(fd, EV_ABS, ABS_PRESSURE, 0);
    write_event(fd, EV_SYN, 0, SYN_REPORT);
}

fn write_tool_event(x: i32, y: i32, pressure: f64, touching: bool, is_eraser: bool) {
    let guard = UINPUT_DEVICE.lock().unwrap();
    let Some(file) = guard.as_ref() else { return };
    let fd = file.as_raw_fd();

    if is_eraser {
        write_event(fd, EV_KEY, BTN_TOOL_RUBBER, 1);
        write_event(fd, EV_KEY, BTN_TOOL_PEN, 0);
    } else {
        write_event(fd, EV_KEY, BTN_TOOL_PEN, 1);
        write_event(fd, EV_KEY, BTN_TOOL_RUBBER, 0);
    }

    write_event(fd, EV_KEY, BTN_TOUCH, if touching { 1 } else { 0 });

    write_event(fd, EV_ABS, ABS_X, x);
    write_event(fd, EV_ABS, ABS_Y, y);

    let p = if touching {
        (pressure.clamp(0.0, 1.0) * 1023.0) as i32
    } else {
        0
    };
    write_event(fd, EV_ABS, ABS_PRESSURE, p);

    write_event(fd, EV_SYN, 0, SYN_REPORT);
}

fn write_event(fd: i32, ev_type: u16, code: u16, value: i32) {
    let event = InputEvent {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        ev_type,
        code,
        value,
    };
    let ptr = &event as *const InputEvent as *const u8;
    let len = mem::size_of::<InputEvent>();
    unsafe {
        let buf = std::slice::from_raw_parts(ptr, len);
        let total = buf.len();
        let mut written = 0;
        while written < total {
            match libc::write(
                fd,
                buf[written..].as_ptr() as *const libc::c_void,
                total - written,
            ) {
                n if n > 0 => written += n as usize,
                -1 => {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() != Some(libc::EAGAIN)
                        && err.raw_os_error() != Some(libc::EINTR)
                    {
                        warn!("[digitizer] write event failed: {}", err);
                        return;
                    }
                }
                _ => return,
            }
        }
    }
}

fn create_uinput_device(
    name: &str,
    width: i32,
    height: i32,
    res_x: i32,
    res_y: i32,
) -> anyhow::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .map_err(|e| anyhow::anyhow!("cannot open /dev/uinput: {}", e))?;

    let fd = file.as_raw_fd();

    enable_event_type(fd, EV_KEY)?;
    enable_event_type(fd, EV_ABS)?;
    enable_event_type(fd, EV_SYN)?;

    enable_key(fd, BTN_TOUCH)?;
    enable_key(fd, BTN_TOOL_PEN)?;
    enable_key(fd, BTN_TOOL_RUBBER)?;

    enable_abs(fd, ABS_X)?;
    enable_abs(fd, ABS_Y)?;
    enable_abs(fd, ABS_PRESSURE)?;

    enable_prop(fd, INPUT_PROP_DIRECT)?;
    enable_prop(fd, INPUT_PROP_POINTER)?;

    let name_bytes = name.as_bytes();
    let mut name_arr = [0u8; 80];
    let len = name_bytes.len().min(79);
    name_arr[..len].copy_from_slice(&name_bytes[..len]);

    let dev = UinputUserDev {
        name: name_arr,
        id: InputId {
            bustype: 0x06,
            vendor: 0,
            product: 0,
            version: 0,
        },
        ff_effects_max: 0,
        absmax: {
            let mut a = [0i32; 64];
            a[ABS_X as usize] = width - 1;
            a[ABS_Y as usize] = height - 1;
            a[ABS_PRESSURE as usize] = 1023;
            a
        },
        absmin: [0i32; 64],
        absfuzz: [0i32; 64],
        absflat: [0i32; 64],
    };

    let ptr = &dev as *const UinputUserDev as *const libc::c_void;
    unsafe {
        libc::write(fd, ptr, mem::size_of::<UinputUserDev>() as libc::size_t);
        if libc::ioctl(fd, UI_DEV_CREATE) < 0 {
            return Err(anyhow::anyhow!(
                "UI_DEV_CREATE failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    set_abs_resolution(fd, ABS_X, res_x)?;
    set_abs_resolution(fd, ABS_Y, res_y)?;

    Ok(file)
}

fn set_abs_resolution(fd: i32, code: u16, resolution: i32) -> anyhow::Result<()> {
    #[repr(C)]
    struct UinputAbsSetup {
        code: u16,
        absinfo: libc::input_absinfo,
    }

    let setup = UinputAbsSetup {
        code,
        absinfo: libc::input_absinfo {
            value: 0,
            minimum: 0,
            maximum: 0,
            fuzz: 0,
            flat: 0,
            resolution,
        },
    };

    let ret = unsafe { libc::ioctl(fd, UI_ABS_SETUP, &setup as *const UinputAbsSetup) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "UI_ABS_SETUP for code {} failed: {}",
            code,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn enable_event_type(fd: i32, ev_type: u16) -> anyhow::Result<()> {
    let ret = unsafe { libc::ioctl(fd, UI_SET_EVBIT, ev_type as u64) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "UI_SET_EVBIT {:?} failed: {}",
            ev_type,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn enable_key(fd: i32, key: u16) -> anyhow::Result<()> {
    let ret = unsafe { libc::ioctl(fd, UI_SET_KEYBIT, key as u64) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "UI_SET_KEYBIT {:?} failed: {}",
            key,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn enable_abs(fd: i32, abs: u16) -> anyhow::Result<()> {
    let ret = unsafe { libc::ioctl(fd, UI_SET_ABSBIT, abs as u64) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "UI_SET_ABSBIT {:?} failed: {}",
            abs,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn enable_prop(fd: i32, prop: u16) -> anyhow::Result<()> {
    let ret = unsafe { libc::ioctl(fd, UI_SET_PROPBIT, prop as u64) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "UI_SET_PROPBIT {:?} failed: {}",
            prop,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_start() {
        let session: DigitizerSession = serde_json::from_value(serde_json::json!({
            "action": "start",
            "width": 1920,
            "height": 1080,
            "resolutionX": 50,
            "resolutionY": 50
        }))
        .unwrap();

        assert_eq!(session.action.as_deref(), Some("start"));
        assert_eq!(session.width, Some(1920));
        assert_eq!(session.height, Some(1080));
        assert_eq!(session.resolution_x, Some(50));
        assert_eq!(session.resolution_y, Some(50));
    }

    #[test]
    fn parses_session_end() {
        let session: DigitizerSession =
            serde_json::from_value(serde_json::json!({"action": "end"})).unwrap();

        assert_eq!(session.action.as_deref(), Some("end"));
    }

    #[test]
    fn parses_tool_event_pen_touching() {
        let event: DigitizerEvent = serde_json::from_value(serde_json::json!({
            "active": true,
            "touching": true,
            "tool": "Pen",
            "x": 500,
            "y": 300,
            "pressure": 0.75
        }))
        .unwrap();

        assert_eq!(event.active, Some(true));
        assert_eq!(event.touching, Some(true));
        assert_eq!(event.tool.as_deref(), Some("Pen"));
        assert_eq!(event.x, Some(500));
        assert_eq!(event.y, Some(300));
        assert_eq!(event.pressure, Some(0.75));
    }

    #[test]
    fn parses_tool_event_eraser_hover() {
        let event: DigitizerEvent = serde_json::from_value(serde_json::json!({
            "active": true,
            "tool": "Rubber",
            "x": 100,
            "y": 200
        }))
        .unwrap();

        assert_eq!(event.tool.as_deref(), Some("Rubber"));
        assert_eq!(event.touching, None);
        assert_eq!(event.pressure, None);
    }

    #[test]
    fn parses_tool_event_inactive() {
        let event: DigitizerEvent =
            serde_json::from_value(serde_json::json!({"active": false})).unwrap();

        assert_eq!(event.active, Some(false));
    }

    #[test]
    fn received_packet_returns_immediately() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let start_session = DigitizerSession {
                action: Some("start".to_string()),
                width: Some(1920),
                height: Some(1080),
                ..Default::default()
            };

            let ok =
                tokio::time::timeout(Duration::from_millis(500), start_session.received_packet())
                    .await
                    .is_ok();

            let end_session = DigitizerSession {
                action: Some("end".to_string()),
                ..Default::default()
            };
            end_session.received_packet().await;

            ok
        });

        rt.shutdown_background();
        assert!(passed);
    }

    #[test]
    fn session_end_cleans_up_immediately() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let end_session = DigitizerSession {
                action: Some("end".to_string()),
                ..Default::default()
            };

            tokio::time::timeout(Duration::from_millis(500), end_session.received_packet())
                .await
                .is_ok()
        });

        rt.shutdown_background();
        assert!(passed);
    }

    #[test]
    fn digitizer_event_received_packet_returns_immediately() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let event = DigitizerEvent {
                active: Some(true),
                touching: Some(true),
                tool: Some("Pen".to_string()),
                x: Some(500),
                y: Some(300),
                pressure: Some(0.5),
            };

            tokio::time::timeout(Duration::from_millis(500), event.received_packet())
                .await
                .is_ok()
        });

        rt.shutdown_background();
        assert!(passed);
    }
}
