use module_api::{guard, hide_console, Request};
use id3::{frame::{Content, Picture, PictureType}, Tag, TagLike, Version};
use regex::Regex;
use reqwest::{blocking::Client, Url};
use std::{ffi::c_char, fs, io::Write, path::{Path, PathBuf}, process::Command, thread};
use tempfile::Builder;
use widevine::{Cdm, Device, KeyType, LicenseType, Pssh};

static MODULE_UI_JSON: &[u8] = b"{\"name\":\"Udio\",\"url_label\":\"Udio track URL\",\"qualities\":[\"128\",\"160\",\"Max\"],\"default_quality\":\"Max\",\"supports_bitrate\":false,\"supports_batch\":true}\0";

#[no_mangle]
pub extern "C" fn module_ui_json() -> *const c_char { MODULE_UI_JSON.as_ptr() as *const c_char }

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const WORKERS: usize = 4;
const RETRIES: usize = 3;

#[no_mangle]
pub extern "C" fn module_abi_version() -> u32 { module_api::ABI_VERSION }

#[no_mangle]
pub unsafe extern "C" fn run_download(
    url: *const c_char, quality: *const c_char, output_dir: *const c_char,
    ytdlp_path: *const c_char, ffmpeg_path: *const c_char,
    err_buf: *mut c_char, err_buf_len: usize,
) -> i32 {
    guard(err_buf, err_buf_len, || {
        let req = Request::from_raw(url, quality, output_dir, ytdlp_path, ffmpeg_path)?;
        download_udio(&req)
    })
}

fn download_udio(req: &Request) -> Result<(), String> {
    let out = Path::new(&req.output_dir);
    fs::create_dir_all(out).map_err(|e| format!("Cannot create output directory: {e}"))?;
    let temp = Builder::new().prefix("udio-track-").tempdir_in(out)
        .map_err(|e| format!("Cannot create temporary directory: {e}"))?;
    let dir = temp.path();
    let client = build_client()?;
    let (manifest_url, meta) = fetch_metadata(&client, &req.url)?;
    let (init_url, segments) = parse_manifest(&client, &manifest_url)?;
    let key = get_key(&client, &manifest_url)?;

    let init = dir.join("init.mp4");
    download_file(&client, &init_url, &init)?;
    let segment_paths = download_segments(&client, &segments, dir)?;
    let encrypted = dir.join("encrypted.mp4");
    {
        let mut dst = fs::File::create(&encrypted).map_err(|e| e.to_string())?;
        for part in std::iter::once(&init).chain(segment_paths.iter()) {
            let mut src = fs::File::open(part).map_err(|e| format!("Cannot open {}: {e}", part.display()))?;
            std::io::copy(&mut src, &mut dst).map_err(|e| format!("Cannot assemble segments: {e}"))?;
        }
        dst.flush().map_err(|e| e.to_string())?;
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let decrypt_exe = exe.parent().ok_or("Cannot locate application directory")?
        .join("Components").join("mp4decrypt.exe");
    if !decrypt_exe.is_file() { return Err(format!("mp4decrypt.exe not found: {}", decrypt_exe.display())); }
    let decrypted = dir.join("decrypted.mp4");
    let mut cmd = Command::new(&decrypt_exe);
    hide_console(&mut cmd);
    let result = cmd.arg("--key").arg(format!("1:{key}")).arg(&encrypted).arg(&decrypted)
        .output().map_err(|e| format!("Cannot start mp4decrypt: {e}"))?;
    if !result.status.success() { return Err(process_error("mp4decrypt", &result.status, &result.stderr)); }
    if !decrypted.is_file() { return Err("mp4decrypt produced no output file".into()); }

    let mp3 = dir.join("track.mp3");
    let mut ff = Command::new(&req.ffmpeg_path);
    hide_console(&mut ff);
    ff.arg("-y").arg("-i").arg(&decrypted).arg("-vn").arg("-acodec").arg("libmp3lame");
    match req.quality.as_str() {
        "128" => { ff.arg("-b:a").arg("128k"); }
        "160" => { ff.arg("-b:a").arg("160k"); }
        _ => { ff.arg("-q:a").arg("2"); } // Same VBR quality as the working Python downloader.
    }
    let result = ff.arg(&mp3).output().map_err(|e| format!("Cannot start FFmpeg: {e}"))?;
    if !result.status.success() { return Err(process_error("FFmpeg", &result.status, &result.stderr)); }
    if !mp3.is_file() { return Err("FFmpeg produced no MP3 file".into()); }

    let cover = meta.cover.as_deref().and_then(|u| download_bytes(&client, u).ok());
    write_tags(&mp3, &meta, cover, &req.url)?;
    let final_path = publish_unique(out, &sanitize(&meta.title), &mp3)?;
    drop(temp);
    Ok(format!("Saved: {}", final_path.display()))
}

fn build_client() -> Result<Client, String> {
    let mut b = Client::builder().user_agent(UA).timeout(std::time::Duration::from_secs(30));
    if let Ok(p) = std::env::var("HTTP_PROXY") {
        if !p.trim().is_empty() { b = b.proxy(reqwest::Proxy::http(&p).map_err(|e| e.to_string())?); }
    }
    if let Ok(p) = std::env::var("HTTPS_PROXY") {
        if !p.trim().is_empty() { b = b.proxy(reqwest::Proxy::https(&p).map_err(|e| e.to_string())?); }
    }
    b.build().map_err(|e| format!("Cannot create HTTP client: {e}"))
}

fn fetch_metadata(c: &Client, track: &str) -> Result<(String, Metadata), String> {
    let html = c.get(track).send().map_err(|e| format!("Track page request failed: {e}"))?
        .error_for_status().map_err(|e| format!("Track page HTTP error: {e}"))?
        .text().map_err(|e| e.to_string())?;
    let uuid = Regex::new(r#"\\?"track\\?"\s*:\s*\{\s*\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?""#).unwrap();
    let fallback = Regex::new(r#"\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?""#).unwrap();
    let id = uuid.captures(&html).or_else(|| fallback.captures(&html))
        .and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
        .ok_or("Could not find Udio track ID")?;
    let title_re = Regex::new(r#"(?i)<meta\s+property=["']og:title["']\s+content=["']([^"']+)"#).unwrap();
    let image_re = Regex::new(r#"(?i)<meta\s+property=["']og:image["']\s+content=["']([^"']+)"#).unwrap();
    let tags_re = Regex::new(r#"\\?"tags\\?"\s*:\s*\[([^\]]*)\]"#).unwrap();
    let item_re = Regex::new(r#"\\?"([^"\\]+)\\?""#).unwrap();
    let raw = title_re.captures(&html).map(|c| decode(c.get(1).unwrap().as_str()))
        .unwrap_or_else(|| "Unknown Track".into()).replace(" | Udio", "");
    let (artist, title) = raw.split_once(" - ").map(|(a,t)| (a.trim().to_string(),t.trim().to_string()))
        .unwrap_or_else(|| ("Unknown Artist".into(),raw.trim().into()));
    let cover = image_re.captures(&html).map(|c| decode(c.get(1).unwrap().as_str()));
    let tags = tags_re.captures(&html).map(|c| item_re.captures_iter(c.get(1).unwrap().as_str())
        .map(|m| decode(m.get(1).unwrap().as_str())).collect()).unwrap_or_default();
    Ok((format!("https://stream.udio.com/api/v2/audio-stream/content/{id}/manifest.m3u8"),
        Metadata { title, artist, tags, cover }))
}

fn parse_manifest(c: &Client, url: &str) -> Result<(String, Vec<String>), String> {
    let text = c.get(url).send().map_err(|e| e.to_string())?.error_for_status()
        .map_err(|e| format!("Manifest HTTP error: {e}"))?.text().map_err(|e| e.to_string())?;
    let base = Url::parse("https://stream.udio.com/").unwrap();
    let map = Regex::new(r#"URI="([^"]+)""#).unwrap();
    let mut init = None;
    let mut parts = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with("#EXT-X-MAP:") {
            let uri = map.captures(line).and_then(|c| c.get(1)).ok_or("No init URI in manifest")?.as_str();
            init = Some(base.join(uri).map_err(|e| e.to_string())?.to_string());
        } else if !line.is_empty() && !line.starts_with('#') {
            parts.push(base.join(line).map_err(|e| e.to_string())?.to_string());
        }
    }
    Ok((init.ok_or("No init segment in manifest")?,
        if parts.is_empty() { return Err("No media segments in manifest".into()); } else { parts }))
}

fn get_key(c: &Client, url: &str) -> Result<String, String> {
    let manifest = c.get(url).send().map_err(|e| e.to_string())?.error_for_status()
        .map_err(|e| format!("License manifest HTTP error: {e}"))?.text().map_err(|e| e.to_string())?;
    let pssh = Regex::new(r#"URI="data:text/plain;base64,([^"]+)"#).unwrap().captures(&manifest)
        .and_then(|m| m.get(1)).map(|m| m.as_str()).ok_or("PSSH not found")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let wvd = exe.parent().ok_or("No application directory")?.join("Components").join("WidevineKey.wvd");
    let device = Device::read_wvd(fs::File::open(&wvd).map_err(|e| format!("Cannot open {}: {e}",wvd.display()))?)
        .map_err(|e| format!("Cannot read Widevine device: {e:?}"))?;
    let request = Cdm::new(device).open().get_license_request(Pssh::from_b64(pssh).map_err(|e| format!("{e:?}"))?, LicenseType::STREAMING)
        .map_err(|e| format!("License challenge creation failed: {e:?}"))?;
    let challenge = request.challenge().map_err(|e| format!("Challenge error: {e:?}"))?;
    let response = c.post("https://stream.udio.com/drm/license?type=widevine")
        .header("User-Agent",UA).header("Content-Type","application/octet-stream")
        .header("Origin","https://www.udio.com").header("Referer","https://www.udio.com/")
        .body(challenge).send().map_err(|e| format!("License POST failed: {e}"))?
        .error_for_status().map_err(|e| format!("License server HTTP error: {e}"))?;
    let data = response.bytes().map_err(|e| e.to_string())?;
    let keys = request.get_keys(&data).map_err(|e| format!("Cannot parse license: {e:?}"))?;
    let key = keys.first_of_type(KeyType::CONTENT)
        .map_err(|e| format!("License response contains no CONTENT key: {e:?}"))?;
    // Do not include the content key, challenge or license body in diagnostics.
    Ok(hex::encode(key.key.clone()))
}

fn download_segments(c: &Client, urls: &[String], dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut ordered: Vec<Option<PathBuf>> = vec![None; urls.len()];
    for start in (0..urls.len()).step_by(WORKERS) {
        let end = (start + WORKERS).min(urls.len());
        let results = thread::scope(|scope| {
            let mut jobs = Vec::new();
            for i in start..end {
                let url = &urls[i];
                let path = dir.join(format!("segment_{:06}.m4s",i+1));
                jobs.push((i,scope.spawn(move || { download_file(c,url,&path)?; Ok::<_,String>(path) })));
            }
            jobs.into_iter().map(|(i,j)| {
                let p = j.join().map_err(|_| format!("Segment worker {} panicked",i+1))??;
                Ok((i,p))
            }).collect::<Result<Vec<_>,String>>()
        })?;
        for (i,p) in results { ordered[i] = Some(p); }
    }
    ordered.into_iter().enumerate().map(|(i,p)| p.ok_or_else(|| format!("Segment {} missing",i+1))).collect()
}

fn download_file(c: &Client, url: &str, path: &Path) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 1..=RETRIES {
        let result = (|| {
            let mut r = c.get(url).send().map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
            let mut f = fs::File::create(path).map_err(|e| e.to_string())?;
            r.copy_to(&mut f).map_err(|e| e.to_string())?;
            f.flush().map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(e) => { last=e; let _=fs::remove_file(path); if attempt<RETRIES { thread::sleep(std::time::Duration::from_millis(300*attempt as u64)); } }
        }
    }
    Err(format!("Segment download failed after {RETRIES} attempts: {url}: {last}"))
}

fn download_bytes(c: &Client,url: &str)->Result<Vec<u8>,String>{
    c.get(url).send().map_err(|e|e.to_string())?.error_for_status().map_err(|e|e.to_string())?
        .bytes().map(|b|b.to_vec()).map_err(|e|e.to_string())
}

fn write_tags(path:&Path,m:&Metadata,cover:Option<Vec<u8>>,url:&str)->Result<(),String>{
    let mut tag=Tag::new();
    tag.set_title(m.title.clone()); tag.set_artist(m.artist.clone());
    if !m.tags.is_empty(){tag.set_genre(m.tags.join(", "));}
    tag.add_frame(id3::Frame::with_content("WOAR",Content::Link(url.to_string())));
    if let Some(data)=cover{
        tag.add_frame(id3::Frame::with_content("APIC",Content::Picture(Picture{
            mime_type:"image/jpeg".into(),picture_type:PictureType::CoverFront,
            description:"Cover".into(),data
        })));
    }
    tag.write_to_path(path,Version::Id3v24).map_err(|e|format!("Cannot write ID3 tags: {e}"))
}

fn publish_unique(dir:&Path,base:&str,source:&Path)->Result<PathBuf,String>{
    for n in 0..10000 {
        let name=if n==0{format!("{base}.mp3")}else{format!("{base} {n}.mp3")};
        let target=dir.join(name);
        // Atomic same-volume hard-link creation: never overwrite an existing file.
        match fs::hard_link(source,&target){
            Ok(())=>return Ok(target),
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,
            Err(e)=>return Err(format!("Cannot publish {}: {e}",target.display()))
        }
    }
    Err("No free output filename after 10000 attempts".into())
}

fn process_error(name:&str,status:&std::process::ExitStatus,stderr:&[u8])->String{
    let code=status.code().map_or("unknown".into(),|c|c.to_string());
    let details=String::from_utf8_lossy(stderr);
    if details.trim().is_empty(){format!("{name} failed (exit code {code}); stderr empty")}
    else{format!("{name} failed (exit code {code}): {}",details.trim())}
}
fn sanitize(s:&str)->String{
    let cleaned=Regex::new(r#"[<>:"/\\|?*]"#).unwrap().replace_all(s,"_").to_string();
    let result:String=cleaned.trim_matches(|c:char|c=='.'||c.is_whitespace()).chars().take(100).collect();
    if result.is_empty(){"Unknown Track".into()}else{result}
}
fn decode(s:&str)->String{s.replace("&amp;","&").replace("&quot;","\"").replace("&#x27;","'").replace("&#39;","'").replace("&lt;","<").replace("&gt;",">")}
struct Metadata{title:String,artist:String,tags:Vec<String>,cover:Option<String>}

#[cfg(test)]
mod tests{
    use super::*;
    #[test] fn sanitizes_windows_filename(){assert_eq!(sanitize(r#"a:b/c*?"#),"a_b_c__");assert_eq!(sanitize("..."),"Unknown Track");}
    #[test] fn decodes_entities(){assert_eq!(decode("A &amp; B &#x27;x&#x27;"),"A & B 'x'");}
}
