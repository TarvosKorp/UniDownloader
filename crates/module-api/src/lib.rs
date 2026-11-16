//! Общий API для всех модулей скачивания.

use serde::{Deserialize, Serialize};

/// Опции, которые пользователь выбирает в интерфейсе.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadOptions {
    /// Например, "360p", "1080p", "mp3".
    pub quality: String,
    /// Битрейт для аудио (если применимо).
    pub bitrate: Option<String>,
}

/// Трейт, который должен реализовать каждый модуль (Udio, YouTube и т.д.).
pub trait DownloaderModule: Send + Sync {
    /// Человекочитаемое имя (для UI).
    fn name(&self) -> &'static str;

    /// Уникальный ID (для аргумента `--module` и ярлыков).
    fn id(&self) -> &'static str;

    /// Основной метод скачивания.
    /// Возвращает Ok(()) при успехе, Err(String) с текстом ошибки при неудаче.
    fn download(&self, url: &str, options: &DownloadOptions) -> Result<(), String>;
}

/// Вспомогательная структура для регистрации модулей в ядре.
pub struct ModuleInfo {
    pub name: &'static str,
    pub id: &'static str,
}