//! Loop che osserva il cursore e cambia canale quando tocca un bordo esterno
//! o entra in un monitor condiviso con l'altro PC.

use crate::config::{Config, Rule};
use crate::hidpp::{self, LogiDevice};
use device_query::{DeviceQuery, DeviceState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    scanning: AtomicBool,
}

impl Shared {
    pub fn new(cfg: Config) -> Arc<Self> {
        Arc::new(Self {
            cfg: Mutex::new(cfg),
            running: AtomicBool::new(false),
            devices: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
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

        loop {
            if !sh.running.load(Ordering::Relaxed) {
                last_scan = None;
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
            let (px, py) = mouse.coords;
            let pressed = mouse.button_pressed.iter().any(|&b| b);
            let dragging = cfg.ignore_while_dragging && pressed;

            // Monitor condiviso con l'altro PC: entrarci = passare all'altro PC
            let cur = monitors.iter().find(|m| m.contains(px, py));
            if let Some(rule) = cur.and_then(|m| cfg.monitor_rule(&m.key())) {
                if now >= cooldown_until && !dragging {
                    if let Some((x, y)) = last_own {
                        set_cursor(x, y); // al rientro il cursore non deve essere lì dentro
                    }
                    armed = false;
                    edge_since = None;
                    cooldown_until = now + Duration::from_secs_f64(cfg.cooldown_s);
                    sh.log(format!("[*] monitor {}: passo al canale {}", cur.unwrap().key(), rule.channel));
                    sh.switch_all(rule);
                }
                continue;
            }
            last_own = Some((px, py));

            let own: Vec<Rect> = monitors.iter().copied().filter(|m| cfg.monitor_rule(&m.key()).is_none()).collect();
            let Some(edge) = outer_edge_hit(&own, px, py, cfg.corner_margin_px) else {
                edge_since = None;
                armed = true;
                continue;
            };
            let Some(rule) = cfg.edge_rule(edge) else { continue };
            if !armed || now < cooldown_until {
                continue;
            }
            if dragging {
                edge_since = None;
                continue;
            }
            match edge_since {
                Some((e, since)) if e == edge => {
                    if now - since < Duration::from_millis(cfg.dwell_ms) {
                        continue;
                    }
                }
                _ => {
                    edge_since = Some((edge, now));
                    continue;
                }
            }

            armed = false;
            edge_since = None;
            cooldown_until = now + Duration::from_secs_f64(cfg.cooldown_s);
            sh.log(format!("[*] bordo {edge}: passo al canale {}", rule.channel));
            sh.switch_all(rule);
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
}
