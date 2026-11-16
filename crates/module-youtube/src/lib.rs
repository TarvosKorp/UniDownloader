use module_api::{DownloaderModule, DownloadOptions};

pub struct YouTubeModule;

impl DownloaderModule for YouTubeModule {
    fn name(&self) -> &'static str {
        "YouTube"
    }

    fn id(&self) -> &'static str {
        "youtube"
    }

    fn download(&self, _url: &str, _options: &DownloadOptions) -> Result<(), String> {
        // Заглушка. Позже здесь будет вызов yt-dlp.exe.
        Err("YouTube module not implemented yet".to_string())
    }
}