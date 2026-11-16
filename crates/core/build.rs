fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        // Путь к иконке: из папки crates/core поднимаемся на два уровня вверх (корень репозитория)
        res.set_icon("../../Icon.ico");
        res.compile().unwrap();
    }
}