#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use module_api::DownloaderModule;
use module_udio::UdioModule;
use module_youtube::YouTubeModule;
use serde::Serialize;

#[derive(Serialize, Clone)]
struct ModuleData {
    id: String,
    name: String,
}

#[tauri::command]
fn get_modules() -> Vec<ModuleData> {
    let modules: Vec<Box<dyn DownloaderModule>> = vec![
        Box::new(UdioModule),
        Box::new(YouTubeModule),
    ];

    modules.iter().map(|m| ModuleData {
        id: m.id().to_string(),
        name: m.name().to_string(),
    }).collect()
}

#[tauri::command]
fn create_shortcuts() -> Result<String, String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let base_dir = exe_path.parent().ok_or("Cannot get exe directory")?;

    let modules: Vec<Box<dyn DownloaderModule>> = vec![
        Box::new(UdioModule),
        Box::new(YouTubeModule),
    ];

    let mut created = 0;
    for module in modules {
        let shortcut_name = format!("{}.lnk", module.name());
        let shortcut_path = base_dir.join(&shortcut_name);

        // build() возвращает Shortcut напрямую, без Result
        let shortcut = lnks::ShortcutBuilder::new(exe_path.clone())
            .arguments(format!("--module {}", module.id()))
            .working_dir(base_dir.to_string_lossy().to_string())
            .description(format!("Download from {}", module.name()))
            .build();

        // Ошибка может возникнуть только при сохранении
        if let Err(e) = shortcut.save(&shortcut_path) {
            return Err(format!("Failed to save {}: {}", shortcut_name, e));
        }
        created += 1;
    }
    Ok(format!("Created {} shortcuts", created))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_modules, create_shortcuts])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}