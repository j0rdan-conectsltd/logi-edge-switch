//! Avvio all'accesso dell'utente, con `--start` (attivo e ridotto a icona).
//! Windows: valore nella chiave `Run` dell'utente. macOS: LaunchAgent dell'utente.

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Logi Edge Switch";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // niente console che lampeggia

    fn reg(args: &[&str]) -> bool {
        Command::new("reg").args(args).creation_flags(CREATE_NO_WINDOW).output().is_ok_and(|o| o.status.success())
    }

    pub fn enabled() -> bool {
        reg(&["query", KEY, "/v", NAME])
    }

    pub fn set(on: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" --start", exe.display());
        let ok = if on { reg(&["add", KEY, "/v", NAME, "/t", "REG_SZ", "/d", &cmd, "/f"]) } else { reg(&["delete", KEY, "/v", NAME, "/f"]) };
        if ok { Ok(()) } else { Err("registro di Windows non aggiornato".into()) }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    const LABEL: &str = "app.logiedgeswitch";

    fn plist() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    pub fn enabled() -> bool {
        plist().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool) -> Result<(), String> {
        let path = plist().ok_or("HOME non impostata")?;
        if !on {
            return std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }).map_err(|e| e.to_string());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;");
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{exe}</string><string>--start</string></array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        );
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, xml).map_err(|e| e.to_string())
    }
}

/// `None` dove l'avvio automatico non è gestito (Linux: vedi README).
pub fn enabled() -> Option<bool> {
    #[cfg(any(windows, target_os = "macos"))]
    return Some(imp::enabled());
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}

pub fn set(on: bool) -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos"))]
    return imp::set(on);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = on;
        Err("non supportato su questo sistema".into())
    }
}
