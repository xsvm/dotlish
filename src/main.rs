// 托盘程序，不要控制台窗口（debug 版也一样，出错信息显示在设置窗口里）
#![windows_subsystem = "windows"]

mod app;
mod audio;
mod config;
mod hotkeys;
mod input;
mod popup;
mod popup_size;
mod services;
mod settings;
mod tray;

use hotkeys::Signal;
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

slint::include_modules!();

/// 单实例锁名：Windows 上是命名互斥量，macOS 上是加锁的文件路径。
fn instance_name() -> String {
    if cfg!(target_os = "macos") {
        std::env::temp_dir()
            .join("dotlish.lock")
            .display()
            .to_string()
    } else {
        "dotlish-single-instance".into()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 已经在运行时直接退出，避免两个实例同时监听快捷键
    let instance = single_instance::SingleInstance::new(&instance_name())?;
    if !instance.is_single() {
        return Ok(());
    }

    let cfg = config::load();
    let agent = services::agent(&cfg)?;
    std::thread::spawn({
        let agent = agent.clone();
        move || services::bing::warm_up(&agent)
    });

    let popup = Popup::new()?;
    let settings = SettingsWindow::new()?;
    popup::setup(&popup);
    settings::setup(&settings);

    let mut errors = Vec::new();
    if let Err(e) = settings::activate(&cfg) {
        errors.push(e);
    }
    app::init(app::App {
        cfg,
        agent,
        popup,
        settings,
        listener: None,
        tray: None,
    });

    // 事件由各库在自己的线程里回调，转到 UI 线程处理；空闲时不占 CPU，不用轮询。
    match hotkeys::start(|signal| {
        let _ = slint::invoke_from_event_loop(move || match signal {
            Signal::Fire(action) => popup::run(action),
            Signal::Recorded(b) => settings::on_recorded(Some(b)),
            Signal::RecordCancelled => settings::on_recorded(None),
            Signal::Click => popup::on_click(),
        });
    }) {
        Ok(l) => app::with(|a| a.listener = Some(l)),
        Err(e) => errors.push(e),
    }
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
        match tray::Tray::new() {
            Ok(t) => app::with(|a| a.tray = Some(t)),
            Err(e) => errors.push(format!("托盘图标创建失败：{e}")),
        }
        // 快捷键有问题时直接打开设置让用户改
        if !errors.is_empty() {
            settings::open(&errors.join("\n"));
        }
    })?;

    slint::run_event_loop_until_quit()?;
    app::shutdown();
    Ok(())
}
