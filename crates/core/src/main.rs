#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use libloading::{Library, Symbol};
use serde::Serialize;
use std::ffi::CString;

#[derive(Serialize, Clone)]
struct ModuleInfo {
    id: String,
    name: String,
}

type RunDownloadFn = unsafe extern "C" fn(
    *const i8, *const i8, *const i8, *const i8, *const i8,
) -> i32;

#[tauri::command]
fn get_modules() -> Vec<ModuleInfo> {
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let modules_dir = exe_dir.join("Modules");
    
    // ОТЛАДКА: выводим путь
    println!("[DEBUG] Exe dir: {:?}", exe_dir);
    println!("[DEBUG] Modules dir: {:?}", modules_dir);
    
    let mut result = Vec::new();
    
    match std::fs::read_dir(&modules_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                println!("[DEBUG] Found entry: {:?}", path);
                
                if path.is_dir() {
                    let json_path = path.join("module.json");
                    println!("[DEBUG] Trying to read: {:?}", json_path);
                    
                    match std::fs::read_to_string(&json_path) {
                        Ok(content) => {
                            println!("[DEBUG] File content: {}", content);
                            
                            match serde_json::from_str::<serde_json::Value>(&content) {
                                Ok(json) => {
                                    let id = json["id"].as_str().unwrap_or("unknown").to_string();
                                    let name = json["name"].as_str().unwrap_or("Unknown").to_string();
                                    println!("[DEBUG] Parsed module: id={}, name={}", id, name);
                                    result.push(ModuleInfo { id, name });
                                }
                                Err(e) => {
                                    println!("[DEBUG] Failed to parse JSON: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            println!("[DEBUG] Failed to read module.json: {}", e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            println!("[DEBUG] Failed to read Modules dir: {}", e);
        }
    }
    
    println!("[DEBUG] Total modules found: {}", result.len());
    result
}

#[tauri::command]
fn run_download(module_id: String, url: String, quality: String) -> Result<String, String> {
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let module_dir = exe_dir.join("Modules").join(&module_id);
    
    let dll_path = module_dir.join(format!("module_{}.dll", module_id));
    
    let ytdlp_path = exe_dir.join("Components").join("yt-dlp.exe");
    let ffmpeg_path = exe_dir.join("Components").join("ffmpeg.exe");
    let output_dir = exe_dir.join("Downloads").join(&module_id);
    
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;

    unsafe {
        let lib = Library::new(&dll_path).map_err(|e| format!("Failed to load DLL: {}", e))?;
        
        let func: Symbol<RunDownloadFn> = lib
            .get(b"run_download\0")
            .map_err(|e| format!("Function not found: {}", e))?;

        let c_url = CString::new(url).map_err(|e| e.to_string())?;
        let c_quality = CString::new(quality).map_err(|e| e.to_string())?;
        let c_output = CString::new(output_dir.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        let c_ytdlp = CString::new(ytdlp_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;
        let c_ffmpeg = CString::new(ffmpeg_path.to_string_lossy().to_string()).map_err(|e| e.to_string())?;

        let result_code = func(
            c_url.as_ptr(), c_quality.as_ptr(), c_output.as_ptr(), c_ytdlp.as_ptr(), c_ffmpeg.as_ptr()
        );

        std::mem::forget(lib);

        if result_code == 0 {
            Ok("Success".to_string())
        } else {
            Err("Download failed. Check logs.".to_string())
        }
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_modules, run_download])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}