use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::process::Command;
use std::path::PathBuf;

// Экспортируемая функция. Ядро будет её вызывать.
// Принимает C-строки (указатели на char), возвращает 0 (успех) или 1 (ошибка).
#[no_mangle]
pub extern "C" fn run_download(
    url: *const c_char,
    quality: *const c_char,
    output_dir: *const c_char,
    ytdlp_path: *const c_char,
    ffmpeg_path: *const c_char,
) -> i32 {
    // Конвертируем C-строки в Rust String (unsafe, так как это FFI)
    let url = unsafe { CStr::from_ptr(url).to_string_lossy().to_string() };
    let quality = unsafe { CStr::from_ptr(quality).to_string_lossy().to_string() };
    let output_dir = unsafe { CStr::from_ptr(output_dir).to_string_lossy().to_string() };
    let ytdlp = unsafe { CStr::from_ptr(ytdlp_path).to_string_lossy().to_string() };
    let ffmpeg = unsafe { CStr::from_ptr(ffmpeg_path).to_string_lossy().to_string() };

    // Формируем команду для yt-dlp (как мы делали в Python)
    let mut cmd = Command::new(&ytdlp);
    cmd.arg("--no-playlist");
    cmd.arg("--ffmpeg-location").arg(&ffmpeg);

    if quality == "MP3" {
        cmd.arg("-x").arg("--audio-format").arg("mp3");
        cmd.arg("--audio-quality").arg("160K");
        cmd.arg("--embed-thumbnail").arg("--embed-metadata");
        cmd.arg("--convert-thumbnails").arg("jpg");
        // Обрезка обложки (как в Python-версии)
        let crop_filter = "crop='if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'";
        cmd.arg("--ppa").arg(format!("ThumbnailsConvertor+ffmpeg_o:-vf \"{}\"", crop_filter));
    } else {
        let height = quality.replace("p", "");
        let format_str = format!("bestvideo[height<={}]+bestaudio/best[height<={}]", height, height);
        cmd.arg("-f").arg(format_str);
        cmd.arg("--merge-output-format").arg("mp4");
    }

    cmd.arg("-o").arg(format!("{}/%(title)s.%(ext)s", output_dir));
    cmd.arg(&url);

    // Запускаем процесс
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                0 // Успех
            } else {
                eprintln!("yt-dlp error: {}", String::from_utf8_lossy(&output.stderr));
                1 // Ошибка
            }
        }
        Err(e) => {
            eprintln!("Failed to run yt-dlp: {}", e);
            1
        }
    }
}