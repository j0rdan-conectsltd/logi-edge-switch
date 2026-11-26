use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const EDGES: [&str; 4] = ["left", "right", "top", "bottom"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    /// left / right / top / bottom (vuoto se la regola usa `monitor`)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub edge: String,
    /// "x,y": scatta entrando in questo monitor (condiviso con l'altro PC)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    pub channel: u8,
    /// eccezioni per dispositivo: {"MX Keys": 3}
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_channels: BTreeMap<String, u8>,
}

impl Rule {
    pub fn channel_for(&self, device_name: &str) -> u8 {
        let n = device_name.to_lowercase();
        self.device_channels
            .iter()
            .find(|(k, _)| n.contains(&k.to_lowercase()))
            .map_or(self.channel, |(_, &c)| c)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub rules: Vec<Rule>,
    /// filtro per nome; vuoto = tutte
    pub devices: Vec<String>,
    pub dwell_ms: u64,
    pub corner_margin_px: i32,
    pub cooldown_s: f64,
    pub ignore_while_dragging: bool,
    pub poll_hz: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            devices: Vec::new(),
            dwell_ms: 150,
            corner_margin_px: 40,
            cooldown_s: 2.0,
            ignore_while_dragging: true,
            poll_hz: 100,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let cfg: Config = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        for r in &cfg.rules {
            if r.monitor.is_none() && !EDGES.contains(&r.edge.as_str()) {
                return Err(format!("bordo non valido: {:?}", r.edge));
            }
            if !(1..=3).contains(&r.channel) {
                return Err(format!("canale non valido: {}", r.channel));
            }
        }
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn edge_rule(&self, edge: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.monitor.is_none() && r.edge == edge)
    }

    pub fn monitor_rule(&self, key: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.monitor.as_deref().map(|m| m.replace(' ', "")) == Some(key.to_string()))
    }

    /// Imposta (o rimuove con None) la regola per un bordo o un monitor.
    pub fn set_rule(&mut self, edge: &str, monitor: Option<&str>, channel: Option<u8>) {
        let pos = self.rules.iter().position(|r| match monitor {
            Some(m) => r.monitor.as_deref() == Some(m),
            None => r.monitor.is_none() && r.edge == edge,
        });
        match (pos, channel) {
            (Some(i), Some(c)) => self.rules[i].channel = c,
            (Some(i), None) => {
                self.rules.remove(i);
            }
            (None, Some(c)) => self.rules.push(Rule {
                edge: if monitor.is_some() { String::new() } else { edge.into() },
                monitor: monitor.map(Into::into),
                channel: c,
                device_channels: BTreeMap::new(),
            }),
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_python_config_and_round_trips() {
        let json = r#"{"rules":[{"monitor":"0,0","channel":2},{"edge":"left","channel":1,"peer":"1.2.3.4:5"}],
                       "dwell_ms":200,"listen_port":50555}"#;
        let mut c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.monitor_rule("0,0").unwrap().channel, 2);
        assert_eq!(c.edge_rule("left").unwrap().channel, 1);
        assert_eq!(c.dwell_ms, 200);
        assert_eq!(c.cooldown_s, 2.0);
        c.set_rule("", Some("0,0"), None);
        c.set_rule("right", None, Some(3));
        assert!(c.monitor_rule("0,0").is_none());
        assert_eq!(c.edge_rule("right").unwrap().channel, 3);
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn example_configs_are_valid() {
        for f in std::fs::read_dir("esempi").unwrap().chain(std::fs::read_dir(".").unwrap()) {
            let p = f.unwrap().path();
            if p.extension().is_some_and(|e| e == "json") {
                Config::load(&p).unwrap_or_else(|e| panic!("{e}"));
            }
        }
        let mac = Config::load(Path::new("esempi/mac-centro.json")).unwrap();
        assert_eq!(mac.edge_rule("left").unwrap().channel, 1);
        assert_eq!(mac.edge_rule("right").unwrap().channel, 1);
    }
}
