fn main() {
    // Иконка для Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../Icon.ico");
        res.compile().unwrap();
    }
    // Сборка Tauri
    tauri_build::build()
}