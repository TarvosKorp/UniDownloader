# UniDownloader

UniDownloader uses one Windows executable as a launcher and module host. Each downloader is built as a separate DLL and is started in its own process through a shortcut.

## Build on GitHub Actions

All three build workflows are **manual only**. They do not run on pushes or pull requests.

Open the repository's **Actions** tab, choose the required workflow from the left sidebar, click **Run workflow**, select branch `main`, and confirm **Run workflow**.

| Workflow | Output artifact |
|---|---|
| **Build Core** (`.github/workflows/build.yml`) | `UniDownloader-Core-Windows` — only `UniDownloader.exe` |
| **Build Udio Module** (`.github/workflows/build-udio.yml`) | `UniDownloader-Udio-Windows` — Udio DLL and manifest |
| **Build YouTube Module** (`.github/workflows/build-youtube.yml`) | `UniDownloader-YouTube-Windows` — YouTube DLL and manifest |

After a workflow finishes successfully, open its run, scroll to **Artifacts**, and download the corresponding ZIP. Build the Core first, then build the module DLLs. Extract the three artifacts into the same runtime folder, preserving the `Modules/udio` and `Modules/youtube` subfolders.

The workflows use Windows runners and the stable Rust MSVC toolchain. Nothing needs to be installed or built on your computer.

## Runtime folder layout

Place the files beside `UniDownloader.exe` in this layout:

```text
UniDownloader/
├── UniDownloader.exe
├── config.ini
├── Components/
│   ├── ffmpeg.exe
│   ├── yt-dlp.exe
│   ├── mp4decrypt.exe
│   └── WidevineKey.wvd
├── Modules/
│   ├── udio/
│   │   ├── module.json
│   │   └── module_udio.dll
│   └── youtube/
│       ├── module.json
│       └── module_youtube.dll
└── Downloads/
```

Each module artifact contains its own `module.json` beside the matching DLL. The Core artifact contains only `UniDownloader.exe`; it does not package the `Modules` directory or `Icon.ico`. Create the `Modules/<id>/` folders by extracting the two module artifacts.

## Launching modules

1. Start `UniDownloader.exe`.
2. Select **Create / refresh module shortcuts**.
3. Open the generated Udio or YouTube shortcut.

A shortcut starts the same executable with `--module <id>`. The DLL supplies its UI descriptor and download implementation. The core renders the descriptor using a generic UI host; it does not contain Udio- or YouTube-specific widgets.

Each module runs in a separate process. Different modules can run simultaneously. A Windows named mutex prevents a second process from opening the same module while its first instance is running.

## Configuration

The application reads `config.ini` beside the executable. Proxy settings are enabled only when `USE_PROXY=true`.

Example:

```ini
USE_PROXY=false
HTTP_PROXY=
HTTPS_PROXY=
FFMPEG_PATH=
```

If `FFMPEG_PATH` is empty or absent, the application uses `Components/ffmpeg.exe`.

## Batch lists and output

- Udio list: `DownloadudioList.txt`
- YouTube list: `DownloadyoutubeList.txt`
- Output: `Downloads/<module-id>/`
- Error diagnostics: `ErrorsLog.txt`

Batch tracks are processed sequentially. After each completed track, the remaining list is written through a temporary file and replacement. If the process stops while a track is downloading, that current URL remains in the list for the next run. Lines beginning with `BadLink ` are retained as failed entries and skipped on subsequent batch runs.

Udio may download up to four segments of the current track concurrently. It never downloads separate batch tracks concurrently. Output filenames are reserved without overwriting existing tracks.

## Module ABI

The current ABI version is **2**. Each DLL exports:

- `module_abi_version`
- `module_ui_json` — a static, null-terminated JSON UI descriptor
- `run_download` — the C ABI download entry point

The core rejects modules with a mismatched ABI version. Do not copy an old DLL into a new runtime package.
