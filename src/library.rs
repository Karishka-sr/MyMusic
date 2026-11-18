//! Поиск музыки на телефоне, чтение тегов, фильтры, обложки.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

use lofty::{
    config::ParseOptions,
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::Accessor,
};
use walkdir::WalkDir;

use crate::settings::Settings;

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
}

// ───────────────────────── Что считается «не музыкой» ─────────────────────────
// Сравнение идёт по пути файла, в нижнем регистре (русские буквы тоже).
// Чтобы что-то вернуть в список — удали строку.

/// Всегда скрываем: рингтоны, уведомления, будильники.
const SYSTEM_SOUNDS: &[&str] = &["/ringtones/", "/notifications/", "/alarms/"];

const MESSENGERS: &[&str] = &[
    "telegram",
    "whatsapp",
    "viber",
    "vkontakte",
    "/vk/",
    "/signal/",
    "skype",
    "discord",
    "com.facebook.orca",
];

const RECORDINGS: &[&str] = &[
    "/recordings/",
    "/recorder/",
    "/recording/",
    "voice recorder",
    "voicerecorder",
    "voice notes",
    "voice memos",
    "sound_recorder",
    "callrecord",
    "call_rec",
    "диктофон",
    "запис", // «запись», «записи», «Записи звонков»
];

/// Форматы, которые умеет играть декодер.
const EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "m4a", "aac", "wav"];

// ───────────────────────── Сканирование ─────────────────────────

fn roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/storage/emulated/0")];
    // SD-карты лежат в /storage/<ID>
    if let Ok(entries) = fs::read_dir("/storage") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name != "emulated" && name != "self" {
                roots.push(entry.path());
            }
        }
    }
    roots
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Находит все аудиофайлы и читает теги в несколько потоков.
pub fn scan() -> Vec<Track> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in roots() {
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                // скрытые папки и Android/ (данные приложений) пропускаем
                !(name.starts_with('.') || (e.file_type().is_dir() && name == "Android"))
            });
        for entry in walker.filter_map(Result::ok) {
            if entry.file_type().is_file() && is_audio(entry.path()) {
                paths.push(entry.into_path());
            }
        }
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4);
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Track>> = Mutex::new(Vec::with_capacity(paths.len()));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    local.push(read_track(path));
                }
                out.lock().unwrap().extend(local);
            });
        }
    });

    let mut tracks = out.into_inner().unwrap();
    tracks.sort_by_key(|t| t.title.to_lowercase());
    tracks
}

fn read_track(path: &Path) -> Track {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Без названия".into());

    let mut track = Track {
        path: path.to_path_buf(),
        title: stem,
        artist: "Неизвестный исполнитель".into(),
        album: "Неизвестный альбом".into(),
        duration: Duration::ZERO,
    };

    let parsed = Probe::open(path)
        .ok()
        .and_then(|p| p.options(ParseOptions::new().read_cover_art(false)).read().ok());

    if let Some(tagged) = parsed {
        track.duration = tagged.properties().duration();
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
                track.title = t.trim().to_string();
            }
            if let Some(a) = tag.artist().filter(|a| !a.trim().is_empty()) {
                track.artist = a.trim().to_string();
            }
            if let Some(a) = tag.album().filter(|a| !a.trim().is_empty()) {
                track.album = a.trim().to_string();
            }
        }
    }
    track
}

// ───────────────────────── Фильтры и сортировка ─────────────────────────

/// Удаляет файл с телефона насовсем.
pub fn delete_file(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)
}

pub fn apply_filters(
    all: &[Track],
    settings: &Settings,
    hidden: &HashSet<PathBuf>,
    query: &str,
) -> Vec<Track> {
    let query = query.trim().to_lowercase();
    let min_len = Duration::from_secs(settings.min_seconds());

    let mut list: Vec<Track> = all
        .iter()
        .filter(|t| {
            if hidden.contains(&t.path) {
                return false;
            }
            let path = t.path.to_string_lossy().to_lowercase();
            if SYSTEM_SOUNDS.iter().any(|p| path.contains(p)) {
                return false;
            }
            if settings.hide_messengers && MESSENGERS.iter().any(|p| path.contains(p)) {
                return false;
            }
            if settings.hide_recordings && RECORDINGS.iter().any(|p| path.contains(p)) {
                return false;
            }
            // длительность 0 = не удалось прочитать, такие не прячем
            if t.duration > Duration::ZERO && t.duration < min_len {
                return false;
            }
            if !query.is_empty() {
                let hit = t.title.to_lowercase().contains(&query)
                    || t.artist.to_lowercase().contains(&query)
                    || t.album.to_lowercase().contains(&query);
                if !hit {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();

    if settings.sort_mode == 1 {
        list.sort_by_key(|t| (t.artist.to_lowercase(), t.title.to_lowercase()));
    } else {
        list.sort_by_key(|t| t.title.to_lowercase());
    }
    list
}

// ───────────────────────── Мелочи для интерфейса ─────────────────────────

pub fn format_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Буква для пузырька при быстрой прокрутке.
pub fn letter(s: &str) -> String {
    match s.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c).to_string(),
        _ => "#".to_string(),
    }
}

// ───────────────────────── Обложки ─────────────────────────

/// Возвращает RGBA-пиксели обложки и её размер: сначала из тегов, потом cover.jpg рядом с файлом.
pub fn load_cover(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = embedded_cover(path).or_else(|| folder_cover(path))?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = img.thumbnail(600, 600).to_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_cover_art(true))
        .read()
        .ok()?;
    tagged
        .tags()
        .iter()
        .flat_map(|tag| tag.pictures())
        .next()
        .map(|pic| pic.data().to_vec())
}

fn folder_cover(path: &Path) -> Option<Vec<u8>> {
    let dir = path.parent()?;
    for name in [
        "cover.jpg",
        "cover.png",
        "folder.jpg",
        "folder.png",
        "front.jpg",
        "album.jpg",
        "albumart.jpg",
    ] {
        if let Ok(bytes) = fs::read(dir.join(name)) {
            return Some(bytes);
        }
    }
    None
}
