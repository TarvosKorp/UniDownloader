#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use module_api::DownloaderModule;
use module_udio::UdioModule;
use module_youtube::YouTubeModule;

fn main() {
    let modules: Vec<Box<dyn DownloaderModule>> = vec![
        Box::new(UdioModule),
        Box::new(YouTubeModule),
    ];

    let args: Vec<String> = std::env::args().collect();
    let active_module = args.iter()
        .position(|a| a == "--module")
        .and_then(|i| args.get(i + 1))
        .cloned();

    tauri::Builder::default()
        .setup(move |_app| {
            if let Some(id) = &active_module {
                println!("Launching with module: {}", id);
            } else {
                println!("Available modules:");
                for m in &modules {
                    println!("  - {} (id: {})", m.name(), m.id());
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}