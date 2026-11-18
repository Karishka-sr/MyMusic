//! Проигрыватель: очередь, перемешивание, повтор, перемотка. Звук — через rodio.
//! Всё живёт в потоке интерфейса (аудиопоток cpal нельзя передавать между потоками).

use std::{
    fs::File,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::library::Track;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    Off,
    All,
    One,
}

pub struct Engine {
    /// Выход на динамики. Пока он жив — звук играет; если убрать — замолчит.
    sink: MixerDeviceSink,
    /// Плеер текущего трека. `None` — ничего не играет (конец очереди или файл не открылся).
    player: Option<Player>,
    queue: Vec<Track>,
    /// Порядок воспроизведения: индексы в `queue` (по порядку или перемешанные).
    order: Vec<usize>,
    cursor: usize,
    rng: u64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub current: Option<Track>,
    pub length: Duration,
}

impl Engine {
    pub fn new() -> Option<Engine> {
        let sink = DeviceSinkBuilder::open_default_sink().ok()?;
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Some(Engine {
            sink,
            player: None,
            queue: Vec::new(),
            order: Vec::new(),
            cursor: 0,
            rng: seed | 1,
            shuffle: false,
            repeat: Repeat::Off,
            current: None,
            length: Duration::ZERO,
        })
    }

    // ───────── Управление ─────────

    /// Начинает играть `index`-й трек, а очередью становится весь переданный список.
    pub fn play_queue(&mut self, queue: Vec<Track>, index: usize) {
        if queue.is_empty() {
            return;
        }
        let index = index.min(queue.len() - 1);
        self.queue = queue;
        if self.shuffle {
            self.reshuffle(Some(index));
        } else {
            self.order = (0..self.queue.len()).collect();
            self.cursor = index;
        }
        self.start();
    }

    pub fn toggle_play(&mut self) {
        if let Some(p) = &self.player {
            if p.is_paused() {
                p.play();
            } else {
                p.pause();
            }
            return;
        }
        // стоим после конца очереди — играем текущий трек сначала
        if self.current.is_some() {
            self.start();
        }
    }

    /// `auto` = трек доиграл сам (тогда учитываем повтор одного трека и конец очереди).
    pub fn next(&mut self, auto: bool) {
        let n = self.order.len();
        if n == 0 {
            return;
        }
        if auto && self.repeat == Repeat::One {
            self.start();
            return;
        }
        if self.cursor + 1 < n {
            self.cursor += 1;
        } else if self.repeat == Repeat::All || !auto {
            if self.shuffle {
                self.reshuffle(None);
            }
            self.cursor = 0;
        } else {
            // очередь закончилась, повтор выключен — останавливаемся
            self.player = None;
            return;
        }
        self.start();
    }

    pub fn previous(&mut self) {
        if self.order.is_empty() {
            return;
        }
        let at_start = self.cursor == 0 && self.repeat != Repeat::All;
        if self.position() > Duration::from_secs(3) || at_start {
            // как в обычных плеерах: в начале трека — предыдущий, дальше — перемотка в начало
            if self.player.is_some() {
                self.seek(0.0);
            } else {
                self.start();
            }
            return;
        }
        self.cursor = if self.cursor == 0 { self.order.len() - 1 } else { self.cursor - 1 };
        self.start();
    }

    pub fn seek(&mut self, secs: f32) {
        if let Some(p) = &self.player {
            let _ = p.try_seek(Duration::from_secs_f32(secs.max(0.0)));
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.queue.is_empty() || self.order.is_empty() {
            return;
        }
        let current_in_queue = self.order[self.cursor.min(self.order.len() - 1)];
        if self.shuffle {
            self.reshuffle(Some(current_in_queue));
        } else {
            self.order = (0..self.queue.len()).collect();
            self.cursor = current_in_queue;
        }
    }

    /// Повтор: выкл → весь список → один трек → выкл.
    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
    }

    /// Вызывается несколько раз в секунду: если трек доиграл — включает следующий.
    pub fn tick(&mut self) {
        let finished = self.player.as_ref().map(|p| p.empty()).unwrap_or(false);
        if finished {
            self.next(true);
        }
    }

    /// Файл удалили с телефона: если он сейчас играет — останавливаем плеер.
    pub fn stop_if_current(&mut self, path: &Path) {
        let is_current = self
            .current
            .as_ref()
            .map(|t| t.path.as_path() == path)
            .unwrap_or(false);
        if is_current {
            self.player = None;
            self.current = None;
            self.length = Duration::ZERO;
        }
    }

    // ───────── Состояние ─────────

    pub fn position(&self) -> Duration {
        self.player.as_ref().map(|p| p.get_pos()).unwrap_or(Duration::ZERO)
    }

    pub fn is_playing(&self) -> bool {
        self.player
            .as_ref()
            .map(|p| !p.is_paused() && !p.empty())
            .unwrap_or(false)
    }

    // ───────── Внутреннее ─────────

    /// Запускает трек под курсором. Если файл не открывается — пробует следующие.
    fn start(&mut self) {
        self.player = None; // старый плеер при удалении сам замолкает
        let n = self.order.len();
        for _ in 0..n {
            let Some(&qi) = self.order.get(self.cursor) else { return };
            let track = self.queue[qi].clone();
            if let Some((player, len)) = open(&self.sink, &track) {
                self.length = len.unwrap_or(track.duration);
                self.player = Some(player);
                self.current = Some(track);
                return;
            }
            self.current = Some(track);
            self.cursor = (self.cursor + 1) % n;
        }
        self.length = Duration::ZERO;
    }

    fn reshuffle(&mut self, first: Option<usize>) {
        let n = self.queue.len();
        self.order = (0..n).collect();
        // тасование Фишера — Йетса
        for i in (1..n).rev() {
            let j = (self.rand() % (i as u64 + 1)) as usize;
            self.order.swap(i, j);
        }
        // выбранный трек ставим первым, чтобы он не сменился
        if let Some(f) = first {
            if let Some(p) = self.order.iter().position(|&x| x == f) {
                self.order.swap(0, p);
            }
        }
        self.cursor = 0;
    }

    fn rand(&mut self) -> u64 {
        // xorshift64 — для перемешивания хватает
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

fn open(sink: &MixerDeviceSink, track: &Track) -> Option<(Player, Option<Duration>)> {
    let file = File::open(&track.path).ok()?;
    let decoder = Decoder::try_from(file).ok()?;
    let length = decoder.total_duration();
    let player = Player::connect_new(sink.mixer());
    player.append(decoder);
    Some((player, length))
}
