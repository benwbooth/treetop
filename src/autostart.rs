//! "Start at login": an XDG autostart entry that starts treetop in the tray.
//! When devloop manages treetop (its launcher is marked X-Devloop), the entry
//! goes through devloop so the dev build keeps rebuilding; otherwise it runs
//! the installed binary.

use std::path::PathBuf;

const ID: &str = "com.benwbooth.treetop.desktop";

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

fn entry_path() -> PathBuf {
    config_home().join("autostart").join(ID)
}

/// The command that launches treetop: devloop's when it manages treetop,
/// else whatever `treetop` is on PATH (the Nix wrapper), else this binary.
fn launch_command() -> String {
    let launcher = data_home().join("applications").join(ID);
    if let Ok(text) = std::fs::read_to_string(&launcher) {
        if text.lines().any(|l| l.trim() == "X-Devloop=true") {
            if let Some(exec) = text.lines().find_map(|l| l.strip_prefix("Exec=")) {
                return exec.trim_end_matches(" %U").trim_end_matches(" %u").to_string();
            }
        }
    }
    let on_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| PathBuf::from(d).join("treetop"))
        .find(|p| p.exists());
    on_path
        .or_else(|| std::env::current_exe().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "treetop".into())
}

/// Whether the treetop widget is enabled in a Plasma system tray (then the
/// app doesn't need a tray icon of its own).
pub fn tray_widget_present() -> bool {
    let rc = config_home().join("plasma-org.kde.plasma.desktop-appletsrc");
    std::fs::read_to_string(rc)
        .map(|t| t.lines().any(|l| l.starts_with("extraItems=") && l.split(['=', ',']).any(|i| i == "com.benwbooth.treetop")))
        .unwrap_or(false)
}

pub fn enabled() -> bool {
    std::fs::read_to_string(entry_path())
        .map(|t| !t.lines().any(|l| l.trim() == "Hidden=true"))
        .unwrap_or(false)
}

pub fn set(on: bool) -> Result<String, String> {
    let path = entry_path();
    if !on {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok("treetop won't start at login".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("treetop won't start at login".into()),
            Err(e) => Err(format!("couldn't remove {}: {e}", path.display())),
        };
    }
    let cmd = launch_command();
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=treetop\n\
         Comment=System monitor in the tray\n\
         Exec={cmd} --hidden\n\
         Icon=treetop\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         X-KDE-autostart-phase=2\n"
    );
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, entry).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok("treetop will start in the tray at login".into())
}
