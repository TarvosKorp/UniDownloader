const tauri = window.__TAURI__;
const statusDiv = document.getElementById('status');
const moduleSelect = document.getElementById('module-select');
const createBtn = document.getElementById('create-shortcuts-btn');

if (!tauri) {
    moduleSelect.innerHTML = '<option value="">Tauri API unavailable</option>';
    throw new Error('window.__TAURI__ is undefined (withGlobalTauri?)');
}

const { invoke } = tauri.core;

async function loadModules() {
    try {
        const modules = await invoke('get_modules');
        console.log("Modules from Rust:", modules);
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
        console.error("Failed to load modules:", e);
        moduleSelect.innerHTML = '<option value="">Error loading modules</option>';
    }
}

createBtn.addEventListener('click', async () => {
    createBtn.disabled = true;
    statusDiv.textContent = "Creating shortcuts...";
    statusDiv.style.color = "#00a8ff";
    
    try {
        const result = await invoke('create_shortcuts');
        statusDiv.textContent = result;
        statusDiv.style.color = "#00c864";
    } catch (e) {
        statusDiv.textContent = `Error: ${e}`;
        statusDiv.style.color = "#ff4d4d";
    } finally {
        createBtn.disabled = false;
        setTimeout(() => {
            if (statusDiv.style.color !== "#ff4d4d") statusDiv.textContent = "";
        }, 3000);
    }
});

// Запуск
loadModules();