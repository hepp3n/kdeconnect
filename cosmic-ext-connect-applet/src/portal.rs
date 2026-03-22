// #[allow(dead_code)] = Placeholder for code that will be used once features are fully integrated

use ashpd::desktop::file_chooser::SelectedFiles;
use percent_encoding::percent_decode;
use tracing::{debug, error};

/// Convert a `file://` URI to a plain filesystem path.
/// `file:///home/user/photo.jpg` → `/home/user/photo.jpg`
fn uri_to_path(uri: &str) -> String {
    let decoded = percent_decode(uri.as_bytes())
        .decode_utf8()
        .unwrap_or_default()
        .to_string();
    decoded
        .strip_prefix("file://")
        .unwrap_or(&decoded)
        .to_string()
}

pub async fn pick_files(
    title: impl Into<String>,
    multiple: bool,
    _filters: Option<Vec<FileFilter>>,
) -> Vec<String> {
    let title_str = title.into();

    match SelectedFiles::open_file()
        .title(title_str.as_str())
        .accept_label("Select")
        .modal(true)
        .multiple(multiple)
        .send()
        .await
    {
        Ok(request) => match request.response() {
            Ok(files) => {
                let paths: Vec<String> = files
                    .uris()
                    .iter()
                    .map(|u| uri_to_path(u.as_str()))
                    .filter(|s| !s.is_empty())
                    .collect();

                debug!("Selected {} file(s): {:?}", paths.len(), paths);
                return paths;
            }
            Err(e) => {
                error!("Failed to get file picker response: {}", e);
            }
        },
        Err(e) => {
            error!("Failed to open file picker: {}", e);
        }
    }

    Vec::new()
}

/// File filter for the portal file picker (for future use)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileFilter {
    pub name: String,
    pub patterns: Vec<String>,
}

impl FileFilter {
    #[allow(dead_code)]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            patterns: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.patterns.push(pattern.into());
        self
    }

    #[allow(dead_code)]
    pub fn patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }
}

/// Open folder picker dialog for selecting a directory
#[allow(dead_code)]
pub async fn pick_folder(title: impl Into<String>) -> Option<String> {
    let title_str = title.into();

    match SelectedFiles::open_file()
        .title(title_str.as_str())
        .accept_label("Select")
        .modal(true)
        .directory(true)
        .send()
        .await
    {
        Ok(request) => match request.response() {
            Ok(files) => {
                if let Some(uri) = files.uris().first() {
                    let path = uri_to_path(uri.as_str());
                    if !path.is_empty() {
                        return Some(path);
                    }
                }
            }
            Err(e) => {
                error!("Failed to get folder picker response: {}", e);
            }
        },
        Err(e) => {
            error!("Failed to open folder picker: {}", e);
        }
    }

    None
}

/// Read clipboard content using wl-paste
pub async fn read_clipboard() -> Result<String, std::io::Error> {
    let output = tokio::process::Command::new("wl-paste").output().await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to read clipboard",
        ))
    }
}

/// Write content to the desktop clipboard using wl-copy
pub async fn write_clipboard(content: String) -> Result<(), std::io::Error> {
    use tokio::io::AsyncWriteExt as _;

    let mut child = tokio::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(content.as_bytes()).await?;
    }

    let status = child.wait().await?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "wl-copy exited with non-zero status",
        ))
    }
}