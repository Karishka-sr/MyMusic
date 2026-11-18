mod engine;
mod library;
mod platform;
mod settings;

slint::include_modules!();

use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use slint::{
    ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    TimerMode, VecModel,
};

use engine::{Engine, Repeat};
use library::Track;
use settings::Settings;

/// Всё изменяемое состояние приложения. Живёт в потоке интерфейса.
struct State {
    data_dir: Option<PathBuf>,
    settings: Settings,
    /// Всё, что нашли на телефоне.
    all: Vec<Track>,
    /// То, что видно в списке (после фильтров, скрытия, поиска и сортировки).
    visible: Vec<Track>,
    /// Скрытые вручную треки.
    hidden: HashSet<PathBuf>,
    query: String,
    engine: Option<Engine>,
    /// Трек, данные которого сейчас выведены на экран (чтобы не перегружать обложку зря).
    shown_path: Option<PathBuf>,
    permission_granted: bool,
    scan_started: bool,
    scanning: bool,
    /// Сколько тиков ещё показывать сообщение внизу.
    message_ttl: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        data_dir: None,
        settings: Settings::default(),
        all: Vec::new(),
        visible: Vec::new(),
        hidden: HashSet::new(),
        query: String::new(),
        engine: None,
        shown_path: None,
        permission_granted: false,
        scan_started: false,
        scanning: false,
        message_ttl: 0,
    });
}

#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    let data_dir = app.internal_data_path();
    slint::android::init(app).unwrap();
    run(data_dir);
}

fn run(data_dir: Option<PathBuf>) {
    let ui = MainWindow::new().unwrap();

    STATE.with_borrow_mut(|s| {
        s.settings = Settings::load(&data_dir);
        s.hidden = settings::load_hidden(&data_dir);
        s.data_dir = data_dir;
    });
    apply_settings_to_ui(&ui);

    ui.on_request_permission(platform::request_audio_permission);

    let w = ui.as_weak();
    ui.on_play_track(move |index| play_index(&w.unwrap(), index as usize));

    let w = ui.as_weak();
    ui.on_toggle_play(move || with_engine(&w.unwrap(), |e| e.toggle_play()));

    let w = ui.as_weak();
    ui.on_next(move || with_engine(&w.unwrap(), |e| e.next(false)));

    let w = ui.as_weak();
    ui.on_previous(move || with_engine(&w.unwrap(), |e| e.previous()));

    let w = ui.as_weak();
    ui.on_seek(move |secs| with_engine(&w.unwrap(), |e| e.seek(secs)));

    let w = ui.as_weak();
    ui.on_toggle_shuffle(move || with_engine(&w.unwrap(), |e| e.toggle_shuffle()));

    let w = ui.as_weak();
    ui.on_cycle_repeat(move || with_engine(&w.unwrap(), |e| e.cycle_repeat()));

    let w = ui.as_weak();
    ui.on_search_changed(move |text| {
        STATE.with_borrow_mut(|s| s.query = text.to_string());
        refresh_list(&w.unwrap());
    });

    let w = ui.as_weak();
    ui.on_settings_changed(move || {
        let ui = w.unwrap();
        STATE.with_borrow_mut(|s| {
            s.settings = Settings {
                theme_mode: ui.get_theme_mode(),
                hide_messengers: ui.get_hide_messengers(),
                hide_recordings: ui.get_hide_recordings(),
                min_length: ui.get_min_length(),
                sort_mode: ui.get_sort_mode(),
                auto_open_player: ui.get_auto_open_player(),
            };
            s.settings.save(&s.data_dir);
        });
        refresh_list(&ui);
    });

    let w = ui.as_weak();
    ui.on_rescan(move || start_scan(&w.unwrap()));

    let w = ui.as_weak();
    ui.on_hide_track(move |index| hide_index(&w.unwrap(), index as usize));

    let w = ui.as_weak();
    ui.on_delete_track(move |index| delete_index(&w.unwrap(), index as usize));

    let w = ui.as_weak();
    ui.on_unhide_all(move || unhide_all(&w.unwrap()));

    // Первая проверка до показа окна, чтобы экран «нужен доступ» не мигал, если доступ уже есть.
    tick(&ui);

    // Четыре раза в секунду: позиция трека, переход на следующий, проверка разрешения.
    let timer = Timer::default();
    let w = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(ui) = w.upgrade() {
            tick(&ui);
        }
    });

    ui.run().unwrap();
}

// ───────────────────────── Периодическая работа ─────────────────────────

fn tick(ui: &MainWindow) {
    let (granted, need_scan) = STATE.with_borrow_mut(|s| {
        if !s.permission_granted && platform::has_audio_permission() {
            s.permission_granted = true;
        }
        let need = s.permission_granted && !s.scan_started;
        if need {
            s.scan_started = true;
        }
        (s.permission_granted, need)
    });
    ui.set_permission_granted(granted);
    if need_scan {
        start_scan(ui);
    }

    STATE.with_borrow_mut(|s| {
        if s.message_ttl > 0 {
            s.message_ttl -= 1;
            if s.message_ttl == 0 {
                ui.set_message(SharedString::new());
            }
        }
        if let Some(e) = s.engine.as_mut() {
            e.tick();
        }
        sync_ui(ui, s);
    });
}

// ───────────────────────── Библиотека ─────────────────────────

fn start_scan(ui: &MainWindow) {
    let already = STATE.with_borrow_mut(|s| std::mem::replace(&mut s.scanning, true));
    if already {
        return;
    }
    ui.set_loading(true);

    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let tracks = library::scan();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            STATE.with_borrow_mut(|s| {
                s.all = tracks;
                s.scanning = false;
            });
            ui.set_loading(false);
            refresh_list(&ui);
        });
    });
}

/// Пересобирает список на экране: фильтры, скрытые, поиск, сортировка.
fn refresh_list(ui: &MainWindow) {
    let (items, total, hidden) = STATE.with_borrow_mut(|s| {
        s.visible = library::apply_filters(&s.all, &s.settings, &s.hidden, &s.query);
        let by_artist = s.settings.sort_mode == 1;
        let items: Vec<TrackItem> = s
            .visible
            .iter()
            .map(|t| TrackItem {
                key: key_of(&t.path),
                title: t.title.as_str().into(),
                artist: t.artist.as_str().into(),
                duration: library::format_time(t.duration).into(),
                letter: library::letter(if by_artist { &t.artist } else { &t.title }).into(),
            })
            .collect();
        (items, s.all.len(), s.hidden.len())
    });
    ui.set_total_count(total as i32);
    ui.set_hidden_count(hidden as i32);
    ui.set_tracks(ModelRc::new(VecModel::from(items)));
}

fn key_of(path: &Path) -> SharedString {
    path.to_string_lossy().as_ref().into()
}

fn apply_settings_to_ui(ui: &MainWindow) {
    STATE.with_borrow(|s| {
        let c = &s.settings;
        ui.set_theme_mode(c.theme_mode);
        ui.set_hide_messengers(c.hide_messengers);
        ui.set_hide_recordings(c.hide_recordings);
        ui.set_min_length(c.min_length);
        ui.set_sort_mode(c.sort_mode);
        ui.set_auto_open_player(c.auto_open_player);
    });
}

// ───────────────────────── Скрыть и удалить ─────────────────────────

fn show_message(ui: &MainWindow, text: &str) {
    ui.set_message(text.into());
    STATE.with_borrow_mut(|s| s.message_ttl = 20); // ~5 секунд
}

/// Прячет трек из списка. Файл остаётся на телефоне, вернуть можно в настройках.
fn hide_index(ui: &MainWindow, index: usize) {
    let title = STATE.with_borrow_mut(|s| {
        let track = s.visible.get(index)?.clone();
        s.hidden.insert(track.path.clone());
        settings::save_hidden(&s.data_dir, &s.hidden);
        Some(track.title)
    });
    refresh_list(ui);
    if let Some(title) = title {
        show_message(ui, &format!("Скрыто: {title}. Вернуть можно в настройках"));
    }
}

fn unhide_all(ui: &MainWindow) {
    STATE.with_borrow_mut(|s| {
        s.hidden.clear();
        settings::save_hidden(&s.data_dir, &s.hidden);
    });
    refresh_list(ui);
    show_message(ui, "Скрытые треки снова в списке");
}

/// Удаляет файл с телефона насовсем (подтверждение уже получено на экране).
fn delete_index(ui: &MainWindow, index: usize) {
    let message = STATE.with_borrow_mut(|s| {
        let track = s.visible.get(index)?.clone();

        if !platform::has_all_files_access() {
            platform::request_all_files_access();
            return Some(
                "Включи «Доступ ко всем файлам» для приложения, вернись сюда и повтори удаление"
                    .to_string(),
            );
        }

        Some(match library::delete_file(&track.path) {
            Ok(()) => {
                platform::notify_file_removed(&track.path);
                s.all.retain(|t| t.path != track.path);
                if s.hidden.remove(&track.path) {
                    settings::save_hidden(&s.data_dir, &s.hidden);
                }
                if let Some(e) = s.engine.as_mut() {
                    e.stop_if_current(&track.path);
                }
                format!("Удалено: {}", track.title)
            }
            Err(err) => format!("Не удалось удалить файл: {err}"),
        })
    });

    refresh_list(ui);
    STATE.with_borrow_mut(|s| sync_ui(ui, s));
    if let Some(message) = message {
        show_message(ui, &message);
    }
}

// ───────────────────────── Воспроизведение ─────────────────────────

fn play_index(ui: &MainWindow, index: usize) {
    STATE.with_borrow_mut(|s| {
        if index >= s.visible.len() {
            return;
        }
        if s.engine.is_none() {
            s.engine = Engine::new();
        }
        let queue = s.visible.clone();
        let auto_open = s.settings.auto_open_player;
        if let Some(e) = s.engine.as_mut() {
            e.play_queue(queue, index);
            if auto_open {
                ui.set_player_open(true); // окно плеера открывается само
            }
        }
        sync_ui(ui, s);
    });
}

fn with_engine(ui: &MainWindow, f: impl FnOnce(&mut Engine)) {
    STATE.with_borrow_mut(|s| {
        if let Some(e) = s.engine.as_mut() {
            f(e);
        }
        sync_ui(ui, s);
    });
}

/// Переносит состояние плеера на экран.
fn sync_ui(ui: &MainWindow, s: &mut State) {
    let Some(engine) = s.engine.as_ref() else { return };

    let current_path = engine.current.as_ref().map(|t| t.path.clone());
    if current_path != s.shown_path {
        match &engine.current {
            Some(t) => {
                ui.set_has_current(true);
                ui.set_current_key(key_of(&t.path));
                ui.set_current_title(t.title.as_str().into());
                ui.set_current_artist(t.artist.as_str().into());
                ui.set_has_cover(false);
                ui.set_current_cover(Image::default());
                load_cover_async(ui.as_weak(), t.path.clone());
            }
            None => {
                ui.set_has_current(false);
                ui.set_player_open(false);
                ui.set_current_key(SharedString::new());
            }
        }
        s.shown_path = current_path;
    }

    ui.set_playing(engine.is_playing());
    ui.set_play_position(engine.position().as_secs_f32());
    ui.set_track_length(engine.length.as_secs_f32());
    ui.set_shuffle(engine.shuffle);
    ui.set_repeat_mode(match engine.repeat {
        Repeat::Off => 0,
        Repeat::All => 1,
        Repeat::One => 2,
    });
}

/// Обложку читаем в отдельном потоке, чтобы интерфейс не подвисал.
fn load_cover_async(ui: slint::Weak<MainWindow>, path: PathBuf) {
    std::thread::spawn(move || {
        let Some((raw, w, h)) = library::load_cover(&path) else { return };
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&raw, w, h);
        let _ = ui.upgrade_in_event_loop(move |ui| {
            // пока читали, трек могли сменить — тогда обложка уже не нужна
            let still_current =
                STATE.with_borrow(|s| s.shown_path.as_deref() == Some(path.as_path()));
            if still_current {
                ui.set_current_cover(Image::from_rgba8(buffer));
                ui.set_has_cover(true);
            }
        });
    });
}
