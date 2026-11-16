#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use libloading::{Library, Symbol};
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

type RunDownloadFn = unsafe extern "C" fn(
    *const i8, *const i8, *const i8, *const i8, *const i8, *mut i8, usize,
) -> i32;

#[derive(Clone)]
struct ModuleInfo {
    id: String,
    name: String,
}

struct UniApp {
    modules: Vec<ModuleInfo>,
    selected_module: usize,
    url: String,
    quality: String,
    bitrate: String,
    status: String,
    is_downloading: bool,
    tx: Sender<String>,
    rx: Receiver<String>,
    // Контекстное меню для поля URL
    context_menu_pos: Option<egui::Pos2>,
}

impl Default for UniApp {
    fn default() -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            modules: Vec::new(),
            selected_module: 0,
            url: String::new(),
            quality: "360p".to_string(),
            bitrate: "160".to_string(),
            status: String::new(),
            is_downloading: false,
            tx,
            rx,
            context_menu_pos: None,
        };
        app.modules = get_modules();
        app
    }
}

fn get_exe_dir_path() -> PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

// Чтение пути к ffmpeg из config.ini
fn get_ffmpeg_path() -> PathBuf {
    let exe_dir = get_exe_dir_path();
    let config_path = exe_dir.join("config.ini");
    
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("FFMPEG_PATH") && line.contains('=') {
                let path = line.split('=').nth(1).unwrap_or("").trim();
                if !path.is_empty() {
                    return PathBuf::from(path);
                }
            }
        }
    }
    // По умолчанию — папка Windows
    PathBuf::from("C:\\Windows\\ffmpeg.exe")
}

fn apply_proxy_from_config() {
    let exe_dir = get_exe_dir_path();
    let config_path = exe_dir.join("config.ini");
    
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let line = line.trim();
            // Пропускаем пустые строки и комментарии
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                if key.eq_ignore_ascii_case("HTTP_PROXY") {
                    std::env::set_var("HTTP_PROXY", value);
                } else if key.eq_ignore_ascii_case("HTTPS_PROXY") {
                    std::env::set_var("HTTPS_PROXY", value);
                }
            }
        }
    }
}

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

fn call_module_download(
    module_id: &str,
    url: &str,
    quality: &str,
    bitrate: Option<&str>,
    ffmpeg_path: &PathBuf,
) -> Result<String, String> {
    let exe_dir = get_exe_dir_path();
    let module_dir = exe_dir.join("Modules").join(module_id);
    let dll_path = module_dir.join(format!("module_{}.dll", module_id));
    let ytdlp_path = exe_dir.join("Components").join("yt-dlp.exe");
    let output_dir = exe_dir.join("Downloads").join(module_id);
    
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;

    unsafe {
        let lib = Library::new(&dll_path).map_err(|e| format!("Failed to load DLL: {}", e))?;
        let func: Symbol<RunDownloadFn> = lib
            .get(b"run_download\0")
            .map_err(|e| format!("Function not found: {}", e))?;

        let final_quality = if quality.to_uppercase() == "MP3" {
            if let Some(br) = bitrate { format!("MP3:{}", br) } else { "MP3".to_string() }
        } else { quality.to_string() };

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

        if result_code == 0 { Ok("Success".to_string()) }
        else {
            let err_str = std::ffi::CStr::from_ptr(err_buf.as_ptr()).to_string_lossy().to_string();
            Err(err_str)
        }
    }
}

fn batch_download(
    module_id: &str, 
    quality: &str, 
    bitrate: Option<&str>,
    ffmpeg_path: &PathBuf,
) -> Result<String, String> {
    let exe_dir = get_exe_dir_path();
    let list_file = exe_dir.join(format!("Download{}List.txt", module_id));
    if !list_file.exists() { return Err("List file not found".to_string()); }

    let content = std::fs::read_to_string(&list_file).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut success = 0;
    let mut fail = 0;

    for i in 0..lines.len() {
        let raw_url = lines[i].trim().to_string();
        if raw_url.is_empty() || raw_url.starts_with("BadLink ") { continue; }

        match call_module_download(module_id, &raw_url, quality, bitrate, ffmpeg_path) {
            Ok(_) => { success += 1; lines[i] = String::new(); }
            Err(_) => { fail += 1; lines[i] = format!("BadLink {}", raw_url); }
        }

        let mut out = String::new();
        for l in &lines { if !l.trim().is_empty() { out.push_str(l); out.push('\n'); } }
        std::fs::write(&list_file, out).map_err(|e| e.to_string())?;
    }
    Ok(format!("Batch complete. Success: {}, Errors: {}", success, fail))
}

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
                        let shortcut_path = base_dir.join(format!("{}.lnk", name));
                        let icon_path = path.join("Icon.ico");
                        let shortcut = lnks::ShortcutBuilder::new(exe_path.clone())
                            .arguments(format!("--module {}", id))
                            .working_dir(base_dir.to_string_lossy().to_string())
                            .description(format!("Download from {}", name))
                            .icon(lnks::Icon::new(icon_path))
                            .build();
                        if let Err(e) = shortcut.save(&shortcut_path) {
                            return Err(format!("Failed to save {}: {}", name, e));
                        }
                        created += 1;
                    }
                }
            }
        }
    }
    Ok(format!("Created {} shortcuts", created))
}

impl eframe::App for UniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(msg) = self.rx.try_recv() {
            self.status = msg;
            self.is_downloading = false;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Цвет заголовка
            let header_color = egui::Color32::from_rgb(0, 168, 255);
            ui.label(egui::RichText::new("UniDownloader").size(22.0).color(header_color).strong());
            ui.separator();

            // Выбор модуля
            ui.horizontal(|ui| {
                ui.label("Module:");
                let selected_name = self.modules.get(self.selected_module).map(|m| m.name.as_str()).unwrap_or("None");
                egui::ComboBox::from_id_salt("module_combo")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (i, m) in self.modules.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_module, i, &m.name);
                        }
                    });
            });

            ui.add_space(5.0);

            // URL с контекстным меню
            ui.horizontal(|ui| {
                ui.label("URL:");
                let response = ui.add(egui::TextEdit::singleline(&mut self.url).desired_width(400.0));
                
                // Контекстное меню по правой кнопке
                response.context_menu(|ui| {
                    if ui.button("Paste").clicked() {
                        if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            self.url = text;
                        }
                        ui.close_menu();
                    }
                    if ui.button("Clear").clicked() {
                        self.url.clear();
                        ui.close_menu();
                    }
                });
            });

            ui.add_space(5.0);

            // Качество и битрейт
            ui.horizontal(|ui| {
                ui.label("Quality:");
                
                // Определяем, выбран ли модуль Udio
                let is_udio = self.modules.get(self.selected_module)
                    .map(|m| m.id == "udio").unwrap_or(false);

                egui::ComboBox::from_id_salt("quality_combo")
                    .selected_text(&self.quality)
                    .show_ui(ui, |ui| {
                        if is_udio {
                            // Для Udio: 128, 160, Max
                            for q in &["128", "160", "Max"] {
                                ui.selectable_value(&mut self.quality, q.to_string(), *q);
                            }
                        } else {
                            // Для YouTube и остальных: видео качества + MP3
                            for q in &["144p", "240p", "360p", "480p", "720p", "1080p", "MP3"] {
                                ui.selectable_value(&mut self.quality, q.to_string(), *q);
                            }
                        }
                    });

                // Показываем выбор битрейта только для YouTube в режиме MP3
                if !is_udio && self.quality == "MP3" {
                    ui.label("Bitrate:");
                    egui::ComboBox::from_id_salt("bitrate_combo")
                        .selected_text(format!("{} kbps", self.bitrate))
                        .show_ui(ui, |ui| {
                            for b in &["128", "160", "192", "256"] {
                                ui.selectable_value(&mut self.bitrate, b.to_string(), format!("{} kbps", b));
                            }
                        });
                }
            });

            ui.add_space(10.0);

            // Кнопки
            ui.horizontal(|ui| {
                let btn_enabled = !self.is_downloading && !self.modules.is_empty();
                if ui.add_enabled(btn_enabled, egui::Button::new("Download")).clicked() {
                    let module = self.modules[self.selected_module].id.clone();
                    let url = self.url.clone();
                    let quality = self.quality.clone();
                    let bitrate = if quality == "MP3" { Some(self.bitrate.clone()) } else { None };
                    let ffmpeg = get_ffmpeg_path();
                    let tx = self.tx.clone();
                    self.status = "Downloading...".to_string();
                    self.is_downloading = true;
                    std::thread::spawn(move || {
                        let res = call_module_download(&module, &url, &quality, bitrate.as_deref(), &ffmpeg);
                        let msg = match res { Ok(s) => s, Err(e) => format!("Error: {}", e) };
                        let _ = tx.send(msg);
                    });
                }

                if ui.add_enabled(btn_enabled, egui::Button::new("Batch Download")).clicked() {
                    let module = self.modules[self.selected_module].id.clone();
                    let quality = self.quality.clone();
                    let bitrate = if quality == "MP3" { Some(self.bitrate.clone()) } else { None };
                    let ffmpeg = get_ffmpeg_path();
                    let tx = self.tx.clone();
                    self.status = "Batch processing...".to_string();
                    self.is_downloading = true;
                    std::thread::spawn(move || {
                        let res = batch_download(&module, &quality, bitrate.as_deref(), &ffmpeg);
                        let msg = match res { Ok(s) => s, Err(e) => format!("Error: {}", e) };
                        let _ = tx.send(msg);
                    });
                }
            });

            ui.add_space(5.0);

            // Кнопки открытия и ярлыков
            ui.horizontal(|ui| {
                let module_id = self.modules.get(self.selected_module).map(|m| m.id.clone()).unwrap_or_default();
                if ui.button("📄 Open List").clicked() {
                    let exe_dir = get_exe_dir_path();
                    let list = exe_dir.join(format!("Download{}List.txt", module_id));
                    let _ = std::process::Command::new("explorer").arg(list).spawn();
                }
                if ui.button("📁 Open Folder").clicked() {
                    let exe_dir = get_exe_dir_path();
                    let folder = exe_dir.join("Downloads").join(&module_id);
                    let _ = std::process::Command::new("explorer").arg(folder).spawn();
                }
                if ui.button("Create Shortcuts").clicked() {
                    match create_shortcuts() {
                        Ok(s) => self.status = s,
                        Err(e) => self.status = format!("Error: {}", e),
                    }
                }
            });

            ui.add_space(10.0);
            
            // Статус с цветом (зелёный для успеха, красный для ошибки)
            let status_color = if self.status.starts_with("Error") {
                egui::Color32::from_rgb(255, 77, 77)
            } else {
                header_color
            };
            ui.label(egui::RichText::new(&self.status).color(status_color));

            if self.is_downloading {
                ui.spinner();
            }
        });
    }
}

fn main() -> eframe::Result {
    // Применяем прокси из config.ini перед запуском
    apply_proxy_from_config();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([550.0, 300.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "UniDownloader",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(30, 30, 30);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(UniApp::default()))
        }),
    )
}