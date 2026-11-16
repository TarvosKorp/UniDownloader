const tauri = window.__TAURI__;
const statusDiv = document.getElementById('status');
const moduleSelect = document.getElementById('module-select');
const urlInput = document.getElementById('url-input');
const qualitySelect = document.getElementById('quality-select');
const bitrateGroup = document.getElementById('bitrate-group');
const bitrateSelect = document.getElementById('bitrate-select');
const downloadBtn = document.getElementById('download-btn');
const batchBtn = document.getElementById('batch-btn');
const openListBtn = document.getElementById('open-list-btn');
const openFolderBtn = document.getElementById('open-folder-btn');
const createShortcutsBtn = document.getElementById('create-shortcuts-btn');

if (!tauri) {
    moduleSelect.innerHTML = '<option value="">Tauri API unavailable</option>';
    throw new Error('window.__TAURI__ is undefined');
}

const { invoke } = tauri.core;

// Показываем/скрываем битрейт при выборе MP3
qualitySelect.addEventListener('change', () => {
    if (qualitySelect.value === 'MP3') {
        bitrateGroup.style.display = 'block';
    } else {
        bitrateGroup.style.display = 'none';
    }
});

async function loadModules() {
    try {
        const modules = await invoke('get_modules');
        moduleSelect.innerHTML = '';
        if (modules.length === 0) {
            moduleSelect.innerHTML = '<option value="">No modules found</option>';
            return;
        }
        modules.forEach(m => {
            const opt = document.createElement('option');
            opt.value = m.id;
            opt.textContent = m.name;
            moduleSelect.appendChild(opt);
        });
    } catch (e) {
        console.error(e);
        moduleSelect.innerHTML = '<option value="">Error loading modules</option>';
    }
}

// Одиночное скачивание
downloadBtn.addEventListener('click', async () => {
    const module = moduleSelect.value;
    const url = urlInput.value.trim();
    if (!module || !url) {
        statusDiv.textContent = "Please select module and enter URL.";
        statusDiv.style.color = "#ff4d4d";
        return;
    }

    downloadBtn.disabled = true;
    batchBtn.disabled = true;
    statusDiv.textContent = "Downloading...";
    statusDiv.style.color = "#00a8ff";

    try {
        const bitrate = qualitySelect.value === 'MP3' ? bitrateSelect.value : null;
        const res = await invoke('run_download', { 
            moduleId: module, 
            url: url, 
            quality: qualitySelect.value,
            bitrate: bitrate 
        });
        statusDiv.textContent = res;
        statusDiv.style.color = "#00c864";
        urlInput.value = '';
    } catch (e) {
        statusDiv.textContent = `Error: ${e}`;
        statusDiv.style.color = "#ff4d4d";
    } finally {
        downloadBtn.disabled = false;
        batchBtn.disabled = false;
    }
});

// Пакетное скачивание
batchBtn.addEventListener('click', async () => {
    const module = moduleSelect.value;
    if (!module) return;

    downloadBtn.disabled = true;
    batchBtn.disabled = true;
    statusDiv.textContent = "Batch processing...";
    statusDiv.style.color = "#00a8ff";

    try {
        const bitrate = qualitySelect.value === 'MP3' ? bitrateSelect.value : null;
        const res = await invoke('batch_download', { 
            moduleId: module, 
            quality: qualitySelect.value,
            bitrate: bitrate 
        });
        statusDiv.textContent = res;
        statusDiv.style.color = "#00c864";
    } catch (e) {
        statusDiv.textContent = `Error: ${e}`;
        statusDiv.style.color = "#ff4d4d";
    } finally {
        downloadBtn.disabled = false;
        batchBtn.disabled = false;
    }
});

// Открыть список
openListBtn.addEventListener('click', async () => {
    const module = moduleSelect.value;
    if (!module) return;
    try {
        const exeDir = await invoke('get_exe_dir');
        const path = `${exeDir}\\Download${module.charAt(0).toUpperCase() + module.slice(1)}List.txt`;
        await invoke('open_file', { path });
    } catch (e) {
        alert("Failed to open list file: " + e);
    }
});

// Открыть папку загрузок
openFolderBtn.addEventListener('click', async () => {
    const module = moduleSelect.value;
    if (!module) return;
    try {
        await invoke('open_folder', { moduleId: module });
    } catch (e) {
        alert("Failed to open folder: " + e);
    }
});

createShortcutsBtn.addEventListener('click', async () => {
    createShortcutsBtn.disabled = true;
    statusDiv.textContent = "Creating shortcuts...";
    try {
        const res = await invoke('create_shortcuts');
        statusDiv.textContent = res;
        statusDiv.style.color = "#00c864";
    } catch (e) {
        statusDiv.textContent = `Error: ${e}`;
        statusDiv.style.color = "#ff4d4d";
    } finally {
        createShortcutsBtn.disabled = false;
    }
});

loadModules();