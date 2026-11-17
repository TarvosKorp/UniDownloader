import requests
import re
import os
import sys
import subprocess
import shutil
import html
import configparser
from urllib.parse import urljoin
from mutagen.mp3 import MP3
from mutagen.id3 import ID3, APIC, TIT2, TPE1, TCON, WOAR, error

# ================= ПУТИ =================
if getattr(sys, 'frozen', False):
    SCRIPT_DIR = os.path.dirname(sys.executable)
    BUNDLE_DIR = os.path.join(SCRIPT_DIR, "_internal")
    if not os.path.exists(BUNDLE_DIR):
        BUNDLE_DIR = sys._MEIPASS
else:
    SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
    BUNDLE_DIR = SCRIPT_DIR

# Файлы-ресурсы (ключ, exe-шники) ищем в BUNDLE_DIR
WVD_PATH = os.path.join(BUNDLE_DIR, "WidevineKey.wvd")
MP4DECRYPT_PATH = os.path.join(BUNDLE_DIR, "mp4decrypt.exe")
FFMPEG_PATH = os.path.join(BUNDLE_DIR, "ffmpeg.exe")

# Пользовательские файлы (конфиг, список, папка загрузок) ищем в SCRIPT_DIR
CONFIG_FILE = os.path.join(SCRIPT_DIR, "config.ini")
LIST_FILE = os.path.join(SCRIPT_DIR, "_DownloadUdioList.txt")
BAD_LINK_PREFIX = "BadLink "

# ================= КОНФИГ =================
config = configparser.ConfigParser()

if not os.path.exists(CONFIG_FILE):
    # Создаем файл вручную, чтобы сохранить комментарии
    config_text = """[Network]
# Поставь True, если нужно использовать прокси
USE_PROXY = True
# USE_PROXY = False

HTTP_PROXY = http://192.168.43.1:10809
HTTPS_PROXY = http://192.168.43.1:10809

[Paths]
DOWNLOAD_DIR = Downloads
"""
    with open(CONFIG_FILE, 'w', encoding='utf-8') as f:
        f.write(config_text)
    
    # Создаем пустой файл списка
    with open(LIST_FILE, 'w', encoding='utf-8') as f:
        pass
        
    print(f"Создан шаблон конфига: {CONFIG_FILE}")
    print(f"Создан пустой список: {LIST_FILE}")
    print("Настройте прокси при необходимости и запустите снова.")
    input("Нажмите Enter для выхода...")
    sys.exit(0)

# Читаем конфиг (комментарии игнорируются)
config.read(CONFIG_FILE, encoding='utf-8')

# Настройки сети
USE_PROXY = config.getboolean('Network', 'USE_PROXY', fallback=False)
PROXIES = None
if USE_PROXY:
    PROXIES = {
        "http": config.get('Network', 'HTTP_PROXY', fallback='http://192.168.43.1:10809'),
        "https": config.get('Network', 'HTTPS_PROXY', fallback='http://192.168.43.1:10809')
    }

# Настройки путей
DOWNLOAD_DIR = config.get('Paths', 'DOWNLOAD_DIR', fallback='Downloads')
if not os.path.isabs(DOWNLOAD_DIR):
    DOWNLOAD_DIR = os.path.join(SCRIPT_DIR, DOWNLOAD_DIR)

LICENSE_URL = "https://stream.udio.com/drm/license?type=widevine"
# ====================================================

from pywidevine.cdm import Cdm
from pywidevine.device import Device
from pywidevine.pssh import PSSH

def get_pssh(manifest_text):
    match = re.search(r'URI="data:text/plain;base64,([^"]+)"', manifest_text)
    return match.group(1)

def get_license_key(pssh_b64):
    device = Device.load(WVD_PATH)
    cdm = Cdm.from_device(device)
    session_id = cdm.open()
    pssh = PSSH(pssh_b64)
    challenge = cdm.get_license_challenge(session_id, pssh)
    
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Content-Type": "application/octet-stream",
        "Origin": "https://www.udio.com",
        "Referer": "https://www.udio.com/"
    }
    
    lic_resp = requests.post(LICENSE_URL, data=challenge, headers=headers, proxies=PROXIES, timeout=30)
    lic_resp.raise_for_status()
    cdm.parse_license(session_id, lic_resp.content)
    
    content_key = None
    for key in cdm.get_keys(session_id):
        if key.type == "CONTENT":
            content_key = key.key.hex()
            break
    cdm.close(session_id)
    return content_key

def download_file(url, path):
    r = requests.get(url, proxies=PROXIES, stream=True, timeout=30)
    r.raise_for_status()
    with open(path, 'wb') as f:
        for chunk in r.iter_content(chunk_size=8192):
            f.write(chunk)

def fetch_metadata_and_manifest(track_url):
    headers = {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"}
    r = requests.get(track_url, headers=headers, proxies=PROXIES, timeout=30)
    r.raise_for_status()
    html_text = r.text

    uuid_match = re.search(r'\\?"track\\?"\s*:\s*\{\s*\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?"', html_text)
    if not uuid_match:
        uuid_match = re.search(r'\\?"id\\?"\s*:\s*\\?"([a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12})\\?"', html_text)
    if not uuid_match:
        raise ValueError("Не удалось найти внутренний ID трека")
    
    content_id = uuid_match.group(1)
    manifest_url = f"https://stream.udio.com/api/v2/audio-stream/content/{content_id}/manifest.m3u8"

    title_match = re.search(r'<meta\s+property=["\']og:title["\']\s+content=["\']([^"\']+)', html_text, re.IGNORECASE)
    image_match = re.search(r'<meta\s+property=["\']og:image["\']\s+content=["\']([^"\']+)', html_text, re.IGNORECASE)
    
    raw_title = title_match.group(1) if title_match else "Unknown Track"
    cover_url = image_match.group(1) if image_match else None

    raw_title = html.unescape(raw_title).replace(" | Udio", "").strip()
    if " - " in raw_title:
        artist, title = raw_title.split(" - ", 1)
    else:
        artist, title = "Unknown Artist", raw_title

    tags = []
    tags_match = re.search(r'\\?"tags\\?"\s*:\s*\[([^\]]*)\]', html_text)
    if tags_match:
        tags = [t.strip('"\\') for t in re.findall(r'\\?"([^"\\]+)\\?"', tags_match.group(1))]
        tags = [html.unescape(t) for t in tags]

    return manifest_url, {
        "title": title.strip(),
        "artist": artist.strip(),
        "tags": tags,
        "cover_url": cover_url
    }

def tag_mp3(file_path, metadata, cover_data, track_url):
    try:
        audio = MP3(file_path, ID3=ID3)
    except error:
        audio = MP3(file_path)
        audio.add_tags()

    audio.tags.add(TIT2(encoding=3, text=html.unescape(metadata['title'])))
    audio.tags.add(TPE1(encoding=3, text=html.unescape(metadata['artist'])))
    if metadata['tags']:
        audio.tags.add(TCON(encoding=3, text=", ".join(metadata['tags'])))
    audio.tags.add(WOAR(url=track_url))

    if cover_data:
        audio.tags.add(APIC(encoding=3, mime='image/jpeg', type=3, desc='Cover', data=cover_data))
    audio.save()

def get_unique_filename(base_name, extension):
    final_name = f"{base_name}{extension}"
    counter = 1
    while os.path.exists(os.path.join(DOWNLOAD_DIR, final_name)):
        final_name = f"{base_name} {counter}{extension}"
        counter += 1
    return final_name

def process_track(track_url):
    try:
        manifest_url, metadata = fetch_metadata_and_manifest(track_url)
        
        r = requests.get(manifest_url, proxies=PROXIES, timeout=30)
        r.raise_for_status()
        manifest = r.text
        
        pssh_b64 = get_pssh(manifest)
        key = get_license_key(pssh_b64)

        base_url = "https://stream.udio.com"
        init_url = None
        segment_urls = []
        for line in manifest.splitlines():
            line = line.strip()
            if line.startswith('#EXT-X-MAP:'):
                uri = re.search(r'URI="([^"]+)"', line).group(1)
                init_url = urljoin(base_url, uri)
            elif line and not line.startswith('#'):
                segment_urls.append(urljoin(base_url, line))

        temp_dir = os.path.join(SCRIPT_DIR, "temp_segments")
        os.makedirs(temp_dir, exist_ok=True)
        
        if init_url:
            download_file(init_url, os.path.join(temp_dir, "init.mp4"))
        
        for i, url in enumerate(segment_urls):
            download_file(url, os.path.join(temp_dir, f"segment_{i+1}.m4s"))

        encrypted_path = os.path.join(temp_dir, "encrypted.mp4")
        with open(encrypted_path, "wb") as outfile:
            if init_url:
                with open(os.path.join(temp_dir, "init.mp4"), "rb") as infile:
                    outfile.write(infile.read())
            for i in range(len(segment_urls)):
                with open(os.path.join(temp_dir, f"segment_{i+1}.m4s"), "rb") as infile:
                    outfile.write(infile.read())

        decrypted_path = os.path.join(temp_dir, "decrypted.mp4")
        subprocess.run([MP4DECRYPT_PATH, "--key", f"1:{key}", encrypted_path, decrypted_path], check=True, capture_output=True)

        temp_mp3_name = os.path.join(SCRIPT_DIR, "temp_track.mp3")
        cmd = [FFMPEG_PATH, "-y", "-i", decrypted_path, "-vn", "-acodec", "libmp3lame", "-q:a", "2", temp_mp3_name]
        subprocess.run(cmd, check=True, capture_output=True)

        cover_data = None
        if metadata['cover_url']:
            try:
                img_resp = requests.get(metadata['cover_url'], proxies=PROXIES, timeout=30)
                img_resp.raise_for_status()
                cover_data = img_resp.content
            except:
                pass

        safe_title = re.sub(r'[<>:"/\\|?*]', '_', html.unescape(metadata['title']))[:100]
        final_filename = get_unique_filename(safe_title, ".mp3")
        final_path = os.path.join(DOWNLOAD_DIR, final_filename)
        
        os.rename(temp_mp3_name, final_path)
        tag_mp3(final_path, metadata, cover_data, track_url)

        shutil.rmtree(temp_dir)
        return True

    except Exception as e:
        print(f"    [!] Ошибка: {e}")
        if os.path.exists(os.path.join(SCRIPT_DIR, "temp_segments")):
            shutil.rmtree(os.path.join(SCRIPT_DIR, "temp_segments"))
        if os.path.exists(os.path.join(SCRIPT_DIR, "temp_track.mp3")):
            os.remove(os.path.join(SCRIPT_DIR, "temp_track.mp3"))
        return False

def main():
    if not os.path.exists(WVD_PATH):
        print(f"КРИТИЧЕСКАЯ ОШИБКА: Файл ключа не найден по пути {WVD_PATH}")
        print("Убедитесь, что файл WidevineKey.wvd собран в exe.")
        input("Нажмите Enter для выхода...")
        return

    # Проверка списка (создается вместе с конфигом при первом запуске)
    if not os.path.exists(LIST_FILE):
        print(f"Файл {LIST_FILE} не найден. Создаю пустой...")
        with open(LIST_FILE, 'w', encoding='utf-8') as f:
            pass
        print("Добавьте в него ссылки и запустите снова.")
        input("Нажмите Enter для выхода...")
        return

    os.makedirs(DOWNLOAD_DIR, exist_ok=True)

    with open(LIST_FILE, 'r', encoding='utf-8') as f:
        lines = f.readlines()

    total_valid = sum(1 for line in lines if line.strip() and not line.startswith(BAD_LINK_PREFIX))
    
    if total_valid == 0:
        print("Нет новых ссылок для скачивания.")
        input("Нажмите Enter для выхода...")
        return

    print(f"Найдено {total_valid} ссылок. Начинаю...\n")
    current_idx = 1

    for i, line in enumerate(lines):
        raw_line = line.strip()
        
        if not raw_line or raw_line.startswith(BAD_LINK_PREFIX):
            continue
            
        print(f"[{current_idx}/{total_valid}] Обработка: {raw_line}")
        
        success = process_track(raw_line)
        
        if success:
            print("    [+] Успешно скачано.")
            lines[i] = ""
        else:
            print("    [-] Не удалось скачать.")
            lines[i] = f"{BAD_LINK_PREFIX}{raw_line}\n"
        
        current_idx += 1
        
        with open(LIST_FILE, 'w', encoding='utf-8') as f:
            f.writelines([l for l in lines if l.strip() != ""])

    print("\nВсе задачи выполнены!")
    input("Нажмите Enter для выхода...")

if __name__ == "__main__":
    main()