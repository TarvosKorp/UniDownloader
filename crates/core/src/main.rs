#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use libloading::{Library, Symbol};
use serde::Serialize;
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

type RunDownloadFn = unsafe extern "C" fn(
    *const i8, *const i8, *const i8, *const i8, *const i8, *mut i8, usize,
) -> i32;

#[derive(Serialize, Clone)]
struct ModuleInfo {
    id: String,
    name: String,
}

struct AppState {
    loaded_libs: Mutex<Vec<Library>>,
}

// Вспомогательная функция для получения путей
fn get_exe_dir_path() -> PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

#[tauri::command]
fn get_exe_dir() -> String {
    get_exe_dir_path().to_string_lossy().to_string()
}

#[tauri::command]
fn get_modules() -> Vec<ModuleInfo> {
    let exe_dir = get_exe_dir_path();
    let modules_dir = exe_dir.join("Modules");
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(modules_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let json_path = path.join("module.json");
                if let Ok(content) = std::fs::read_to_string(&json_path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        let id = json["id"].as_str().unwrap_or("unknown").to_string();
                        let name = json["name"].as_str().unwrap_or("Unknown").to_string();
                        result.push(ModuleInfo { id, name });
                    }
                }
            }
        }
    }
    result
}

#[tauri::command]
fn create_shortcuts() -> Result<String, String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let base_dir = exe_path.parent().ok_or("Cannot get exe directory")?;
    let modules_dir = base_dir.join("Modules");
    let mut created = 0;
    if let Ok(entries) = std::fs::read_dir(modules_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let json_path = path.join("module.json");
                if let Ok(content) = std::fs::read_to_string(&json_path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        let id = json["id"].as_str().unwrap_or("unknown");
                        let name = json["name"].as_str().unwrap_or("Unknown");
                        let shortcut_name = format!("{}.lnk", name);
                        let shortcut_path = base_dir.join(&shortcut_name);
                        let icon_path = path.join("Icon.ico");
                        let shortcut = lnks::ShortcutBuilder::new(exe_path.clone())
                            .arguments(format!("--module {}", id))
                            .working_dir(base_dir.to_string_lossy().to_string())
                            .description(format!("Download from {}", name))
                            .icon(lnks::Icon::new(icon_path))
                            .build();
                        if let Err(e) = shortcut.save(&shortcut_path) {
                            return Err(format!("Failed to save {}: {}", shortcut_name, e));
                        }
                        created += 1;
                    }
                }
            }
        }
    }
    Ok(format!("Created {} shortcuts", created))
}

// Загрузка DLL и вызов run_download
fn call_module_download(
    module_id: &str,
    url: &str,
    quality: &str,
    bitrate: Option<&str>,
) -> Result<String, String> {
    let exe_dir = get_exe_dir_path();
    let module_dir = exe_dir.join("Modules").join(module_id);
    let dll_path = module_dir.join(format!("module_{}.dll", module_id));
    let ytdlp_path = exe_dir.join("Components").join("yt-dlp.exe");
    let ffmpeg_path = exe_dir.join("Components").join("ffmpeg.exe");
    let output_dir = exe_dir.join("Downloads").join(module_id);
    
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;

    unsafe {
        let lib = Library::new(&dll_path).map_err(|e| format!("Failed to load DLL: {}", e))?;
        let func: Symbol<RunDownloadFn> = lib
            .get(b"run_download\0")
            .map_err(|e| format!("Function not found: {}", e))?;

        // Формируем quality с учетом битрейта, если это MP3
        let final_quality = if quality.to_uppercase() == "MP3" {
            if let Some(br) = bitrate {
                format!("MP3:{}", br)
            } else {
                "MP3".to_string()
            }
        } else {
            quality.to_string()
        };

        let c_url = CString::new(url).map_err(|e| e.to_string())?;
        let c_quality = CString::new(final_quality).map_err(|e| e.to_string())?;
        let c_output = CString::new(output_dir.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        let c_ytdlp = CString::new(ytdlp_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        let c_ffmpeg = CString::new(ffmpeg_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;

        let mut err_buf = [0i8; 512];
        let result_code = func(
            c_url.as_ptr(), c_quality.as_ptr(), c_output.as_ptr(), c_ytdlp.as_ptr(), c_ffmpeg.as_ptr(),
            err_buf.as_mut_ptr(), err_buf.len()
        );

        std::mem::forget(lib);

        if result_code == 0 {
            Ok("Success".to_string())
        } else {
            let err_str = std::ffi::CStr::from_ptr(err_buf.as_ptr()).to_string_lossy().to_string();
            Err(err_str)
        }
    }
}

#[tauri::command]
async fn run_download(
    module_id: String, 
    url: String, 
    quality: String,
    bitrate: Option<String>
) -> Result<String, String> {
    let br = bitrate.as_deref();
    tauri::async_runtime::spawn_blocking(move || {
        call_module_download(&module_id, &url, &quality, br)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn batch_download(
    module_id: String,
    quality: String,
    bitrate: Option<String>,
) -> Result<String, String> {
    let exe_dir = get_exe_dir_path();
    let list_file = exe_dir.join(format!("Download{}List.txt", module_id));
    
    if !list_file.exists() {
        return Err("List file not found".to_string());
    }

    let content = std::fs::read_to_string(&list_file).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let total_valid = lines.iter().filter(|l| !l.trim().is_empty() && !l.starts_with("BadLink ")).count();
    
    if total_valid == 0 {
        return Ok("List is empty".to_string());
    }

    let br = bitrate.clone();
    let mut success = 0;
    let mut fail = 0;

    for i in 0..lines.len() {
        let raw_url = lines[i].trim().to_string();
        if raw_url.is_empty() || raw_url.starts_with("BadLink ") {
            continue;
        }

        let res = call_module_download(&module_id, &raw_url, &quality, br.as_deref());
        match res {
            Ok(_) => {
                success += 1;
                lines[i] = String::new();
            }
            Err(_) => {
                fail += 1;
                lines[i] = format!("BadLink {}", raw_url);
            }
        }

        // Сохраняем прогресс после каждой ссылки
        let mut out = String::new();
        for l in &lines {
            if !l.trim().is_empty() {
                out.push_str(l);
                out.push('\n');
            }
        }
        std::fs::write(&list_file, out).map_err(|e| e.to_string())?;
    }

    Ok(format!("Batch complete. Success: {}, Errors: {}", success, fail))
}

#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn open_folder(module_id: String) -> Result<(), String> {
    let exe_dir = get_exe_dir_path();
    let folder = exe_dir.join("Downloads").join(module_id);
    std::process::Command::new("explorer")
        .arg(folder)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(AppState { loaded_libs: Mutex::new(Vec::new()) })
        .invoke_handler(tauri::generate_handler![
            get_exe_dir,
            get_modules, 
            create_shortcuts, 
            run_download, 
            batch_download,
            open_file,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}