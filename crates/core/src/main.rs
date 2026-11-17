#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use libloading::Library;
use module_api::{AbiVersionFn, ModuleUiJsonFn, RunDownloadFn, ABI_VERSION, SYM_ABI_VERSION, SYM_MODULE_UI_JSON, SYM_RUN_DOWNLOAD};
use serde::Deserialize;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Clone, Deserialize)]
struct ModuleUi {
    name: String,
    url_label: String,
    qualities: Vec<String>,
    default_quality: String,
    supports_bitrate: bool,
    supports_batch: bool,
}

#[derive(Deserialize)]
struct ModuleManifest { id: String, name: String }

struct ModuleLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl ModuleLock {
    fn acquire(module_id: &str) -> Result<Self, String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;
            let name = format!("Local\\UniDownloader.Module.{module_id}");
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            SetLastError(0);
            let handle = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
            if handle.is_null() {
                return Err(format!("Cannot create module mutex (Windows error {})", GetLastError()));
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return Err(format!("The {module_id} module is already running."));
            }
            return Ok(Self { handle });
        }
        #[cfg(not(windows))]
        {
            let _ = module_id;
            Ok(Self {})
        }
    }
}

impl Drop for ModuleLock {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if !self.handle.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

struct ModuleRuntime {
    _library: Library,
    run_download: RunDownloadFn,
    ui: ModuleUi,
    id: String,
    _lock: ModuleLock,
}

fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())?
        .parent().map(Path::to_path_buf).ok_or("Cannot locate executable directory".into())
}

fn valid_module_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn load_module(module_id: &str) -> Result<ModuleRuntime, String> {
    if !valid_module_id(module_id) { return Err("Invalid module ID in command line".into()); }
    let lock = ModuleLock::acquire(module_id)?;
    let root = exe_dir()?.join("Modules").join(module_id);
    let manifest_path = root.join("module.json");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Cannot read {}: {e}", manifest_path.display()))?;
    let manifest: ModuleManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("Invalid module manifest: {e}"))?;
    if manifest.id != module_id || !valid_module_id(&manifest.id) {
        return Err("Module manifest ID does not match the requested module".into());
    }

    let dll_path = root.join(format!("module_{module_id}.dll"));
    let library = unsafe { Library::new(&dll_path) }
        .map_err(|e| format!("Cannot load module DLL {}: {e}", dll_path.display()))?;
    unsafe {
        let abi: AbiVersionFn = *library.get(SYM_ABI_VERSION)
            .map_err(|e| format!("Module does not export ABI version: {e}"))?;
        let found = abi();
        if found != ABI_VERSION {
            return Err(format!("Incompatible module ABI: expected {ABI_VERSION}, found {found}. Rebuild this module."));
        }
        let ui_fn: ModuleUiJsonFn = *library.get(SYM_MODULE_UI_JSON)
            .map_err(|e| format!("Module does not export its UI descriptor: {e}"))?;
        let ui_ptr = ui_fn();
        if ui_ptr.is_null() { return Err("Module returned a null UI descriptor".into()); }
        let ui_json = CStr::from_ptr(ui_ptr).to_str()
            .map_err(|e| format!("Module UI descriptor is not UTF-8: {e}"))?;
        let ui: ModuleUi = serde_json::from_str(ui_json)
            .map_err(|e| format!("Invalid module UI descriptor: {e}"))?;
        if ui.qualities.is_empty() || !ui.qualities.contains(&ui.default_quality) {
            return Err("Module UI descriptor has invalid quality options".into());
        }
        let run_download: RunDownloadFn = *library.get(SYM_RUN_DOWNLOAD)
            .map_err(|e| format!("Module does not export run_download: {e}"))?;
        Ok(ModuleRuntime { _library: library, run_download, ui, id: module_id.to_string(), _lock: lock })
    }
}

fn config_value(content: &str, key: &str) -> Option<String> {
    content.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') { return None; }
        let (k, v) = line.split_once('=')?;
        if k.trim().eq_ignore_ascii_case(key) { Some(v.trim().to_string()) } else { None }
    }).last()
}

fn config_text() -> String {
    fs::read_to_string(exe_dir().unwrap_or_default().join("config.ini")).unwrap_or_default()
}

fn ffmpeg_path() -> PathBuf {
    let dir = exe_dir().unwrap_or_default();
    match config_value(&config_text(), "FFMPEG_PATH").filter(|s| !s.is_empty()).map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        Some(path) => dir.join(path),
        None => dir.join("Components").join("ffmpeg.exe"),
    }
}

fn apply_proxy_config() {
    let config = config_text();
    let enabled = config_value(&config, "USE_PROXY")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1" || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    if !enabled {
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("http_proxy");
        std::env::remove_var("https_proxy");
        return;
    }
    if let Some(v) = config_value(&config, "HTTP_PROXY") { std::env::set_var("HTTP_PROXY", v); }
    if let Some(v) = config_value(&config, "HTTPS_PROXY") { std::env::set_var("HTTPS_PROXY", v); }
}

fn c_string(value: &str, field: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| format!("{field} contains an invalid NUL character"))
}

fn call_download(run: RunDownloadFn, module_id: &str, url: &str, quality: &str) -> Result<(), String> {
    let root = exe_dir()?;
    let output = root.join("Downloads").join(module_id);
    fs::create_dir_all(&output).map_err(|e| format!("Cannot create download directory: {e}"))?;
    let ytdlp = root.join("Components").join("yt-dlp.exe");
    let ffmpeg = ffmpeg_path();
    let args = [
        c_string(url, "URL")?, c_string(quality, "quality")?,
        c_string(&output.to_string_lossy(), "output path")?,
        c_string(&ytdlp.to_string_lossy(), "yt-dlp path")?,
        c_string(&ffmpeg.to_string_lossy(), "FFmpeg path")?,
    ];
    let mut error = vec![0i8; 8192];
    let code = unsafe {
        run(args[0].as_ptr(), args[1].as_ptr(), args[2].as_ptr(), args[3].as_ptr(),
            args[4].as_ptr(), error.as_mut_ptr(), error.len())
    };
    if code == 0 { return Ok(()); }
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().trim().to_string();
    if message.is_empty() { Err(format!("Module returned error code {code}")) }
    else { Err(message) }
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("List file has no parent directory")?;
    let mut temp_path = None;
    let mut file = None;
    for n in 0..100u32 {
        let candidate = parent.join(format!(".{}.{}.{}.tmp", path.file_name().and_then(|s| s.to_str()).unwrap_or("batch-list"),
            std::process::id(), n));
        match OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(f) => { temp_path = Some(candidate); file = Some(f); break; }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create temporary list file: {e}")),
        }
    }
    let temp_path = temp_path.ok_or("Cannot allocate a temporary list filename")?;
    let mut file = file.unwrap();
    let write_result = file.write_all(data).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Cannot write or flush temporary list: {error}"));
    }
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Cannot replace batch list: {error}"));
    }
    Ok(())
}

fn append_error_log(module_id: &str, url: &str, error: &str) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Keep diagnostics useful but do not persist DRM keys or request bodies.
    let safe = error.replace('\n', " ").replace('\r', " ");
    let line = format!("{timestamp}\t{module_id}\t{url}\tdownload\t{safe}\n");
    if let Ok(dir) = exe_dir() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(dir.join("ErrorsLog.txt")) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn batch_download(run: RunDownloadFn, module_id: &str, quality: &str) -> Result<String, String> {
    let list = exe_dir()?.join(format!("Download{module_id}List.txt"));
    if !list.is_file() { return Err(format!("Batch list not found: {}", list.display())); }
    let text = fs::read_to_string(&list).map_err(|e| format!("Cannot read batch list: {e}"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut success = 0usize;
    let mut failed = 0usize;

    // Strictly sequential: a new track starts only after the previous call returns.
    for i in 0..lines.len() {
        let url = lines[i].trim().to_string();
        if url.is_empty() || url.starts_with("BadLink ") { continue; }
        match call_download(run, module_id, &url, quality) {
            Ok(()) => { success += 1; lines[i].clear(); }
            Err(error) => {
                failed += 1;
                append_error_log(module_id, &url, &error);
                lines[i] = format!("BadLink {url}");
            }
        }
        let remaining = lines.iter().filter(|s| !s.trim().is_empty()).cloned().collect::<Vec<_>>().join("\n");
        let bytes = if remaining.is_empty() { Vec::new() } else { format!("{remaining}\n").into_bytes() };
        // Current URL is changed only after its download has returned. If the
        // process is interrupted during that call, it remains in the list.
        atomic_write(&list, &bytes)?;
    }
    Ok(format!("Batch complete. Success: {success}, errors: {failed}"))
}

fn discover_modules() -> Vec<ModuleManifest> {
    let mut found = Vec::new();
    let modules = exe_dir().unwrap_or_default().join("Modules");
    if let Ok(entries) = fs::read_dir(modules) {
        for entry in entries.flatten() {
            let folder_id = entry.file_name().to_string_lossy().to_string();
            let path = entry.path().join("module.json");
            if let Ok(data) = fs::read_to_string(path) {
                if let Ok(m) = serde_json::from_str::<ModuleManifest>(&data) {
                    if valid_module_id(&m.id) && m.id == folder_id && !m.name.trim().is_empty() {
                        found.push(m);
                    }
                }
            }
        }
    }
    found
}

fn create_shortcuts() -> Result<String, String> {
    let base = exe_dir()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut created = 0usize;
    for module in discover_modules() {
        let folder = base.join("Modules").join(&module.id);
        let dll = folder.join(format!("module_{}.dll", module.id));
        if !dll.is_file() { continue; }
        let safe_name: String = module.name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
        let shortcut_path = base.join(format!("{safe_name}.lnk"));
        let icon = folder.join("Icon.ico");
        let mut builder = lnks::ShortcutBuilder::new(exe.clone())
            .arguments(format!("--module {}", module.id))
            .working_dir(base.to_string_lossy().to_string())
            .description(format!("Launch {} downloader", module.name));
        if icon.is_file() {
            builder = builder.icon(lnks::Icon::new(icon));
        }
        let shortcut = builder.build();
        shortcut.save(&shortcut_path).map_err(|e| format!("Cannot create shortcut for {}: {e}", module.name))?;
        created += 1;
    }
    Ok(format!("Created {created} module shortcuts"))
}

struct LauncherApp { status: String }
impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("UniDownloader");
            ui.separator();
            ui.label("Modules are launched through their own shortcuts.");
            if ui.button("Create / refresh module shortcuts").clicked() {
                self.status = create_shortcuts().unwrap_or_else(|e| format!("Error: {e}"));
            }
            ui.label(&self.status);
        });
    }
}

struct ModuleApp {
    runtime: ModuleRuntime,
    url: String,
    quality: String,
    bitrate: String,
    status: String,
    busy: bool,
    tx: Sender<String>,
    rx: Receiver<String>,
}

impl ModuleApp {
    fn new(runtime: ModuleRuntime) -> Self {
        let quality = runtime.ui.default_quality.clone();
        let (tx, rx) = channel();
        Self { runtime, url: String::new(), quality, bitrate: "160".into(),
            status: String::new(), busy: false, tx, rx }
    }
}

impl eframe::App for ModuleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(message) = self.rx.try_recv() { self.status = message; self.busy = false; }
        let ui_spec = self.runtime.ui.clone();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(&ui_spec.name);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(&ui_spec.url_label);
                ui.add(egui::TextEdit::singleline(&mut self.url).desired_width(390.0));
            });
            ui.horizontal(|ui| {
                ui.label("Quality:");
                egui::ComboBox::from_id_salt("quality")
                    .selected_text(&self.quality).show_ui(ui, |ui| {
                        for quality in &ui_spec.qualities {
                            ui.selectable_value(&mut self.quality, quality.clone(), quality);
                        }
                    });
                if ui_spec.supports_bitrate && self.quality == "MP3" {
                    ui.label("Bitrate:");
                    egui::ComboBox::from_id_salt("bitrate").selected_text(format!("{} kbps", self.bitrate))
                        .show_ui(ui, |ui| {
                            for rate in ["128","160","192","256"] {
                                ui.selectable_value(&mut self.bitrate, rate.to_string(), format!("{rate} kbps"));
                            }
                        });
                }
            });
            ui.horizontal(|ui| {
                let enabled = !self.busy && !self.url.trim().is_empty();
                if ui.add_enabled(enabled, egui::Button::new("Download")).clicked() {
                    let run = self.runtime.run_download;
                    let module = self.runtime.id.clone();
                    let url = self.url.trim().to_string();
                    let quality = if self.quality == "MP3" { format!("MP3:{}",self.bitrate) } else { self.quality.clone() };
                    let tx = self.tx.clone();
                    self.busy = true; self.status = "Downloading...".into();
                    std::thread::spawn(move || {
                        let result = call_download(run,&module,&url,&quality);
                        if let Err(ref e) = result { append_error_log(&module,&url,e); }
                        let _ = tx.send(result.map(|_| "Download complete".into()).unwrap_or_else(|e| format!("Error: {e}")));
                    });
                }
                if ui_spec.supports_batch && ui.add_enabled(!self.busy,egui::Button::new("Batch Download")).clicked() {
                    let run = self.runtime.run_download;
                    let module = self.runtime.id.clone();
                    let quality = if self.quality == "MP3" { format!("MP3:{}",self.bitrate) } else { self.quality.clone() };
                    let tx = self.tx.clone();
                    self.busy = true; self.status = "Processing batch sequentially...".into();
                    std::thread::spawn(move || {
                        let result = batch_download(run,&module,&quality);
                        let _ = tx.send(result.unwrap_or_else(|e| format!("Error: {e}")));
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Open batch list").clicked() {
                    let p = exe_dir().unwrap_or_default().join(format!("Download{}List.txt",self.runtime.id));
                    let _ = std::process::Command::new("explorer").arg(p).spawn();
                }
                if ui.button("Open downloads folder").clicked() {
                    let p = exe_dir().unwrap_or_default().join("Downloads").join(&self.runtime.id);
                    let _ = std::process::Command::new("explorer").arg(p).spawn();
                }
            });
            ui.label(&self.status);
            if self.busy { ui.spinner(); }
        });
    }
}

fn run_window<T: eframe::App + 'static>(title: String, size: [f32;2], app: T) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(size).with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(&title, options, Box::new(|cc| {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(30,30,30);
        cc.egui_ctx.set_visuals(visuals);
        Ok(Box::new(app))
    }))
}

fn show_error(message: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        let title: Vec<u16> = "UniDownloader".encode_utf16().chain(std::iter::once(0)).collect();
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}

fn main() -> eframe::Result {
    apply_proxy_config();
    let args: Vec<String> = std::env::args().collect();
    let module_arg = args.windows(2).find(|w| w[0] == "--module").map(|w| w[1].clone());
    if let Some(id) = module_arg {
        let runtime = match load_module(&id) {
            Ok(r) => r,
            Err(error) => {
                show_error(&error);
                return Ok(());
            }
        };
        let title = runtime.ui.name.clone();
        run_window(title, [600.0, 260.0], ModuleApp::new(runtime))
    } else {
        run_window("UniDownloader".into(), [430.0, 180.0], LauncherApp { status: String::new() })
    }
}
