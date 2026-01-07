use crate::device::DeviceState;
use crate::event::ConnectionEvent;
use crate::plugin_interface::Plugin;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityReport {
    pub signal_strengths: HashMap<String, ConnectivityReportSignal>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityReportSignal {
    pub network_type: ConnectivityReportNetworkType,
    pub signal_strength: i32,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum ConnectivityReportNetworkType {
    #[serde(rename = "GSM")]
    Gsm,
    #[serde(rename = "CDMA")]
    Cdma,
    #[serde(rename = "iDEN")]
    Iden,
    #[serde(rename = "UMTS")]
    Umts,
    #[serde(rename = "CDMA2000")]
    Cdma2000,
    #[serde(rename = "EDGE")]
    Edge,
    #[serde(rename = "GPRS")]
    Gprs,
    #[serde(rename = "HSPA")]
    Hspa,
    #[serde(rename = "LTE")]
    Lte,
    #[serde(rename = "5G")]
    FiveG,
    #[serde(rename = "Unknown")]
    Unknown,
}

impl Display for ConnectivityReportNetworkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectivityReportNetworkType::Gsm => write!(f, "GSM"),
            ConnectivityReportNetworkType::Cdma => write!(f, "CDMA"),
            ConnectivityReportNetworkType::Iden => write!(f, "iDEN"),
            ConnectivityReportNetworkType::Umts => write!(f, "UMTS"),
            ConnectivityReportNetworkType::Cdma2000 => write!(f, "CDMA2000"),
            ConnectivityReportNetworkType::Edge => write!(f, "EDGE"),
            ConnectivityReportNetworkType::Gprs => write!(f, "GPRS"),
            ConnectivityReportNetworkType::Hspa => write!(f, "HSPA"),
            ConnectivityReportNetworkType::Lte => write!(f, "LTE"),
            ConnectivityReportNetworkType::FiveG => write!(f, "5G"),
            ConnectivityReportNetworkType::Unknown => write!(f, "Unknown"),
        }
    }
}

impl Plugin for ConnectivityReport {
    fn id(&self) -> &'static str {
        "kdeconnect.connectivity_report"
    }
}
impl ConnectivityReport {
    pub async fn received_packet(&self, event: mpsc::UnboundedSender<ConnectionEvent>) {
        self.signal_strengths.values().for_each(|v| {
            event
                .send(ConnectionEvent::StateUpdated(DeviceState::Connectivity((
                    v.network_type.to_string(),
                    v.signal_strength,
                ))))
                .unwrap();
        });
    }
}
