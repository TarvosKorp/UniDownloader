#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use libloading::{Library, Symbol};
use serde::Serialize;
use std::ffi::CString;
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

#[tauri::command]
fn get_modules() -> Vec<ModuleInfo> {
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
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
                            .icon(lnks::Icon::new(icon_path)) // Исправлено!
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

#[tauri::command]
async fn run_download(
    _state: State<'_, AppState>,
    module_id: String, 
    url: String, 
    quality: String
) -> Result<String, String> {
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let module_dir = exe_dir.join("Modules").join(&module_id);
    
    let dll_path = module_dir.join(format!("module_{}.dll", module_id));
    
    let ytdlp_path = exe_dir.join("Components").join("yt-dlp.exe");
    let ffmpeg_path = exe_dir.join("Components").join("ffmpeg.exe");
    let output_dir = exe_dir.join("Downloads").join(&module_id);
    
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;

    let dll_path_clone = dll_path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        unsafe {
            let lib = Library::new(&dll_path_clone).map_err(|e| format!("Failed to load DLL: {}", e))?;
            
            let func: Symbol<RunDownloadFn> = lib
                .get(b"run_download\0")
                .map_err(|e| format!("Function not found: {}", e))?;

            let c_url = CString::new(url).map_err(|e| e.to_string())?;
            let c_quality = CString::new(quality).map_err(|e| e.to_string())?;
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
    }).await.map_err(|e| e.to_string())?;

    result
}

fn main() {
    tauri::Builder::default()
        .manage(AppState { loaded_libs: Mutex::new(Vec::new()) })
        .invoke_handler(tauri::generate_handler![get_modules, create_shortcuts, run_download])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}