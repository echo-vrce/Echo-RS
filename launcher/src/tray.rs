//! Hide to tray: the window goes away (nothing is drawn, so the launcher
//! costs next to nothing) while the servers keep running; the tray icon
//! brings it back or quits.
//!
//! Windows uses `tray-icon`; Linux uses `ksni` (StatusNotifierItem over
//! D-Bus, pure Rust). A hidden window can't wake itself, so on Windows the
//! tray shows it again directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use eframe::egui;

/// What the tray asks of the window, read in `update`.
pub struct Bridge {
    pub ctx: egui::Context,
    show: AtomicBool,
    quit: AtomicBool,
    /// Windows: the launcher's HWND.
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: AtomicIsize,
}

impl Bridge {
    pub fn new(ctx: egui::Context) -> Arc<Self> {
        Arc::new(Self { ctx, show: AtomicBool::new(false), quit: AtomicBool::new(false), hwnd: AtomicIsize::new(0) })
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_hwnd(&self, h: isize) {
        self.hwnd.store(h, Ordering::Relaxed);
    }

    fn wake(&self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, SetForegroundWindow, ShowWindow};
            let h = self.hwnd.load(Ordering::Relaxed);
            if h != 0 {
                // SAFETY: the handle is the launcher's own top-level window, alive while the app runs.
                unsafe {
                    ShowWindow(h as _, SW_SHOW);
                    SetForegroundWindow(h as _);
                }
            }
        }
        self.ctx.request_repaint();
    }

    pub fn request_show(&self) {
        self.show.store(true, Ordering::Relaxed);
        self.wake();
    }

    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.wake();
    }

    pub fn take_show(&self) -> bool {
        self.show.swap(false, Ordering::Relaxed)
    }

    pub fn take_quit(&self) -> bool {
        self.quit.swap(false, Ordering::Relaxed)
    }
}

/// A 32x32 round icon in the launcher's accent colour, as RGBA.
fn icon_rgba() -> Vec<u8> {
    let mut v = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((15.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
            let inner = d < 6.0;
            let (r, g, b) = if inner { (255, 255, 255) } else { (88, 140, 255) };
            v.extend_from_slice(&[r, g, b, a]);
        }
    }
    v
}

/// The tray icon, alive while the app keeps it.
pub struct Tray {
    #[cfg(windows)]
    _icon: tray_icon::TrayIcon,
    #[cfg(target_os = "linux")]
    _handle: ksni::blocking::Handle<LinuxTray>,
}

#[cfg(windows)]
pub fn create(bridge: Arc<Bridge>) -> Result<Tray, String> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::new("Show launcher", true, None);
    let quit = MenuItem::new("Stop servers and quit", true, None);
    let menu = Menu::new();
    menu.append(&show).map_err(|e| e.to_string())?;
    menu.append(&quit).map_err(|e| e.to_string())?;
    let (show_id, quit_id) = (show.id().clone(), quit.id().clone());

    let b = bridge.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == show_id {
            b.request_show();
        } else if e.id == quit_id {
            b.request_quit();
        }
    }));
    let b = bridge;
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            b.request_show();
        }
    }));

    let icon = Icon::from_rgba(icon_rgba(), 32, 32).map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Echo Server Launcher (servers running)")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(Tray { _icon: tray })
}

#[cfg(target_os = "linux")]
pub struct LinuxTray {
    bridge: Arc<Bridge>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "echo-server-launcher".into()
    }
    fn title(&self) -> String {
        "Echo Server Launcher".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // RGBA -> ARGB.
        let data = icon_rgba().chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        vec![ksni::Icon { width: 32, height: 32, data }]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.bridge.request_show();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem { label: "Show launcher".into(), activate: Box::new(|t: &mut Self| t.bridge.request_show()), ..Default::default() }.into(),
            StandardItem { label: "Stop servers and quit".into(), activate: Box::new(|t: &mut Self| t.bridge.request_quit()), ..Default::default() }
                .into(),
        ]
    }
}

#[cfg(target_os = "linux")]
pub fn create(bridge: Arc<Bridge>) -> Result<Tray, String> {
    use ksni::blocking::TrayMethods;
    let handle = LinuxTray { bridge }.spawn().map_err(|e| e.to_string())?;
    Ok(Tray { _handle: handle })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn create(_bridge: Arc<Bridge>) -> Result<Tray, String> {
    Err("no tray on this system".into())
}
