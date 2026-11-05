//! Tray-Icon: Fenster schließen = in den Tray, Menü „Öffnen“, „Backup jetzt“, „Beenden“.
//! Ist kein Tray verfügbar (z. B. GNOME ohne AppIndicator), liefert `start` `None` und die App
//! verhält sich wie ohne Tray.
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayCmd {
    Open,
    Backup,
    Quit,
}

pub struct Tray {
    pub rx: Receiver<TrayCmd>,
    /// Unter Windows muss das Icon im Haupt-Thread (mit Nachrichtenschleife) leben
    #[cfg(windows)]
    _icon: TrayIcon,
}

fn icon() -> Option<Icon> {
    let d = eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icon.png")).ok()?;
    Icon::from_rgba(d.rgba, d.width, d.height).ok()
}

/// Baut Icon und Menü; liefert zusätzlich die Menü-IDs (Öffnen, Backup, Beenden).
fn build() -> Option<(TrayIcon, [MenuId; 3])> {
    let (open, backup, quit) = (MenuItem::new("Öffnen", true, None), MenuItem::new("Backup jetzt", true, None), MenuItem::new("Beenden", true, None));
    let menu = Menu::new();
    menu.append_items(&[&open, &backup, &PredefinedMenuItem::separator(), &quit]).ok()?;
    let tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("restic Backup").with_icon(icon()?).build().ok()?;
    Some((tray, [open.id().clone(), backup.id().clone(), quit.id().clone()]))
}

fn install_handlers(ids: [MenuId; 3], tx: Sender<TrayCmd>, ctx: egui::Context) {
    let tx_menu = tx.clone();
    let ctx_menu = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let cmd = if e.id == ids[0] {
            TrayCmd::Open
        } else if e.id == ids[1] {
            TrayCmd::Backup
        } else if e.id == ids[2] {
            TrayCmd::Quit
        } else {
            return;
        };
        let _ = tx_menu.send(cmd);
        ctx_menu.request_repaint();
    }));
    // Linksklick auf das Icon öffnet das Fenster (Linux/AppIndicator meldet keine Klicks)
    tray_icon::TrayIconEvent::set_event_handler(Some(move |e: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = e {
            let _ = tx.send(TrayCmd::Open);
            ctx.request_repaint();
        }
    }));
}

/// Legt das Tray-Icon an. Linux: eigener Thread mit GTK-Hauptschleife; scheitert das
/// (fehlende Bibliothek, kein Tray-Dienst), gibt es `None`.
#[cfg(target_os = "linux")]
pub fn start(ctx: egui::Context) -> Option<Tray> {
    let (tx, rx) = channel();
    let (ready_tx, ready_rx) = channel::<bool>();
    std::thread::spawn(move || {
        // Ein Panic (z. B. libappindicator fehlt) lässt ready_tx fallen → Fallback unten
        if gtk::init().is_err() {
            let _ = ready_tx.send(false);
            return;
        }
        let Some((tray, ids)) = build() else {
            let _ = ready_tx.send(false);
            return;
        };
        install_handlers(ids, tx, ctx);
        let _ = ready_tx.send(true);
        gtk::main();
        drop(tray);
    });
    match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(true) => Some(Tray { rx }),
        _ => None,
    }
}

#[cfg(windows)]
pub fn start(ctx: egui::Context) -> Option<Tray> {
    let (tx, rx) = channel();
    let (icon, ids) = build()?;
    install_handlers(ids, tx, ctx);
    Some(Tray { rx, _icon: icon })
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn start(_ctx: egui::Context) -> Option<Tray> {
    None
}
