//! Interfaccia in Rune: finestra trasparente con Acrylic di Windows 11 dietro e pannelli
//! "liquid glass" (riempimento traslucido, riflesso a gradiente, bordo chiaro, ombra morbida).
use crate::config::{Config, Rule};
use crate::engine::{get_monitors, Glow, Rect, Shared};
use rune::tok::motion;
use rune::{
    col, el, oklch, oklcha, row, text, App, AppSpec, Color, Cx, DisplayTarget, El, KeyPolicy, OverlayInput, OverlayLevel, OverlayOptions,
    OverlayPlacement, OverlaySpec, ScrollSpec, TextStyle, Tx, WindowBackdrop, WindowOptions,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const EDGE_LABELS: [(&str, &str); 4] = [("left", "Sinistra"), ("right", "Destra"), ("top", "Sopra"), ("bottom", "Sotto")];
const MAP: (f32, f32) = (600.0, 210.0);

pub struct Gui {
    sh: Arc<Shared>,
    cfg_path: PathBuf,
    saved: Config,
    monitors: Vec<Rect>,
    status: String,
    focused: bool,
    minimize: bool,
    seen: (usize, Option<String>, usize, bool),
}

pub fn run(sh: Arc<Shared>, cfg_path: PathBuf, start_minimized: bool) {
    let saved = sh.cfg.lock().unwrap().clone();
    let state = Gui { sh, cfg_path, saved, monitors: get_monitors(), status: String::new(), focused: true, minimize: start_minimized, seen: (0, None, 0, false) };
    let spec = AppSpec { min_size: Some((680.0, 600.0)), poll: Some(poll), ..AppSpec::new("Logi Edge Switch", (720.0, 760.0), || unreachable!(), view) };
    let sh = state.sh.clone();
    let mut app = App::new(spec)
        .init(move || state)
        .args(Vec::<String>::new()) // la riga di comando è nostra (--start, -c), non di Rune
        .window_options(WindowOptions { transparent: true, ..Default::default() })
        .window_backdrop(WindowBackdrop::Acrylic)
        .on_focus(|s, f, _| s.focused = f);

    // Bagliore: overlay trasparente, non cliccabile, a tutto schermo sul monitor del cursore.
    let opts = OverlayOptions {
        display: DisplayTarget::UnderPointer,
        follow_pointer: true,
        placement: Some(OverlayPlacement::FullScreen),
        input: OverlayInput::None,
        keys: KeyPolicy::None,
        level: OverlayLevel::AboveOverlays,
        prewarm: true,
        ..Default::default()
    };
    let overlay = app.overlay(OverlaySpec::new("edge-glow", (1.0, 1.0), glow_view).options(opts), EdgeGlow { edge: "left", at: 0.0, level: 0.0 });
    let shown = AtomicBool::new(false);
    let overlay = Mutex::new(overlay);
    let _ = sh.glow.set(Box::new(move |g: Option<Glow>| {
        let h = overlay.lock().unwrap();
        let Some(g) = g else {
            if shown.swap(false, Ordering::Relaxed) {
                h.fade_out(Duration::from_millis(220));
            }
            return true;
        };
        if !shown.swap(true, Ordering::Relaxed) {
            h.update(|s: &mut EdgeGlow, _| s.level = 0.0); // niente residui del bagliore precedente
            h.show();
        }
        // Finché lo show non ha effetto non c'è conversione: il loop riprova al giro dopo.
        let Some((lx, ly)) = h.screen_to_local((g.x, g.y)) else { return false };
        let at = if matches!(g.edge, "left" | "right") { ly } else { lx };
        h.update(move |s: &mut EdgeGlow, _| *s = EdgeGlow { edge: g.edge, at, level: g.level });
        true
    }));
    app.run();
}

#[derive(Clone, Copy)]
struct EdgeGlow {
    edge: &'static str,
    /// posizione lungo il bordo, in punti dell'overlay
    at: f32,
    level: f32,
}

/// Lama di luce azzurra sul bordo + alone largo; lunghezza e intensità crescono con `level`.
fn glow_view(g: &EdgeGlow, _: &mut Cx<EdgeGlow>) -> El {
    let cyan = oklch(0.86, 0.11, 215.0);
    let l = g.level.clamp(0.0, 1.0);
    let bar = |len: f32, thick: f32, alpha: f32, blur: f32| {
        let start = g.at - len / 2.0;
        let (w, h, pin) = match g.edge {
            "left" => (thick, len, (Some(0.0), Some(start), None, None)),
            "right" => (thick, len, (None, Some(start), Some(0.0), None)),
            "top" => (len, thick, (Some(start), Some(0.0), None, None)),
            _ => (len, thick, (Some(start), None, None, Some(0.0))),
        };
        el().size(w, h).radius(thick / 2.0).pin(pin.0, pin.1, pin.2, pin.3)
            .fill(cyan.alpha(alpha * l)).shadow(0.0, blur, cyan.alpha(alpha * l))
    };
    el().key("glow").w_full().h_full()
        .child(bar(260.0 + 260.0 * l, 2.0, 0.45, 140.0 * l)) // alone
        .child(bar(140.0 + 180.0 * l, 5.0, 0.95, 36.0 + 30.0 * l)) // lama
}

/// Il motore scrive log/periferiche da altri thread: ricontrolla ogni 300 ms.
fn poll(s: &mut Gui, _: f64, tx: &mut Tx) -> bool {
    if std::mem::take(&mut s.minimize) {
        tx.minimize();
    }
    tx.wake_after(Duration::from_millis(300));
    let log = s.sh.log.lock().unwrap();
    let now = (log.len(), log.last().cloned(), s.sh.devices.lock().unwrap().len(), s.sh.running.load(Ordering::Relaxed));
    drop(log);
    std::mem::replace(&mut s.seen, now.clone()) != now
}

struct Pal {
    fg: Color,
    dim: Color,
    sheen: (Color, Color),
    rim: Color,
    chip: Color,
    shadow: Color,
    well: Color,
    accent: Color,
    ok: Color,
}

fn pal(dark: bool) -> Pal {
    let w = Color::WHITE;
    if dark {
        Pal {
            fg: w,
            dim: w.alpha(0.62),
            sheen: (w.alpha(0.16), w.alpha(0.05)),
            rim: w.alpha(0.22),
            chip: w.alpha(0.10),
            shadow: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.35 },
            well: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.22 },
            accent: oklch(0.78, 0.15, 70.0),
            ok: oklch(0.78, 0.17, 150.0),
        }
    } else {
        let ink = oklch(0.22, 0.01, 260.0);
        Pal {
            fg: ink,
            dim: ink.alpha(0.6),
            sheen: (w.alpha(0.72), w.alpha(0.38)),
            rim: w.alpha(0.85),
            chip: w.alpha(0.55),
            shadow: Color { r: 0.1, g: 0.12, b: 0.2, a: 0.18 },
            well: w.alpha(0.4),
            accent: oklch(0.68, 0.16, 60.0),
            ok: oklch(0.62, 0.17, 150.0),
        }
    }
}

/// Pannello di vetro: riflesso in alto più chiaro, bordo luminoso, ombra diffusa.
fn glass(p: &Pal, title: &str) -> El {
    col().w_full().radius(22.0).padding(18.0).gap(12.0)
        .gradient(p.sheen.0, p.sheen.1)
        .border(1.0, p.rim)
        .shadow(10.0, 30.0, p.shadow)
        .child(text(title).font(TextStyle::new(15.0, 650.0)).color(p.fg))
}

/// Pulsante a pillola; `on` lo evidenzia col colore d'accento.
fn pill(p: &Pal, key: impl Into<String>, label: impl Into<String>, on: bool) -> El {
    let label = label.into();
    row().key(key).role("button").label(label.clone()).focusable(true).hit()
        .h(30.0).px(14.0).center().radius(15.0)
        .fill(if on { p.accent.alpha(0.85) } else { p.chip })
        .border(1.0, if on { p.accent } else { p.rim })
        .motion(motion::SNAPPY)
        .child(text(label).font(TextStyle::new(12.5, 600.0)).color(if on { Color::WHITE } else { p.fg }))
}

fn body(p: &Pal, s: impl Into<String>) -> El {
    text(s).w_full().wrap().font(TextStyle::new(12.5, 450.0)).color(p.dim)
}

fn view(s: &Gui, cx: &mut Cx<Gui>) -> El {
    let dark = cx.system_dark();
    let p = pal(dark);
    let cfg = s.sh.cfg.lock().unwrap().clone();
    // Acrylic si vede solo con la finestra in primo piano: altrimenti fondo pieno leggibile.
    let live = cx.window_backdrop().is_active() && s.focused;
    let (bg_top, bg_bot) = match (live, dark) {
        (true, true) => (oklcha(0.30, 0.06, 265.0, 0.35), oklcha(0.22, 0.07, 310.0, 0.45)),
        (true, false) => (oklcha(0.95, 0.03, 240.0, 0.35), oklcha(0.90, 0.05, 320.0, 0.45)),
        (false, true) => (oklch(0.22, 0.03, 265.0), oklch(0.18, 0.04, 310.0)),
        (false, false) => (oklch(0.95, 0.02, 240.0), oklch(0.91, 0.03, 320.0)),
    };

    let content = col().key("content").w_full().grow(1.0).scroll(ScrollSpec::default()).gap(14.0).pb(4.0)
        .child(layout_card(s, &cfg, &p))
        .child(devices_card(s, &p))
        .child(settings_card(&cfg, &p));

    col().key("root").w_full().h_full().padding(16.0).gap(14.0).gradient(bg_top, bg_bot)
        .child(header(s, &cfg, &p))
        .child(content)
        .child(log_card(s, &p))
}

fn header(s: &Gui, cfg: &Config, p: &Pal) -> El {
    let running = s.sh.running.load(Ordering::Relaxed);
    let dirty = *cfg != s.saved;
    let dot = el().size(10.0, 10.0).radius(5.0).fill(if running { p.ok } else { p.dim }).motion(motion::SNAPPY);
    let toggle = pill(p, "run", if running { "Ferma" } else { "Avvia" }, !running).on_press(move |s: &mut Gui| {
        s.sh.running.store(!running, Ordering::Relaxed);
        s.sh.log(if running { "[i] fermato" } else { "[i] avviato" });
    });
    let save = if dirty {
        pill(p, "save", "Salva configurazione", true).on_press(|s: &mut Gui| {
            let cfg = s.sh.cfg.lock().unwrap().clone();
            s.status = match cfg.save(&s.cfg_path) {
                Ok(()) => {
                    s.saved = cfg;
                    format!("Salvato in {}", s.cfg_path.display())
                }
                Err(e) => format!("Errore: {e}"),
            };
        })
    } else {
        pill(p, "save", "Salva configurazione", false).disabled(true).opacity(0.45)
    };
    let note = if dirty { "modifiche non salvate (già attive)".to_string() } else { s.status.clone() };
    row().w_full().radius(26.0).px(16.0).py(10.0).gap(10.0).center_y()
        .gradient(p.sheen.0, p.sheen.1).border(1.0, p.rim).shadow(8.0, 24.0, p.shadow)
        .child(dot)
        .child(text(if running { "ATTIVO" } else { "FERMO" }).font(TextStyle::new(13.0, 700.0).tracking(0.6)).color(p.fg))
        .child(toggle)
        .child(save)
        .child(text(note).grow(1.0).truncate().font(TextStyle::new(12.0, 450.0)).color(p.dim))
}

fn layout_card(s: &Gui, cfg: &Config, p: &Pal) -> El {
    let mut card = glass(p, "Disposizione schermi").child(body(p,
        "Clic su un monitor per indicare se appartiene a questo PC o è condiviso con un altro PC (ingresso diverso). \
         Entrando in un monitor condiviso, tastiera e mouse passano al canale indicato."));
    if s.monitors.is_empty() {
        return card.child(text("Nessun monitor rilevato").font(TextStyle::new(13.0, 600.0)).color(oklch(0.65, 0.2, 25.0)));
    }

    let (min_x, min_y) = (s.monitors.iter().map(|m| m.x).min().unwrap(), s.monitors.iter().map(|m| m.y).min().unwrap());
    let max_x = s.monitors.iter().map(|m| m.x + m.w).max().unwrap();
    let max_y = s.monitors.iter().map(|m| m.y + m.h).max().unwrap();
    let pad = 34.0;
    let scale = ((MAP.0 - 2.0 * pad) / (max_x - min_x) as f32).min((MAP.1 - 2.0 * pad) / (max_y - min_y) as f32);
    let (bw, bh) = ((max_x - min_x) as f32 * scale, (max_y - min_y) as f32 * scale);
    let (ox, oy) = ((MAP.0 - bw) / 2.0, (MAP.1 - bh) / 2.0);

    let screens = s.monitors.iter().map(|m| {
        let key = m.key();
        let shared = cfg.monitor_rule(&key).map(|r| r.channel);
        let role = shared.map_or("Questo PC".to_string(), |c| format!("Altro PC · canale {c}"));
        let k = key.clone();
        col().key(format!("mon-{key}")).role("button").label(format!("{key}: {role}")).focusable(true).hit()
            .absolute(ox + (m.x - min_x) as f32 * scale + 3.0, oy + (m.y - min_y) as f32 * scale + 3.0)
            .size(m.w as f32 * scale - 6.0, m.h as f32 * scale - 6.0)
            .radius(12.0).center().gap(2.0)
            .fill(if shared.is_some() { p.accent.alpha(0.55) } else { p.chip })
            .border(1.0, if shared.is_some() { p.accent } else { p.rim })
            .motion(motion::SNAPPY)
            .on_press(move |s: &mut Gui| s.cycle_monitor(&k))
            .child(text(role).font(TextStyle::new(12.0, 650.0)).color(p.fg))
            .child(text(format!("{key}  {}x{}", m.w, m.h)).font(TextStyle::new(10.5, 450.0)).color(p.dim))
    });
    // Frecce sui bordi esterni con il canale associato
    let arrows = EDGE_LABELS.iter().filter_map(|(edge, _)| {
        let c = cfg.edge_rule(edge)?.channel;
        let (x, y, t) = match *edge {
            "left" => (ox - 30.0, oy + bh / 2.0 - 9.0, format!("◀ {c}")),
            "right" => (ox + bw + 6.0, oy + bh / 2.0 - 9.0, format!("{c} ▶")),
            "top" => (ox + bw / 2.0 - 12.0, oy - 24.0, format!("▲ {c}")),
            _ => (ox + bw / 2.0 - 12.0, oy + bh + 4.0, format!("▼ {c}")),
        };
        Some(text(t).absolute(x, y).font(TextStyle::new(14.0, 700.0)).color(p.accent))
    });
    let map = el().key("map").size(MAP.0, MAP.1).radius(16.0).fill(p.well).border(1.0, p.rim.alpha(0.5)).children(screens).children(arrows);
    card = card.child(row().w_full().center().child(map));
    if s.status.starts_with("Almeno") {
        card = card.child(text(s.status.clone()).w_full().wrap().font(TextStyle::new(12.0, 600.0)).color(p.accent));
    }
    card = card.child(row().child(pill(p, "rescan-mon", "Rileva di nuovo i monitor", false).on_press(|s: &mut Gui| s.monitors = get_monitors())));

    card = card.child(body(p, "Bordi esterni (dove non c'è un altro monitor di questo PC):"));
    for (edge, label) in EDGE_LABELS {
        let cur = cfg.edge_rule(edge).map(|r| r.channel);
        let opts = [None, Some(1u8), Some(2), Some(3)].map(|c| {
            let txt = c.map_or("Nessuna".to_string(), |c| format!("Canale {c}"));
            // "Nessuna" scelta: bordo marcato, l'arancio resta solo per i canali attivi
            let pl = pill(p, format!("edge-{edge}-{c:?}"), txt, c.is_some() && c == cur);
            let pl = if c.is_none() && cur.is_none() { pl.border(1.5, p.fg.alpha(0.55)) } else { pl };
            pl.on_press(move |s: &mut Gui| s.sh.cfg.lock().unwrap().set_rule(edge, None, c))
        });
        card = card.child(row().w_full().gap(6.0).center_y()
            .child(text(label).w(80.0).font(TextStyle::new(12.5, 550.0)).color(p.fg))
            .children(opts));
    }
    card
}

fn devices_card(s: &Gui, p: &Pal) -> El {
    let devs = s.sh.devices.lock().unwrap().clone();
    let mut card = glass(p, "Periferiche");
    if devs.is_empty() {
        card = card.child(body(p, "Nessuna periferica trovata (la ricerca richiede qualche secondo)."));
    }
    card = card.children(devs.iter().map(|d| {
        row().w_full().gap(8.0).center_y()
            .child(el().size(6.0, 6.0).radius(3.0).fill(p.ok))
            .child(text(d.to_string()).grow(1.0).truncate().font(TextStyle::new(12.5, 500.0)).color(p.fg))
    }));
    let tests = (1..=3u8).map(|c| {
        pill(p, format!("test-{c}"), format!("Canale {c}"), false).on_press(move |s: &mut Gui| {
            let sh = s.sh.clone();
            std::thread::spawn(move || sh.switch_all(&Rule { edge: String::new(), monitor: None, channel: c, device_channels: Default::default() }));
        })
    });
    card.child(row().w_full().gap(8.0).center_y()
        .child(pill(p, "rescan-dev", "Cerca periferiche", false).on_press(|s: &mut Gui| s.sh.rescan_async()))
        .child(el().grow(1.0))
        .child(text("Prova:").font(TextStyle::new(12.0, 500.0)).color(p.dim))
        .children(tests))
}

fn settings_card(cfg: &Config, p: &Pal) -> El {
    let stepper = |key: &'static str, label: &str, value: String, f: fn(&mut Config, f64), step: f64| {
        row().w_full().gap(8.0).center_y()
            .child(text(label.to_string()).grow(1.0).font(TextStyle::new(12.5, 500.0)).color(p.fg))
            .child(pill(p, format!("{key}-"), "−", false).w(34.0).on_press(move |s: &mut Gui| f(&mut s.sh.cfg.lock().unwrap(), -step)))
            .child(text(value).w(64.0).center().font(TextStyle::new(13.0, 600.0).features("'tnum' 1")).color(p.fg))
            .child(pill(p, format!("{key}+"), "+", false).w(34.0).on_press(move |s: &mut Gui| f(&mut s.sh.cfg.lock().unwrap(), step)))
    };
    let on = cfg.ignore_while_dragging;
    let knob = el().key("knob").size(18.0, 18.0).radius(9.0).fill(Color::WHITE).absolute(if on { 18.0 } else { 2.0 }, 2.0).shadow(1.0, 3.0, p.shadow);
    let switch = el().key("drag-switch").role("switch").label("Ignora mentre trascini").checked(on).focusable(true).hit()
        .size(38.0, 22.0).radius(11.0)
        .fill(if on { p.ok } else { p.chip }).border(1.0, p.rim)
        .motion(motion::SNAPPY)
        .on_press(|s: &mut Gui| {
            let mut c = s.sh.cfg.lock().unwrap();
            c.ignore_while_dragging = !c.ignore_while_dragging;
        })
        .child(knob);
    glass(p, "Impostazioni")
        .child(stepper("dwell", "Attesa sul bordo (ms)", cfg.dwell_ms.to_string(), |c, d| c.dwell_ms = (c.dwell_ms as f64 + d).clamp(0.0, 2000.0) as u64, 50.0))
        .child(stepper("margin", "Margine angoli (px)", cfg.corner_margin_px.to_string(), |c, d| c.corner_margin_px = (c.corner_margin_px as f64 + d).clamp(0.0, 500.0) as i32, 5.0))
        .child(stepper("cool", "Pausa tra due cambi (s)", format!("{:.1}", cfg.cooldown_s), |c, d| c.cooldown_s = ((c.cooldown_s + d) * 10.0).round().clamp(0.0, 300.0) / 10.0, 0.1))
        .child(stepper("glow", "Bagliore da (px dal bordo, 0 = solo in attesa)", cfg.glow_distance_px.to_string(), |c, d| c.glow_distance_px = (c.glow_distance_px as f64 + d).clamp(0.0, 400.0) as i32, 10.0))
        .child(stepper("resist", "Resistenza cursore (px, 0 = nessuna)", cfg.resist_px.to_string(), |c, d| c.resist_px = (c.resist_px as f64 + d).clamp(0.0, 200.0) as i32, 10.0))
        .child(row().w_full().gap(8.0).center_y()
            .child(text("Ignora mentre trascini").grow(1.0).font(TextStyle::new(12.5, 500.0)).color(p.fg))
            .child(switch))
}

fn log_card(s: &Gui, p: &Pal) -> El {
    let lines = s.sh.log.lock().unwrap().clone();
    col().key("log").w_full().h(150.0).radius(18.0).padding(12.0).gap(6.0)
        .fill(p.well).border(1.0, p.rim)
        .child(text("Registro").font(TextStyle::new(12.0, 650.0)).color(p.dim))
        .child(col().key("log-lines").w_full().grow(1.0).scroll(ScrollSpec::stick_bottom()).gap(1.0)
            .children(lines.into_iter().map(|l| text(l).w_full().truncate().font(TextStyle::new(11.5, 450.0).family(rune::Family::SystemMono)).color(p.fg))))
}

impl Gui {
    fn cycle_monitor(&mut self, key: &str) {
        let mut cfg = self.sh.cfg.lock().unwrap();
        // ciclo: questo PC -> canale 1 -> 2 -> 3 -> questo PC
        let cur = cfg.monitor_rule(key).map(|r| r.channel);
        let next = match cur {
            None => Some(1),
            Some(3) => None,
            Some(c) => Some(c + 1),
        };
        let own_left = self.monitors.iter().filter(|o| o.key() != key && cfg.monitor_rule(&o.key()).is_none()).count();
        if next.is_some() && cur.is_none() && own_left == 0 {
            // altrimenti il cursore sarebbe sempre in un monitor condiviso: cambi di canale a ripetizione
            self.status = "Almeno un monitor deve restare a questo PC: usa i bordi per passare all'altro PC.".into();
        } else {
            cfg.set_rule("", Some(key), next);
        }
    }
}
