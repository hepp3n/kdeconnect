// cosmic-connect-applet/src/portal.rs
// #[allow(dead_code)] = Placeholder for code that will be used once features are fully integrated

use ashpd::desktop::file_chooser::SelectedFiles;
use percent_encoding::percent_decode;

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
        Ok(request) => {
            match request.response() {
                Ok(files) => {
                    let paths: Vec<String> = files
                        .uris()
                        .iter()
                        .map(|u| {
                            percent_decode(u.path().as_bytes())
                                .decode_utf8()
                                .unwrap_or_default()
                                .to_string()
                        })
                        .filter(|s| !s.is_empty())
                        .collect();
                    
                    eprintln!("Selected {} file(s)", paths.len());
                    return paths;
                }
                Err(e) => {
                    eprintln!("Failed to get file picker response: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to open file picker: {}", e);
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
        Ok(request) => {
            match request.response() {
                Ok(files) => {
                    if let Some(uri) = files.uris().first() {
                        let path = percent_decode(uri.path().as_bytes())
                            .decode_utf8()
                            .unwrap_or_default()
                            .to_string();
                        
                        if !path.is_empty() {
                            return Some(path);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to get folder picker response: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to open folder picker: {}", e);
        }
    }
    
    None
}

/// Read clipboard content using wl-paste
pub async fn read_clipboard() -> Result<String, std::io::Error> {
    let output = tokio::process::Command::new("wl-paste")
        .output()
        .await?;
    
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to read clipboard"
        ))
    }
}