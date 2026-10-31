//! 托盘图标：右键菜单进入设置或退出，左键单击直接打开设置。

use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub enum Command {
    Settings,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    settings: MenuId,
    quit: MenuId,
}

impl Tray {
    /// macOS 要求在事件循环启动之后创建。
    pub fn new() -> Result<Self, String> {
        let settings = MenuItem::new("设置…", true, None);
        let quit = MenuItem::new("退出", true, None);
        let menu = Menu::new();
        menu.append_items(&[&settings, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;

        let icon = Icon::from_rgba(icon_rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
            .map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("select-assist")
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _icon: tray,
            settings: settings.id().clone(),
            quit: quit.id().clone(),
        })
    }

    pub fn command(&self, id: &MenuId) -> Option<Command> {
        if *id == self.settings {
            Some(Command::Settings)
        } else if *id == self.quit {
            Some(Command::Quit)
        } else {
            None
        }
    }
}

const ICON_SIZE: u32 = 32;

/// 窗口图标与托盘图标一致。
pub fn window_icon() -> slint::Image {
    let px = icon_rgba(ICON_SIZE);
    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &px, ICON_SIZE, ICON_SIZE,
    ))
}

/// 临时图标：蓝色圆角方块上两条白线（代表文字）。以后换成正式图标。
fn icon_rgba(size: u32) -> Vec<u8> {
    const ACCENT: [u8; 4] = [0x2f, 0x6f, 0xde, 0xff];
    const WHITE: [u8; 4] = [0xff; 4];
    let s = size as f32;
    let r = s * 0.22;
    let line = |y: f32, x0: f32, x1: f32, px: f32, py: f32| {
        (py - y).abs() <= s * 0.06 && px >= x0 && px <= x1
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // 到最近圆角圆心的距离，圆角外透明
            let dx = (r - px).max(px - (s - r)).max(0.0);
            let dy = (r - py).max(py - (s - r)).max(0.0);
            let pixel = if dx * dx + dy * dy > r * r {
                [0; 4]
            } else if line(s * 0.38, s * 0.22, s * 0.78, px, py)
                || line(s * 0.62, s * 0.22, s * 0.6, px, py)
            {
                WHITE
            } else {
                ACCENT
            };
            out.extend_from_slice(&pixel);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::icon_rgba;

    #[test]
    fn icon_shape() {
        let px = icon_rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let at = |x: usize, y: usize| &px[(y * 32 + x) * 4..][..4];
        assert_eq!(at(0, 0)[3], 0, "圆角外透明");
        assert_eq!(at(16, 12), [0xff; 4], "第一条白线");
        assert_eq!(at(16, 26), [0x2f, 0x6f, 0xde, 0xff], "底色");
    }
}
