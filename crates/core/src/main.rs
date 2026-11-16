use module_api::DownloaderModule;
use module_udio::UdioModule;
use module_youtube::YouTubeModule;

fn main() {
    // Собираем все доступные модули
    let modules: Vec<Box<dyn DownloaderModule>> = vec![
        Box::new(UdioModule),
        Box::new(YouTubeModule),
    ];

    // Парсим аргументы командной строки (--module <id>)
    let args: Vec<String> = std::env::args().collect();
    let active_module = args.iter()
        .position(|a| a == "--module")
        .and_then(|i| args.get(i + 1))
        .cloned();

    println!("UniDownloader starting...");
    if let Some(id) = &active_module {
        println!("Launching with module: {}", id);
    } else {
        println!("Available modules:");
        for m in &modules {
            println!("  - {} (id: {})", m.name(), m.id());
        }
    }
}