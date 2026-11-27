use crate::config::{Config, Rule};
use crate::engine::{get_monitors, Rect, Shared};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke, Vec2};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const EDGE_LABELS: [(&str, &str); 4] = [("left", "Sinistra"), ("right", "Destra"), ("top", "Sopra"), ("bottom", "Sotto")];

pub struct App {
    sh: Arc<Shared>,
    cfg_path: PathBuf,
    saved: Config,
    monitors: Vec<Rect>,
    status: String,
}

impl App {
    pub fn new(sh: Arc<Shared>, cfg_path: PathBuf) -> Self {
        let saved = sh.cfg.lock().unwrap().clone();
        Self { sh, cfg_path, saved, monitors: get_monitors(), status: String::new() }
    }

    fn layout_ui(&mut self, ui: &mut egui::Ui, cfg: &mut Config) {
        ui.label("Clic su un monitor per indicare se appartiene a questo PC o è condiviso con un altro PC (ingresso diverso). \
                  Entrando in un monitor condiviso, tastiera e mouse passano al canale indicato.");
        if self.monitors.is_empty() {
            ui.colored_label(Color32::RED, "Nessun monitor rilevato");
            return;
        }
        let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 240.0), Sense::click());
        let area = resp.rect.shrink(36.0);
        let (min_x, min_y) = (self.monitors.iter().map(|m| m.x).min().unwrap(), self.monitors.iter().map(|m| m.y).min().unwrap());
        let max_x = self.monitors.iter().map(|m| m.x + m.w).max().unwrap();
        let max_y = self.monitors.iter().map(|m| m.y + m.h).max().unwrap();
        let scale = (area.width() / (max_x - min_x) as f32).min(area.height() / (max_y - min_y) as f32);
        let size = Vec2::new((max_x - min_x) as f32, (max_y - min_y) as f32) * scale;
        let origin = area.center() - size / 2.0;
        let to_screen = |m: &Rect| {
            egui::Rect::from_min_size(
                origin + Vec2::new((m.x - min_x) as f32, (m.y - min_y) as f32) * scale,
                Vec2::new(m.w as f32, m.h as f32) * scale,
            )
            .shrink(2.0)
        };

        let visuals = ui.visuals().clone();
        for m in &self.monitors {
            let r = to_screen(m);
            let shared = cfg.monitor_rule(&m.key()).map(|r| r.channel);
            let fill = if shared.is_some() { Color32::from_rgb(214, 137, 16) } else { visuals.widgets.inactive.bg_fill };
            painter.rect_filled(r, 4.0, fill);
            painter.rect_stroke(r, 4.0, Stroke::new(1.0_f32,visuals.widgets.inactive.fg_stroke.color), egui::StrokeKind::Inside);
            let role = shared.map_or("Questo PC".to_string(), |c| format!("Altro PC\ncanale {c}"));
            painter.text(r.center(), Align2::CENTER_CENTER, format!("{role}\n{}  {}x{}", m.key(), m.w, m.h), FontId::proportional(13.0), visuals.text_color());
        }

        // Frecce sui bordi esterni con il canale associato
        let bounds = egui::Rect::from_min_size(origin, size);
        for (edge, _) in EDGE_LABELS {
            if let Some(r) = cfg.edge_rule(edge) {
                let (pos, txt, align) = match edge {
                    "left" => (bounds.left_center() - Vec2::X * 4.0, format!("< {}", r.channel), Align2::RIGHT_CENTER),
                    "right" => (bounds.right_center() + Vec2::X * 4.0, format!("{} >", r.channel), Align2::LEFT_CENTER),
                    "top" => (bounds.center_top() - Vec2::Y * 4.0, format!("^ {}", r.channel), Align2::CENTER_BOTTOM),
                    _ => (bounds.center_bottom() + Vec2::Y * 4.0, format!("v {}", r.channel), Align2::CENTER_TOP),
                };
                painter.text(pos, align, txt, FontId::proportional(16.0), Color32::from_rgb(214, 137, 16));
            }
        }

        if resp.clicked() {
            let p: Pos2 = resp.interact_pointer_pos().unwrap_or_default();
            if let Some(m) = self.monitors.iter().find(|m| to_screen(m).contains(p)) {
                let key = m.key();
                // ciclo: questo PC -> canale 1 -> 2 -> 3 -> questo PC
                let next = match cfg.monitor_rule(&key).map(|r| r.channel) {
                    None => Some(1),
                    Some(3) => None,
                    Some(c) => Some(c + 1),
                };
                let own_left = self.monitors.iter().filter(|o| o.key() != key && cfg.monitor_rule(&o.key()).is_none()).count();
                if next.is_some() && cfg.monitor_rule(&key).is_none() && own_left == 0 {
                    // altrimenti il cursore sarebbe sempre in un monitor condiviso: cambi di canale a ripetizione
                    self.status = "Almeno un monitor deve restare a questo PC: usa i bordi per passare all'altro PC.".into();
                } else {
                    cfg.set_rule("", Some(&key), next);
                }
            }
        }

        ui.horizontal(|ui| {
            if ui.button("Rileva di nuovo i monitor").clicked() {
                self.monitors = get_monitors();
            }
        });

        ui.add_space(6.0);
        ui.label("Bordi esterni (dove non c'è un altro monitor di questo PC):");
        egui::Grid::new("edges").num_columns(2).show(ui, |ui| {
            for (edge, label) in EDGE_LABELS {
                ui.label(label);
                let cur = cfg.edge_rule(edge).map(|r| r.channel);
                let mut sel = cur;
                egui::ComboBox::from_id_salt(edge)
                    .selected_text(sel.map_or("Nessuna azione".into(), |c| format!("Canale {c}")))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sel, None, "Nessuna azione");
                        for c in 1..=3 {
                            ui.selectable_value(&mut sel, Some(c), format!("Canale {c}"));
                        }
                    });
                if sel != cur {
                    cfg.set_rule(edge, None, sel);
                }
                ui.end_row();
            }
        });
    }

    fn devices_ui(&mut self, ui: &mut egui::Ui) {
        let devs = self.sh.devices.lock().unwrap().clone();
        if devs.is_empty() {
            ui.label("Nessuna periferica trovata (la ricerca richiede qualche secondo).");
        }
        for d in &devs {
            ui.label(format!("- {d}"));
        }
        ui.horizontal(|ui| {
            if ui.button("Cerca periferiche").clicked() {
                self.sh.rescan_async();
            }
            ui.separator();
            ui.label("Prova:");
            for c in 1..=3u8 {
                if ui.button(format!("canale {c}")).on_hover_text("Sposta subito tastiera e mouse su questo canale").clicked() {
                    let sh = self.sh.clone();
                    std::thread::spawn(move || sh.switch_all(&Rule { edge: String::new(), monitor: None, channel: c, device_channels: Default::default() }));
                }
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(300));
        let mut cfg = self.sh.cfg.lock().unwrap().clone();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let running = self.sh.running.load(Ordering::Relaxed);
                let (txt, col) = if running { ("ATTIVO", Color32::from_rgb(40, 160, 70)) } else { ("FERMO", Color32::GRAY) };
                ui.colored_label(col, txt);
                if ui.button(if running { "Ferma" } else { "Avvia" }).clicked() {
                    self.sh.running.store(!running, Ordering::Relaxed);
                    self.sh.log(if running { "[i] fermato" } else { "[i] avviato" });
                }
                ui.separator();
                let dirty = cfg != self.saved;
                if ui.add_enabled(dirty, egui::Button::new("Salva configurazione")).clicked() {
                    self.status = match cfg.save(&self.cfg_path) {
                        Ok(()) => {
                            self.saved = cfg.clone();
                            format!("Salvato in {}", self.cfg_path.display())
                        }
                        Err(e) => format!("Errore: {e}"),
                    };
                }
                if dirty {
                    ui.label("modifiche non salvate (già attive)");
                } else {
                    ui.label(&self.status);
                }
            });
        });

        egui::TopBottomPanel::bottom("log").resizable(true).default_height(140.0).show(ctx, |ui| {
            ui.label("Registro");
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                for line in self.sh.log.lock().unwrap().iter() {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Disposizione schermi");
                self.layout_ui(ui, &mut cfg);
                ui.separator();
                ui.heading("Periferiche");
                self.devices_ui(ui);
                ui.separator();
                ui.heading("Impostazioni");
                egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                    ui.label("Attesa sul bordo (ms)");
                    ui.add(egui::DragValue::new(&mut cfg.dwell_ms).range(0..=2000));
                    ui.end_row();
                    ui.label("Margine angoli (px)");
                    ui.add(egui::DragValue::new(&mut cfg.corner_margin_px).range(0..=500));
                    ui.end_row();
                    ui.label("Pausa tra due cambi (s)");
                    ui.add(egui::DragValue::new(&mut cfg.cooldown_s).range(0.0..=30.0).speed(0.1));
                    ui.end_row();
                    ui.label("Ignora mentre trascini");
                    ui.checkbox(&mut cfg.ignore_while_dragging, "");
                    ui.end_row();
                });
            });
        });

        *self.sh.cfg.lock().unwrap() = cfg; // le modifiche valgono subito, anche senza salvare
    }
}
