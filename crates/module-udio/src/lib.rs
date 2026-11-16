use module_api::{guard, Request};
use regex::Regex;
use std::ffi::c_char;
use std::fs;
use std::path::Path;
use std::process::Command;

use widevine::{Cdm, Device, LicenseType, Pssh};

#[no_mangle]
pub extern "C" fn module_abi_version() -> u32 {
    module_api::ABI_VERSION
}

#[no_mangle]
pub unsafe extern "C" fn run_download(
    url: *const c_char,
    quality: *const c_char,
    output_dir: *const c_char,
    _ytdlp_path: *const c_char,
    ffmpeg_path: *const c_char,
    err_buf: *mut c_char,
    err_buf_len: usize,
) -> i32 {
    guard(err_buf, err_buf_len, || {
        let req = Request::from_raw(url, quality, output_dir, _ytdlp_path, ffmpeg_path)?;
        download_udio(&req)
    })
}

fn download_udio(req: &Request) -> Result<(), String> {
    let (manifest_url, metadata) = fetch_manifest_and_meta(&req.url)?;
    let key = get_license_key(&manifest_url)?;
    let (init_url, segment_urls) = parse_manifest(&manifest_url)?;
    
    let temp_dir = Path::new(&req.output_dir).join("temp_segments");
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    
    let init_path = temp_dir.join("init.mp4");
    download_file(&init_url, &init_path)?;
    
    let mut segment_paths = Vec::new();
    for (i, url) in segment_urls.iter().enumerate() {
        let seg_path = temp_dir.join(format!("segment_{}.m4s", i + 1));
        download_file(url, &seg_path)?;
        segment_paths.push(seg_path);
    }
    
    let encrypted_path = temp_dir.join("encrypted.mp4");
    let mut encrypted_data = fs::read(&init_path).map_err(|e| e.to_string())?;
    for seg in &segment_paths {
        encrypted_data.extend_from_slice(&fs::read(seg).map_err(|e| e.to_string())?);
    }
    fs::write(&encrypted_path, &encrypted_data).map_err(|e| e.to_string())?;
    
    let decrypted_path = temp_dir.join("decrypted.mp4");
    let mp4decrypt_path = Path::new(&req.ffmpeg_path).parent().unwrap().join("mp4decrypt.exe");
    
    let status = Command::new(mp4decrypt_path)
        .arg("--key")
        .arg(format!("1:{}", key))
        .arg(&encrypted_path)
        .arg(&decrypted_path)
        .status()
        .map_err(|e| format!("Failed to run mp4decrypt: {}", e))?;
    
    if !status.success() {
        return Err("mp4decrypt failed".to_string());
    }
    
    let safe_title = sanitize_filename(&metadata.title);
    let output_path = Path::new(&req.output_dir).join(format!("{}.mp3", safe_title));
    
    let mut ffmpeg_cmd = Command::new(&req.ffmpeg_path);
    ffmpeg_cmd.arg("-y").arg("-i").arg(&decrypted_path).arg("-vn");
    ffmpeg_cmd.arg("-acodec").arg("libmp3lame");
    
    match req.quality.as_str() {
        "128" => { ffmpeg_cmd.arg("-b:a").arg("128k"); }
        "160" => { ffmpeg_cmd.arg("-b:a").arg("160k"); }
        _ => {}
    }
    
    let status = ffmpeg_cmd.arg(&output_path).status()
        .map_err(|e| format!("Failed to run ffmpeg: {}", e))?;
    
    if !status.success() {
        return Err("ffmpeg failed".to_string());
    }
    
    fs::remove_dir_all(&temp_dir).ok();
    Ok(())
}

fn fetch_manifest_and_meta(track_url: &str) -> Result<(String, Metadata), String> {
    let client = build_client()?;
    let html_text = client.get(track_url).send().map_err(|e| e.to_string())?
        .text().map_err(|e| e.to_string())?;
    
    let uuid_re = Regex::new(r#"\\?"track\\?"\s*:\s*\{\s*\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?""#)
        .map_err(|e| e.to_string())?;
    
    let content_id = if let Some(caps) = uuid_re.captures(&html_text) {
        caps.get(1).unwrap().as_str().to_string()
    } else {
        let fallback_re = Regex::new(r#"\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?""#)
            .map_err(|e| e.to_string())?;
        fallback_re.captures(&html_text)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .ok_or("Could not find content ID")?
    };
    
    let manifest_url = format!("https://stream.udio.com/api/v2/audio-stream/content/{}/manifest.m3u8", content_id);
    
    let title_re = Regex::new(r#"<meta\s+property=["']og:title["']\s+content=["']([^"']+)"#).unwrap();
    let image_re = Regex::new(r#"<meta\s+property=["']og:image["']\s+content=["']([^"']+)"#).unwrap();
    
    let raw_title = title_re.captures(&html_text).map(|c| c.get(1).unwrap().as_str().to_string())
        .unwrap_or_else(|| "Unknown Track".to_string());
    let cover_url = image_re.captures(&html_text).map(|c| c.get(1).unwrap().as_str().to_string());
    
    let raw_title = raw_title.replace("&#x27;", "'").replace("&quot;", "\"").replace("&amp;", "&");
    let raw_title = raw_title.replace(" | Udio", "").trim().to_string();
    
    let (artist, title) = if let Some(pos) = raw_title.find(" - ") {
        (raw_title[..pos].trim().to_string(), raw_title[pos+3..].trim().to_string())
    } else {
        ("Unknown Artist".to_string(), raw_title)
    };
    
    Ok((manifest_url, Metadata { title, artist, cover_url }))
}

fn get_license_key(manifest_url: &str) -> Result<String, String> {
    let client = build_client()?;
    let manifest = client.get(manifest_url).send().map_err(|e| e.to_string())?
        .text().map_err(|e| e.to_string())?;
    
    let pssh_re = Regex::new(r#"URI="data:text/plain;base64,([^"]+)"#).unwrap();
    let pssh_b64 = pssh_re.captures(&manifest)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .ok_or("PSSH not found in manifest")?;
    
    let exe_dir = std::env::current_exe().map_err(|e| e.to_string())?.parent()
        .ok_or("No exe dir")?.to_path_buf();
    let wvd_path = exe_dir.join("Components").join("WidevineKey.wvd");
    
    let device = Device::read_wvd(std::fs::File::open(wvd_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Failed to read WVD: {:?}", e))?;
    let cdm = Cdm::new(device);
    
    let pssh = Pssh::from_b64(&pssh_b64).map_err(|e| format!("Invalid PSSH: {:?}", e))?;
    
    let kid = pssh.key_ids.first().ok_or("No KID in PSSH")?.clone();
    
    let request = cdm.open().get_license_request(pssh, LicenseType::STREAMING)
        .map_err(|e| format!("License request failed: {:?}", e))?;
    let challenge = request.challenge().map_err(|e| format!("Challenge failed: {:?}", e))?;
    
    let lic_resp = client.post("https://stream.udio.com/drm/license?type=widevine")
        .header("Content-Type", "application/octet-stream")
        .header("Origin", "https://www.udio.com")
        .header("Referer", "https://www.udio.com/")
        .body(challenge)
        .send()
        .map_err(|e| format!("License POST failed: {}", e))?;
    
    let lic_data = lic_resp.bytes().map_err(|e| e.to_string())?;
    
    let keys = request.get_keys(&lic_data).map_err(|e| format!("Failed to get keys: {:?}", e))?;
    
    let content_key = keys.content_key(&kid)
        .map_err(|e| format!("Content key not found for KID: {:?}", e))?;
    
    Ok(hex::encode(content_key.key.clone()))
}

fn parse_manifest(manifest_url: &str) -> Result<(String, Vec<String>), String> {
    let client = build_client()?;
    let manifest = client.get(manifest_url).send().map_err(|e| e.to_string())?
        .text().map_err(|e| e.to_string())?;
    
    let base_url = "https://stream.udio.com";
    let mut init_url = String::new();
    let mut segments = Vec::new();
    
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-MAP:") {
            let re = Regex::new(r#"URI="([^"]+)"#).unwrap();
            if let Some(caps) = re.captures(line) {
                let uri = caps.get(1).unwrap().as_str();
                init_url = format!("{}{}", base_url, uri);
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            segments.push(format!("{}{}", base_url, line));
        }
    }
    
    if init_url.is_empty() || segments.is_empty() {
        return Err("Failed to parse manifest".to_string());
    }
    
    Ok((init_url, segments))
}

fn download_file(url: &str, path: &Path) -> Result<(), String> {
    let client = build_client()?;
    let mut resp = client.get(url).send().map_err(|e| e.to_string())?;
    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    resp.copy_to(&mut file).map_err(|e| e.to_string())?;
    Ok(())
}

fn build_client() -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
    
    if let Ok(proxy) = std::env::var("HTTP_PROXY") {
        builder = builder.proxy(reqwest::Proxy::http(&proxy).map_err(|e| e.to_string())?);
    }
    if let Ok(proxy) = std::env::var("HTTPS_PROXY") {
        builder = builder.proxy(reqwest::Proxy::https(&proxy).map_err(|e| e.to_string())?);
    }
    
    builder.build().map_err(|e| e.to_string())
}

fn sanitize_filename(name: &str) -> String {
    let re = Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    let cleaned = re.replace_all(name, "_");
    if cleaned.len() > 100 { cleaned[..100].to_string() } else { cleaned.to_string() }
}

struct Metadata {
    title: String,
    artist: String,
    cover_url: Option<String>,
}