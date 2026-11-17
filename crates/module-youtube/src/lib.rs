use module_api::{guard, hide_console, Request};
use std::ffi::c_char;
use std::path::Path;
use std::process::Command;

static MODULE_UI_JSON: &[u8] = br#"{"name":"YouTube","url_label":"Video or playlist URL","qualities":["144p","240p","360p","480p","720p","1080p","MP3"],"default_quality":"360p","supports_bitrate":true,"supports_batch":true}"#;

#[no_mangle]
pub extern "C" fn module_ui_json() -> *const c_char { MODULE_UI_JSON.as_ptr() as *const c_char }

#[no_mangle]
pub extern "C" fn module_abi_version() -> u32 {
    module_api::ABI_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn run_download(
    url: *const c_char,
    quality: *const c_char,
    output_dir: *const c_char,
    ytdlp_path: *const c_char,
    ffmpeg_path: *const c_char,
    err_buf: *mut c_char,
    err_buf_len: usize,
) -> i32 {
    guard(err_buf, err_buf_len, || {
        let req = Request::from_raw(url, quality, output_dir, ytdlp_path, ffmpeg_path)?;
        download(&req)
    })
}

fn download(req: &Request) -> Result<(), String> {
    if !(req.url.starts_with("http://") || req.url.starts_with("https://")) {
        return Err(format!("Invalid URL: {}", req.url));
    }

    let mut cmd = Command::new(&req.ytdlp_path);
    hide_console(&mut cmd);
    cmd.arg("--no-playlist");
    cmd.arg("--ffmpeg-location").arg(&req.ffmpeg_path);

    let quality_upper = req.quality.to_uppercase();
    if quality_upper.starts_with("MP3") {
        // Парсим битрейт, если он есть (формат "MP3:192")
        let bitrate = if let Some(idx) = req.quality.find(':') {
            req.quality[idx+1..].to_string()
        } else {
            "160".to_string() // по умолчанию
        };

        cmd.arg("-x").arg("--audio-format").arg("mp3");
        cmd.arg("--audio-quality").arg(format!("{}K", bitrate));
        cmd.arg("--embed-thumbnail").arg("--embed-metadata");
        cmd.arg("--convert-thumbnails").arg("jpg");
        let crop_filter = "crop='if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'";
        cmd.arg("--ppa")
            .arg(format!("ThumbnailsConvertor+ffmpeg_o:-vf \"{}\"", crop_filter));
    } else {
        let height: u32 = req.quality
            .trim_end_matches(['p', 'P'])
            .parse()
            .map_err(|_| format!("Unsupported quality: {}", req.quality))?;
        cmd.arg("-f").arg(format!(
            "bestvideo[height<={h}]+bestaudio/best[height<={h}]",
            h = height
        ));
        cmd.arg("--merge-output-format").arg("mp4");
    }

    let template = Path::new(&req.output_dir).join("%(title)s.%(ext)s");
    cmd.arg("-o").arg(template);
    cmd.arg("--").arg(&req.url);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run yt-dlp ({}): {e}", req.ytdlp_path))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR"))
            .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
            .unwrap_or("no error output");
        let code = output.status.code().map_or_else(|| "?".to_string(), |c| c.to_string());
        Err(format!("yt-dlp exited with code {code}: {line}"))
    }
}