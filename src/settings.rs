//! Настройки приложения. Хранятся в простом текстовом файле `settings.txt`
//! во внутренней папке приложения (строки вида `ключ=значение`).

use std::{collections::HashSet, fs, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Settings {
    /// 0 — как в системе, 1 — светлая, 2 — тёмная
    pub theme_mode: i32,
    pub hide_messengers: bool,
    pub hide_recordings: bool,
    /// 0 — показывать все, 1 — от 30 с, 2 — от 1 мин, 3 — от 2 мин
    pub min_length: i32,
    /// 0 — по названию, 1 — по исполнителю
    pub sort_mode: i32,
    pub auto_open_player: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_mode: 0,
            hide_messengers: true,
            hide_recordings: true,
            min_length: 0,
            sort_mode: 0,
            auto_open_player: true,
        }
    }
}

impl Settings {
    pub fn min_seconds(&self) -> u64 {
        match self.min_length {
            1 => 30,
            2 => 60,
            3 => 120,
            _ => 0,
        }
    }

    pub fn load(dir: &Option<PathBuf>) -> Settings {
        let mut s = Settings::default();
        let Some(path) = file(dir) else { return s };
        let Ok(text) = fs::read_to_string(path) else { return s };

        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "theme_mode" => s.theme_mode = value.parse().unwrap_or(s.theme_mode).clamp(0, 2),
                "hide_messengers" => s.hide_messengers = value.parse().unwrap_or(s.hide_messengers),
                "hide_recordings" => s.hide_recordings = value.parse().unwrap_or(s.hide_recordings),
                "min_length" => s.min_length = value.parse().unwrap_or(s.min_length).clamp(0, 3),
                "sort_mode" => s.sort_mode = value.parse().unwrap_or(s.sort_mode).clamp(0, 1),
                "auto_open_player" => {
                    s.auto_open_player = value.parse().unwrap_or(s.auto_open_player)
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, dir: &Option<PathBuf>) {
        let Some(path) = file(dir) else { return };
        let text = format!(
            "theme_mode={}\nhide_messengers={}\nhide_recordings={}\nmin_length={}\nsort_mode={}\nauto_open_player={}\n",
            self.theme_mode,
            self.hide_messengers,
            self.hide_recordings,
            self.min_length,
            self.sort_mode,
            self.auto_open_player,
        );
        let _ = fs::write(path, text);
    }
}

/// Список треков, скрытых вручную (файлы остаются на телефоне). Файл `hidden.txt`, по пути в строке.
pub fn load_hidden(dir: &Option<PathBuf>) -> HashSet<PathBuf> {
    let Some(d) = dir else { return HashSet::new() };
    fs::read_to_string(d.join("hidden.txt"))
        .map(|text| {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_hidden(dir: &Option<PathBuf>, hidden: &HashSet<PathBuf>) {
    let Some(d) = dir else { return };
    let text: String = hidden
        .iter()
        .map(|p| format!("{}\n", p.to_string_lossy()))
        .collect();
    let _ = fs::write(d.join("hidden.txt"), text);
}

fn file(dir: &Option<PathBuf>) -> Option<PathBuf> {
    dir.as_ref().map(|d| d.join("settings.txt"))
}
