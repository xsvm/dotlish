//! 全局状态。只在 UI 线程访问。

use crate::config::Config;
use crate::hotkeys::Registry;
use crate::tray::Tray;
use crate::{Popup, SettingsWindow};
use slint::ComponentHandle;
use std::cell::RefCell;
use ureq::Agent;

pub struct App {
    pub cfg: Config,
    /// 共用一个 Agent 复用连接；改代理后整体替换
    pub agent: Agent,
    pub registry: Registry,
    pub popup: Popup,
    pub settings: SettingsWindow,
    pub tray: Option<Tray>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn init(app: App) {
    APP.with(|a| *a.borrow_mut() = Some(app));
}

/// 闭包里不要调用会再次进入 with 的函数（比如显示窗口的流程），先把需要的东西取出来。
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("App 未初始化")))
}

pub fn popup() -> Popup {
    with(|a| a.popup.clone_strong())
}

pub fn settings() -> SettingsWindow {
    with(|a| a.settings.clone_strong())
}
