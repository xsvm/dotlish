#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod hotkeys;
mod input;
mod popup;
mod popup_size;
mod services;
mod settings;
mod tray;

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load();
    let agent = services::agent(&cfg)?;
    let mut registry = hotkeys::Registry::new()?;
    let startup_errors = registry.apply(&cfg.hotkeys);

    let popup = Popup::new()?;
    let settings = SettingsWindow::new()?;
    popup::setup(&popup);
    settings::setup(&settings);
    app::init(app::App {
        cfg,
        agent,
        registry,
        popup,
        settings,
        tray: None,
    });

    // 事件由各库在自己的线程里回调，转到 UI 线程处理；空闲时不占 CPU，不用轮询。
    GlobalHotKeyEvent::set_event_handler(Some(|ev: GlobalHotKeyEvent| {
        if ev.state == HotKeyState::Pressed {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(action) = app::with(|a| a.registry.action(ev.id)) {
                    popup::run(action);
                }
            });
        }
    }));
    MenuEvent::set_event_handler(Some(|ev: MenuEvent| {
        let _ = slint::invoke_from_event_loop(move || {
            match app::with(|a| a.tray.as_ref().and_then(|t| t.command(&ev.id))) {
                Some(tray::Command::Settings) => settings::open(""),
                Some(tray::Command::Quit) => {
                    let _ = slint::quit_event_loop();
                }
                None => {}
            }
        });
    }));
    TrayIconEvent::set_event_handler(Some(|ev: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = ev
        {
            let _ = slint::invoke_from_event_loop(|| settings::open(""));
        }
    }));

    // 托盘在 macOS 上必须等事件循环启动后创建
    slint::invoke_from_event_loop(move || {
        let mut errors = startup_errors;
        match tray::Tray::new() {
            Ok(t) => app::with(|a| a.tray = Some(t)),
            Err(e) => errors.push(format!("托盘图标创建失败：{e}")),
        }
        // 快捷键注册失败（比如被别的程序占用）时直接打开设置让用户改
        if !errors.is_empty() {
            settings::open(&errors.join("\n"));
        }
    })?;

    slint::run_event_loop_until_quit()?;
    Ok(())
}
