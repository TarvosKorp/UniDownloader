use module_api::{DownloaderModule, DownloadOptions};

pub struct UdioModule;

impl DownloaderModule for UdioModule {
    fn name(&self) -> &'static str {
        "Udio"
    }

    fn id(&self) -> &'static str {
        "udio"
    }

    fn download(&self, _url: &str, _options: &DownloadOptions) -> Result<(), String> {
        // Заглушка. Позже здесь будет вызов Python-скрипта или логика на Rust.
        Err("Udio module not implemented yet".to_string())
    }
}