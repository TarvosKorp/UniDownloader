fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        // Указываем путь к иконке относительно папки crates/core
        // (или используй абсолютный путь, если положил в корень)
        res.set_icon("../../Icon.ico");
        res.compile().unwrap();
    }
}