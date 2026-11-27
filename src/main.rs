//! Logi Edge Switch: spingendo il cursore contro un bordo esterno (o dentro un
//! monitor condiviso) sposta tastiera e mouse Logitech sul canale Easy-Switch
//! dell'altro PC. Va eseguito su ogni PC, ciascuno con il proprio config.json.
//!
//! Uso:
//!   logi-edge-switch                 interfaccia grafica
//!   logi-edge-switch --start         interfaccia grafica, già attivo (per l'avvio automatico)
//!   logi-edge-switch --list          elenca periferiche e canali
//!   logi-edge-switch --switch 2      passa subito al canale 2
//!   logi-edge-switch -c altro.json   config diverso
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;
mod gui;
mod hidpp;

use config::Config;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn main() {
    #[cfg(windows)]
    unsafe {
        // Senza questo, con il ridimensionamento (125%, 150%...) coordinate di cursore e monitor non coincidono.
        use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1));

    if flag("--list") || value("--switch").is_some() {
        #[cfg(windows)]
        unsafe {
            // In release l'exe è "GUI": per stampare riusa la console da cui è stato lanciato.
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        let devs = hidpp::discover().unwrap_or_else(|e| {
            eprintln!("Errore: {e}");
            std::process::exit(1)
        });
        if let Some(ch) = value("--switch") {
            let ch: u8 = ch.parse().unwrap_or(0);
            for d in &devs {
                match hidpp::switch_device(d, ch) {
                    Ok(()) => println!("{} -> canale {ch}", d.name),
                    Err(e) => println!("{}: errore {e}", d.name),
                }
            }
        } else if devs.is_empty() {
            println!("Nessuna periferica Logitech con Easy-Switch trovata.");
        } else {
            devs.iter().for_each(|d| println!("- {d}"));
        }
        return;
    }

    let cfg_path = value("-c").map(PathBuf::from).unwrap_or_else(|| {
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("config.json"))).unwrap_or("config.json".into())
    });
    let (cfg, err) = if cfg_path.exists() {
        match Config::load(&cfg_path) {
            Ok(c) => (c, None),
            Err(e) => {
                // "Salva" sovrascriverebbe il file: tieni una copia dell'originale
                let _ = std::fs::copy(&cfg_path, cfg_path.with_extension("json.bak"));
                (Config::default(), Some(e))
            }
        }
    } else {
        (Config::default(), None)
    };

    let sh = engine::Shared::new(cfg);
    if let Some(e) = err {
        sh.log(format!("[!] config non valido, uso i valori predefiniti: {e}"));
    }
    sh.log(format!("[i] config: {}", cfg_path.display()));
    sh.running.store(flag("--start"), Ordering::Relaxed);
    sh.rescan_async();
    engine::spawn(sh.clone());

    gui::run(sh, cfg_path, flag("--start"));
}
