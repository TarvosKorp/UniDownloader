const { invoke } = window.__TAURI__.core;

const moduleSelect = document.getElementById('module-select');
const createBtn = document.getElementById('create-shortcuts-btn');
const statusDiv = document.getElementById('status');

// Загружаем модули при старте
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

        // Выбираем первый по умолчанию
        moduleSelect.selectedIndex = 0;
    } catch (e) {
        console.error(e);
        moduleSelect.innerHTML = '<option value="">Error loading modules</option>';
    }
}

// Создание ярлыков
createBtn.addEventListener('click', async () => {
    createBtn.disabled = true;
    statusDiv.textContent = "Creating shortcuts...";
    
    try {
        const result = await invoke('create_shortcuts');
        statusDiv.textContent = result;
        statusDiv.style.color = "#00c864"; // Зеленый успех
    } catch (e) {
        statusDiv.textContent = `Error: ${e}`;
        statusDiv.style.color = "#ff4d4d"; // Красная ошибка
    } finally {
        createBtn.disabled = false;
        setTimeout(() => {
            if (statusDiv.style.color !== "#ff4d4d") statusDiv.textContent = "";
        }, 3000);
    }
});

// Запуск
loadModules();