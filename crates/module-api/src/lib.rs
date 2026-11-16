//! Контракт между ядром (`UniDownloader.exe`) и модулями
//! (`Modules/<id>/module_<id>.dll`).
//!
//! Граница ядро <-> модуль — это **C ABI**: у Rust нет стабильного ABI, поэтому
//! через неё нельзя передавать `String`, `Vec`, `Box<dyn Trait>` и т.п.
//! Передаются только C-строки и числа.
//!
//! Каждая DLL модуля обязана экспортировать две функции:
//!
//! ```ignore
//! #[no_mangle]
//! pub extern "C" fn module_abi_version() -> u32 { module_api::ABI_VERSION }
//!
//! #[no_mangle]
//! pub unsafe extern "C" fn run_download(/* см. RunDownloadFn */) -> i32 {
//!     module_api::guard(err_buf, err_buf_len, || {
//!         let req = module_api::Request::from_raw(url, quality, output_dir, ytdlp_path, ffmpeg_path)?;
//!         my_download(&req)            // Result<(), String>
//!     })
//! }
//! ```
//!
//! Если вы меняете сигнатуру `RunDownloadFn` — увеличьте `ABI_VERSION`:
//! ядро откажется загружать модули, собранные под старый контракт, и покажет
//! понятную ошибку вместо падения.

use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;

/// Версия контракта. Ядро сравнивает её с `module_abi_version()` из DLL.
pub const ABI_VERSION: u32 = 1;

/// Имена экспортируемых символов (с завершающим нулём — для `libloading`).
pub const SYM_ABI_VERSION: &[u8] = b"module_abi_version\0";
pub const SYM_RUN_DOWNLOAD: &[u8] = b"run_download\0";

/// Коды возврата `run_download`.
pub const RESULT_OK: i32 = 0;
pub const RESULT_ERR: i32 = 1;

pub type AbiVersionFn = unsafe extern "C" fn() -> u32;

/// `err_buf`/`err_buf_len` — буфер, выделенный ядром. При ошибке модуль пишет
/// туда текст (UTF-8, с нулём в конце), и ядро показывает его пользователю.
pub type RunDownloadFn = unsafe extern "C" fn(
    url: *const c_char,
    quality: *const c_char,
    output_dir: *const c_char,
    ytdlp_path: *const c_char,
    ffmpeg_path: *const c_char,
    err_buf: *mut c_char,
    err_buf_len: usize,
) -> i32;

/// Аргументы вызова, уже превращённые в обычные Rust-строки.
#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub quality: String,
    pub output_dir: String,
    pub ytdlp_path: String,
    pub ffmpeg_path: String,
}

impl Request {
    /// # Safety
    /// Каждый указатель должен быть либо null, либо указывать на валидную
    /// C-строку с нулём в конце.
    pub unsafe fn from_raw(
        url: *const c_char,
        quality: *const c_char,
        output_dir: *const c_char,
        ytdlp_path: *const c_char,
        ffmpeg_path: *const c_char,
    ) -> Result<Self, String> {
        Ok(Self {
            url: read_cstr(url, "url")?,
            quality: read_cstr(quality, "quality")?,
            output_dir: read_cstr(output_dir, "output_dir")?,
            ytdlp_path: read_cstr(ytdlp_path, "ytdlp_path")?,
            ffmpeg_path: read_cstr(ffmpeg_path, "ffmpeg_path")?,
        })
    }
}

unsafe fn read_cstr(p: *const c_char, what: &str) -> Result<String, String> {
    if p.is_null() {
        return Err(format!("null pointer passed for `{what}`"));
    }
    Ok(CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// Записывает текст ошибки в буфер ядра (с обрезкой по границе UTF-8 символа).
///
/// # Safety
/// `buf` должен быть null или указывать на `len` записываемых байт.
pub unsafe fn write_err(buf: *mut c_char, len: usize, msg: &str) {
    if buf.is_null() || len == 0 {
        return;
    }
    let mut n = msg.len().min(len - 1);
    while n > 0 && !msg.is_char_boundary(n) {
        n -= 1;
    }
    std::ptr::copy_nonoverlapping(msg.as_ptr(), buf as *mut u8, n);
    *buf.add(n) = 0;
}

/// Обёртка для тела `run_download`: превращает `Result` в код возврата,
/// пишет текст ошибки в буфер ядра и **не даёт панике пересечь границу FFI**
/// (паника, выходящая из `extern "C"`, аварийно завершает весь процесс).
///
/// # Safety
/// См. `write_err`.
pub unsafe fn guard<F>(err_buf: *mut c_char, err_buf_len: usize, f: F) -> i32
where
    F: FnOnce() -> Result<(), String>,
{
    let msg = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => return RESULT_OK,
        Ok(Err(e)) => e,
        Err(payload) => {
            let text = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            format!("module panicked: {text}")
        }
    };
    write_err(err_buf, err_buf_len, &msg);
    RESULT_ERR
}

/// Не показывать окно консоли у дочернего процесса (Windows).
/// Ядро собрано как GUI-приложение, и без этого флага каждый запуск
/// yt-dlp/ffmpeg/powershell мигал бы чёрным окном.
pub fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_err_truncates_on_char_boundary() {
        let mut buf = [1 as c_char; 5];
        // 'я' занимает 2 байта; влезает 4 байта + нуль.
        unsafe { write_err(buf.as_mut_ptr(), buf.len(), "яяя") };
        let s = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
        assert_eq!(s, "яя");
    }

    #[test]
    fn guard_maps_ok_err_and_panic() {
        let mut buf = [0 as c_char; 64];
        let p = buf.as_mut_ptr();
        let n = buf.len();

        assert_eq!(unsafe { guard(p, n, || Ok(())) }, RESULT_OK);

        assert_eq!(unsafe { guard(p, n, || Err("boom".to_string())) }, RESULT_ERR);
        assert_eq!(unsafe { CStr::from_ptr(p) }.to_str().unwrap(), "boom");

        assert_eq!(unsafe { guard(p, n, || panic!("oops")) }, RESULT_ERR);
        assert!(unsafe { CStr::from_ptr(p) }.to_str().unwrap().contains("oops"));
    }
}