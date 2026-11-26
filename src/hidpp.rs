//! Implementazione minimale di Logitech HID++ 2.0: trova le periferiche
//! (Bluetooth diretto o ricevitore Bolt/Unifying), ne legge nome e canale
//! Easy-Switch e cambia canale (feature 0x1814 "Change Host").

use hidapi::{HidApi, HidDevice};
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const LOGITECH_VID: u16 = 0x046D;
const REPORT_SHORT: u8 = 0x10;
const REPORT_LONG: u8 = 0x11;
const LONG_LEN: usize = 20;
const SW_ID: u8 = 0x0A;
const FEATURE_DEVICE_NAME: u16 = 0x0005;
const FEATURE_CHANGE_HOST: u16 = 0x1814;
pub const DEVICE_INDEX_DIRECT: u8 = 0xFF;

/// Collezioni HID con i report HID++ lunghi: ricevitori (0xFF00/0x0002) e BLE (0xFF43/0x0202).
const HIDPP_COLLECTIONS: [(u16, u16); 2] = [(0xFF00, 0x0002), (0xFF43, 0x0202)];

#[derive(Clone, Debug)]
pub struct LogiDevice {
    pub path: CString,
    pub device_index: u8,
    pub name: String,
    change_host_index: u8,
    pub num_hosts: u8,
    pub current_host: u8, // 0-based
}

impl std::fmt::Display for LogiDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let via = if self.device_index == DEVICE_INDEX_DIRECT {
            "Bluetooth".to_string()
        } else {
            format!("ricevitore slot {}", self.device_index)
        };
        write!(f, "{} ({via}) - canale {}/{}", self.name, self.current_host + 1, self.num_hosts)
    }
}

/// Un solo HidApi per processo; serializza anche scan e switch concorrenti.
fn api() -> Result<std::sync::MutexGuard<'static, HidApi>, String> {
    static API: OnceLock<Result<Mutex<HidApi>, String>> = OnceLock::new();
    let m = API
        .get_or_init(|| HidApi::new().map(Mutex::new).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| format!("hidapi: {e}"))?;
    Ok(m.lock().unwrap_or_else(|p| p.into_inner()))
}

struct Channel(HidDevice);

impl Channel {
    fn request(&self, idx: u8, feat: u8, func: u8, params: &[u8], timeout: Duration, reply: bool) -> Result<Vec<u8>, String> {
        let func_sw = ((func & 0x0F) << 4) | SW_ID;
        let mut msg = [0u8; LONG_LEN];
        msg[..4].copy_from_slice(&[REPORT_LONG, idx, feat, func_sw]);
        msg[4..4 + params.len()].copy_from_slice(params);

        let mut buf = [0u8; 64];
        // Svuota risposte vecchie (timeout 0 = non bloccante in hid_read_timeout)
        while self.0.read_timeout(&mut buf, 0).map_err(|e| e.to_string())? > 0 {}
        self.0.write(&msg).map_err(|e| format!("scrittura HID fallita: {e}"))?;
        if !reply {
            return Ok(Vec::new());
        }

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let n = self.0.read_timeout(&mut buf, 50).map_err(|e| e.to_string())?;
            let d = &buf[..n];
            if n < 5 || !(d[0] == REPORT_SHORT || d[0] == REPORT_LONG) || d[1] != idx {
                continue;
            }
            // 0x8F = errore HID++ 1.0 (es. slot vuoto), 0xFF = errore HID++ 2.0
            if (d[2] == 0x8F || d[2] == 0xFF) && d[3] == feat && d[4] == func_sw {
                return Err(format!("errore HID++ 0x{:02X}", d.get(5).copied().unwrap_or(0)));
            }
            if d[2] == feat && d[3] == func_sw {
                return Ok(d[4..].to_vec());
            }
        }
        Err("nessuna risposta dal dispositivo".into())
    }

    fn feature_index(&self, idx: u8, feature: u16, timeout: Duration) -> Result<u8, String> {
        let r = self.request(idx, 0x00, 0x00, &feature.to_be_bytes(), timeout, true)?;
        Ok(r.first().copied().unwrap_or(0))
    }

    fn name(&self, idx: u8) -> Result<String, String> {
        let t = Duration::from_secs(1);
        let feat = self.feature_index(idx, FEATURE_DEVICE_NAME, t)?;
        if feat == 0 {
            return Ok("Dispositivo Logitech".into());
        }
        let len = *self.request(idx, feat, 0x00, &[], t, true)?.first().unwrap_or(&0) as usize;
        let mut name = Vec::new();
        while name.len() < len {
            let chunk = self.request(idx, feat, 0x01, &[name.len() as u8], t, true)?;
            if chunk.is_empty() {
                break;
            }
            let take = chunk.len().min(len - name.len());
            name.extend_from_slice(&chunk[..take]);
        }
        let s = String::from_utf8_lossy(&name).trim_end_matches('\0').to_string();
        Ok(if s.is_empty() { "Dispositivo Logitech".into() } else { s })
    }
}

fn open(api: &HidApi, path: &CString) -> Result<Channel, String> {
    api.open_path(path).map(Channel).map_err(|e| e.to_string())
}

/// Trova tutte le periferiche Logitech connesse che supportano Easy-Switch.
pub fn discover() -> Result<Vec<LogiDevice>, String> {
    let mut api = api()?;
    api.refresh_devices().map_err(|e| e.to_string())?;
    let mut paths: Vec<(CString, u16)> = Vec::new();
    for info in api.device_list() {
        let key = (info.usage_page(), info.usage());
        if info.vendor_id() == LOGITECH_VID
            && HIDPP_COLLECTIONS.contains(&key)
            && !paths.iter().any(|(p, _)| p.as_c_str() == info.path())
        {
            paths.push((info.path().to_owned(), key.0));
        }
    }

    let mut found = Vec::new();
    for (path, usage_page) in paths {
        // Bluetooth diretto: un dispositivo (indice 0xFF). Ricevitore: slot 1..6.
        // Su Windows l'errore "slot vuoto" arriva sulla collezione short, quindi si usa un timeout.
        // shortcut: Bolt risponde in ~0.5 s, ogni slot vuoto costa 1 s; se troppo lento,
        // leggere l'errore 0x8F dalla collezione 0xFF00/0x0001.
        let (indices, timeout): (Vec<u8>, _) = if usage_page == 0xFF43 {
            (vec![DEVICE_INDEX_DIRECT], Duration::from_millis(600))
        } else {
            ((1..=6).collect(), Duration::from_secs(1))
        };
        let Ok(ch) = open(&api, &path) else { continue };
        for idx in indices {
            let probe = || -> Result<Option<LogiDevice>, String> {
                let feat = ch.feature_index(idx, FEATURE_CHANGE_HOST, timeout)?;
                if feat == 0 {
                    return Ok(None);
                }
                let name = ch.name(idx)?;
                let info = ch.request(idx, feat, 0x00, &[], Duration::from_secs(1), true)?;
                let (&num_hosts, &current_host) = (info.first().ok_or("risposta vuota")?, info.get(1).ok_or("risposta corta")?);
                Ok(Some(LogiDevice { path: path.clone(), device_index: idx, name, change_host_index: feat, num_hosts, current_host }))
            };
            if let Ok(Some(d)) = probe() {
                found.push(d); // errori = slot vuoto / dispositivo spento / non HID++ 2.0
            }
        }
    }
    Ok(found)
}

/// Sposta `dev` sul canale Easy-Switch `channel` (1-based).
pub fn switch_device(dev: &LogiDevice, channel: u8) -> Result<(), String> {
    if channel == 0 || channel > dev.num_hosts {
        return Err(format!("{}: canale {channel} non valido (1..{})", dev.name, dev.num_hosts));
    }
    let api = api()?;
    let ch = open(&api, &dev.path)?;
    // Il dispositivo si disconnette subito: nessuna risposta.
    ch.request(dev.device_index, dev.change_host_index, 0x01, &[channel - 1], Duration::ZERO, false)?;
    Ok(())
}
