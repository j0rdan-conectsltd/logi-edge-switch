//! Loop che osserva il cursore e cambia canale quando tocca un bordo esterno
//! o entra in un monitor condiviso con l'altro PC.

use crate::config::{Config, Rule};
use crate::hidpp::{self, LogiDevice};
use device_query::{DeviceQuery, DeviceState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        self.x <= px && px < self.x + self.w && self.y <= py && py < self.y + self.h
    }
    pub fn key(&self) -> String {
        format!("{},{}", self.x, self.y)
    }
}

/// Su macOS usa direttamente CoreGraphics: display-info passa da NSScreen, che va
/// usato solo dal thread principale, mentre il loop lo richiama da un altro thread (crash).
#[cfg(target_os = "macos")]
pub fn get_monitors() -> Vec<Rect> {
    use core_graphics::display::CGDisplay;
    CGDisplay::active_displays()
        .unwrap_or_default()
        .into_iter()
        .map(|id| {
            let b = CGDisplay::new(id).bounds();
            Rect { x: b.origin.x as i32, y: b.origin.y as i32, w: b.size.width as i32, h: b.size.height as i32 }
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn get_monitors() -> Vec<Rect> {
    display_info::DisplayInfo::all()
        .unwrap_or_default()
        .iter()
        .map(|d| Rect { x: d.x, y: d.y, w: d.width as i32, h: d.height as i32 })
        .collect()
}

/// Bordo su cui è appoggiato il cursore, se oltre quel bordo non c'è un altro
/// monitor di `monitors`. Gli angoli (entro `margin` px) sono ignorati.
pub fn outer_edge_hit(monitors: &[Rect], px: i32, py: i32, margin: i32) -> Option<&'static str> {
    let mon = monitors.iter().find(|m| m.contains(px, py))?;
    let free = |x, y| !monitors.iter().any(|m| m.contains(x, y));
    let (right, bottom) = (mon.x + mon.w - 1, mon.y + mon.h - 1);
    let in_v = mon.y + margin <= py && py <= bottom - margin;
    let in_h = mon.x + margin <= px && px <= right - margin;
    if px <= mon.x && in_v && free(px - 1, py) {
        Some("left")
    } else if px >= right && in_v && free(px + 1, py) {
        Some("right")
    } else if py <= mon.y && in_h && free(px, py - 1) {
        Some("top")
    } else if py >= bottom && in_h && free(px, py + 1) {
        Some("bottom")
    } else {
        None
    }
}

/// Bordo esterno entro `dist` px dal cursore: (bordo, distanza, punto del bordo davanti al cursore).
pub fn near_edge(monitors: &[Rect], px: i32, py: i32, margin: i32, dist: i32) -> Option<(&'static str, i32, (i32, i32))> {
    let m = monitors.iter().find(|m| m.contains(px, py))?;
    let (r, b) = (m.x + m.w - 1, m.y + m.h - 1);
    [("left", px - m.x, (m.x, py)), ("right", r - px, (r, py)), ("top", py - m.y, (px, m.y)), ("bottom", b - py, (px, b))]
        .into_iter()
        .filter(|&(e, d, (x, y))| d <= dist && outer_edge_hit(monitors, x, y, margin) == Some(e))
        .min_by_key(|&(_, d, _)| d)
}

/// Bagliore da disegnare: punto del bordo (px desktop), bordo, intensità 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub x: i32,
    pub y: i32,
    pub edge: &'static str,
    pub level: f32,
}

/// Riceve il bagliore dal loop (`None` = spegni); false = non ancora mostrabile, riprova. Impostato dalla GUI.
pub type GlowSink = Box<dyn Fn(Option<Glow>) -> bool + Send + Sync>;

/// Resistenza durante l'attesa sul bordo: lo spostamento verso l'interno è frenato
/// (il cursore fa 1/4 della strada); oltre `limit` px di spinta si rilascia e si annulla.
#[derive(Debug)]
struct Hold {
    edge: &'static str,
    edge_n: i32,
    set: (i32, i32),
    intent: i32,
}

impl Hold {
    fn new(edge: &'static str, px: i32, py: i32) -> Self {
        let edge_n = if matches!(edge, "left" | "right") { px } else { py };
        Self { edge, edge_n, set: (px, py), intent: 0 }
    }

    /// Nuova posizione del cursore, o `None` se l'utente ha tirato oltre `limit`.
    fn pull(&mut self, px: i32, py: i32, limit: i32) -> Option<(i32, i32)> {
        let vertical = matches!(self.edge, "left" | "right");
        let dir = if matches!(self.edge, "left" | "top") { 1 } else { -1 };
        let (n, set_n) = if vertical { (px, self.set.0) } else { (py, self.set.1) };
        self.intent = (self.intent + (n - set_n) * dir).max(0);
        if self.intent > limit {
            return None;
        }
        let t = self.edge_n + dir * self.intent / 4;
        self.set = if vertical { (t, py) } else { (px, t) };
        Some(self.set)
    }
}

/// Manda il bagliore alla GUI solo se cambiato in modo visibile.
fn emit(sh: &Shared, last: &mut Option<Glow>, g: Option<Glow>) {
    let same = match (*last, g) {
        (Some(a), Some(b)) => a.edge == b.edge && a.x == b.x && a.y == b.y && (a.level - b.level).abs() < 0.01,
        (a, b) => a.is_none() && b.is_none(),
    };
    if !same && sh.glow.get().map_or(true, |sink| sink(g)) {
        *last = g;
    }
}

#[cfg(windows)]
fn set_cursor(x: i32, y: i32) {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y) };
}

#[cfg(target_os = "macos")]
fn set_cursor(x: i32, y: i32) {
    let _ = core_graphics::display::CGDisplay::warp_mouse_cursor_position(core_graphics::geometry::CGPoint::new(x as f64, y as f64));
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_cursor(_x: i32, _y: i32) {} // shortcut: su Linux il cursore non viene riposizionato

/// Stato condiviso tra GUI e loop.
pub struct Shared {
    pub cfg: Mutex<Config>,
    pub running: AtomicBool,
    pub devices: Mutex<Vec<LogiDevice>>,
    pub log: Mutex<Vec<String>>,
    pub glow: OnceLock<GlowSink>,
    scanning: AtomicBool,
}

impl Shared {
    pub fn new(cfg: Config) -> Arc<Self> {
        Arc::new(Self {
            cfg: Mutex::new(cfg),
            running: AtomicBool::new(false),
            devices: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
            glow: OnceLock::new(),
            scanning: AtomicBool::new(false),
        })
    }

    pub fn log(&self, msg: impl Into<String>) {
        let msg = msg.into();
        println!("{msg}");
        let mut log = self.log.lock().unwrap();
        log.push(msg);
        if log.len() > 300 {
            log.drain(..100);
        }
    }

    /// Cerca le periferiche (bloccante). Applica il filtro per nome del config.
    pub fn rescan(&self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return; // già in corso
        }
        let filter: Vec<String> = self.cfg.lock().unwrap().devices.iter().map(|s| s.to_lowercase()).collect();
        match hidpp::discover() {
            Ok(devs) => {
                let devs: Vec<_> = devs
                    .into_iter()
                    .filter(|d| filter.is_empty() || filter.iter().any(|f| d.name.to_lowercase().contains(f)))
                    .collect();
                let key = |v: &[LogiDevice]| v.iter().map(|d| (d.path.clone(), d.device_index)).collect::<Vec<_>>();
                let changed = key(&self.devices.lock().unwrap()) != key(&devs);
                *self.devices.lock().unwrap() = devs.clone();
                if changed {
                    self.log(if devs.is_empty() { "[i] nessuna periferica collegata" } else { "[i] periferiche collegate:" });
                    for d in &devs {
                        self.log(format!("      - {d}"));
                    }
                }
            }
            Err(e) => self.log(format!("[!] errore nella ricerca dispositivi: {e}")),
        }
        self.scanning.store(false, Ordering::SeqCst);
    }

    pub fn rescan_async(self: &Arc<Self>) {
        let me = self.clone();
        thread::spawn(move || me.rescan());
    }

    pub fn switch_all(&self, rule: &Rule) {
        let mut devs = self.devices.lock().unwrap().clone();
        if devs.is_empty() {
            self.rescan();
            devs = self.devices.lock().unwrap().clone();
        }
        let mut failed = Vec::new();
        for d in &devs {
            let ch = rule.channel_for(&d.name);
            match hidpp::switch_device(d, ch) {
                Ok(()) => self.log(format!("[>] {} -> canale {ch}", d.name)),
                Err(e) => failed.push((d.name.clone(), ch, e)),
            }
        }
        if !failed.is_empty() {
            // L'handle potrebbe essere scaduto (riconnessione): riprova dopo un nuovo scan
            self.rescan();
            let fresh = self.devices.lock().unwrap().clone();
            for (name, ch, e) in failed {
                let r = fresh.iter().find(|d| d.name == name).ok_or("non più trovata".to_string()).and_then(|d| hidpp::switch_device(d, ch));
                match r {
                    Ok(()) => self.log(format!("[>] {name} -> canale {ch} (secondo tentativo)")),
                    Err(e2) => self.log(format!("[!] {name}: cambio canale fallito ({e} / {e2})")),
                }
            }
        }
        self.devices.lock().unwrap().clear(); // dopo lo switch non sono più qui
    }
}

/// Avvia il thread di osservazione del cursore (attivo solo con `running`).
pub fn spawn(sh: Arc<Shared>) {
    thread::spawn(move || {
        let dq = DeviceState::new();
        let mut monitors = get_monitors();
        let mut last_mon = Instant::now();
        let mut last_scan: Option<Instant> = None;
        let mut edge_since: Option<(&str, Instant)> = None;
        let mut cooldown_until = Instant::now();
        let mut armed = true;
        let mut last_own: Option<(i32, i32)> = None;
        let mut mon_armed = true;
        let mut warned_all_shared = false;
        let mut hold: Option<Hold> = None;
        let mut last_glow: Option<Glow> = None;

        loop {
            if !sh.running.load(Ordering::Relaxed) {
                last_scan = None;
                hold = None;
                emit(&sh, &mut last_glow, None);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            let cfg = sh.cfg.lock().unwrap().clone();
            thread::sleep(Duration::from_secs_f64(1.0 / cfg.poll_hz.max(1) as f64));
            let now = Instant::now();

            if now - last_mon > Duration::from_secs(5) {
                monitors = get_monitors();
                last_mon = now;
            }
            if last_scan.map_or(true, |t| now - t > Duration::from_secs(15)) {
                sh.rescan_async();
                last_scan = Some(now);
            }

            let mouse = dq.get_mouse();
            let (mut px, mut py) = mouse.coords;
            let pressed = mouse.button_pressed.iter().any(|&b| b);
            let dragging = cfg.ignore_while_dragging && pressed;

            // Monitor condiviso con l'altro PC: entrarci = passare all'altro PC.
            // Se tutti i monitor sono marcati condivisi la regola è un errore di
            // configurazione (il cursore sarebbe sempre "dentro"): la si ignora.
            let own: Vec<Rect> = monitors.iter().copied().filter(|m| cfg.monitor_rule(&m.key()).is_none()).collect();
            if own.is_empty() && !monitors.is_empty() {
                if !warned_all_shared {
                    sh.log("[!] tutti i monitor sono marcati come condivisi: regole dei monitor ignorate. Clicca il monitor per riportarlo a \"Questo PC\".");
                    warned_all_shared = true;
                }
            } else {
                warned_all_shared = false;
            }
            let cur = monitors.iter().find(|m| m.contains(px, py));
            let shared_rule = if own.is_empty() { None } else { cur.and_then(|m| cfg.monitor_rule(&m.key())) };
            if let Some(rule) = shared_rule {
                // mon_armed: dopo un cambio il cursore deve tornare su un monitor di
                // questo PC prima di poter scattare di nuovo (niente cambi a ripetizione).
                if mon_armed && now >= cooldown_until && !dragging {
                    mon_armed = false;
                    if let Some((x, y)) = last_own {
                        set_cursor(x, y); // al rientro il cursore non deve essere lì dentro
                    }
                    armed = false;
                    edge_since = None;
                    cooldown_until = now + Duration::from_secs_f64(cfg.cooldown_s);
                    sh.log(format!("[*] monitor {}: passo al canale {}", cur.unwrap().key(), rule.channel));
                    sh.switch_all(rule);
                }
                hold = None;
                emit(&sh, &mut last_glow, None);
                continue;
            }
            last_own = Some((px, py));
            mon_armed = true;

            let glow = 'g: {
                let mut hit = outer_edge_hit(&own, px, py, cfg.corner_margin_px);
                if let Some(mut h) = hold.take() {
                    if !dragging && hit.map_or(true, |e| e == h.edge) {
                        if let Some(p) = h.pull(px, py, cfg.resist_px) {
                            if p != (px, py) {
                                set_cursor(p.0, p.1);
                            }
                            (px, py) = p;
                            hit = Some(h.edge);
                            hold = Some(h);
                        }
                    }
                    if hold.is_none() {
                        edge_since = None; // strappo deciso: attesa annullata
                    }
                }

                let Some(edge) = hit else {
                    edge_since = None;
                    armed = true;
                    // Avvicinamento: bagliore tenue che cresce man mano che il cursore si accosta.
                    if !dragging && now >= cooldown_until && cfg.glow_distance_px > 0 {
                        if let Some((e, d, (x, y))) = near_edge(&own, px, py, cfg.corner_margin_px, cfg.glow_distance_px) {
                            if cfg.edge_rule(e).is_some() {
                                let level = 0.35 * (1.0 - d as f32 / (cfg.glow_distance_px + 1) as f32);
                                break 'g Some(Glow { x, y, edge: e, level });
                            }
                        }
                    }
                    break 'g None;
                };
                let Some(rule) = cfg.edge_rule(edge) else { break 'g None };
                if !armed || now < cooldown_until {
                    break 'g None;
                }
                if dragging {
                    edge_since = None;
                    break 'g None;
                }
                let since = match edge_since {
                    Some((e, since)) if e == edge => since,
                    _ => {
                        edge_since = Some((edge, now));
                        if cfg.resist_px > 0 {
                            hold = Some(Hold::new(edge, px, py));
                        }
                        now
                    }
                };
                let progress = (now - since).as_secs_f32() * 1000.0 / cfg.dwell_ms.max(1) as f32;
                if since == now || progress < 1.0 {
                    break 'g Some(Glow { x: px, y: py, edge, level: 0.35 + 0.65 * progress.min(1.0) });
                }

                emit(&sh, &mut last_glow, Some(Glow { x: px, y: py, edge, level: 1.0 }));
                armed = false;
                edge_since = None;
                hold = None;
                cooldown_until = now + Duration::from_secs_f64(cfg.cooldown_s);
                sh.log(format!("[*] bordo {edge}: passo al canale {}", rule.channel));
                sh.switch_all(rule);
                None
            };
            emit(&sh, &mut last_glow, glow);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_with_three_monitors_and_shared_center() {
        let l = Rect { x: -3840, y: 0, w: 3840, h: 2160 };
        let r = Rect { x: 3840, y: 0, w: 3840, h: 2160 };
        let c = Rect { x: 0, y: 0, w: 3840, h: 2160 };
        let all = [l, c, r];
        assert_eq!(outer_edge_hit(&all, -1, 1000, 40), None); // bordo interno
        assert_eq!(outer_edge_hit(&all, -3840, 1000, 40), Some("left"));
        assert_eq!(outer_edge_hit(&all, -3840, 10, 40), None); // angolo
        assert_eq!(outer_edge_hit(&[l, r], -1, 1000, 40), Some("right")); // centro escluso
        assert_eq!(outer_edge_hit(&all, 100, 2159, 40), Some("bottom"));
    }

    #[test]
    fn near_edge_only_outer_and_within_distance() {
        let l = Rect { x: -1920, y: 0, w: 1920, h: 1080 };
        let c = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let all = [l, c];
        assert_eq!(near_edge(&all, -1900, 500, 40, 50), Some(("left", 20, (-1920, 500))));
        assert_eq!(near_edge(&all, -1800, 500, 40, 50), None); // troppo lontano
        assert_eq!(near_edge(&all, 10, 500, 40, 50), None); // bordo interno
        assert_eq!(near_edge(&all, 1900, 500, 40, 50), Some(("right", 19, (1919, 500))));
    }

    #[test]
    fn hold_brakes_then_releases() {
        let mut h = Hold::new("left", 0, 500);
        assert_eq!(h.pull(8, 500, 30), Some((2, 500))); // 8 px di spinta -> 2 px reali
        assert_eq!(h.pull(2, 520, 30), Some((2, 520))); // lungo il bordo: libero
        assert_eq!(h.pull(0, 520, 30), Some((1, 520))); // ritorno verso il bordo riduce la spinta
        assert_eq!(h.pull(30, 520, 30), None); // strappo oltre il limite
        let mut b = Hold::new("bottom", 700, 1079);
        assert_eq!(b.pull(700, 1071, 30), Some((700, 1077)));
    }
}
