//! 弹窗尺寸与位置的纯计算，不依赖窗口系统，方便测试。

const RATIO_W: f32 = 800.0 / 2560.0;
const RATIO_H: f32 = 400.0 / 1440.0;
// 高缩放屏幕上按比例算出来太小，内容装不下
const MIN_W: f32 = 640.0;
const MIN_H: f32 = 320.0;
/// 弹窗与光标的间距（逻辑像素）
const GAP: f32 = 12.0;

/// 按屏幕比例（2560×1440 上 800×400）计算，并设逻辑像素下限。输入输出都是逻辑尺寸。
pub fn size(screen_w: f32, screen_h: f32) -> (f32, f32) {
    (
        (screen_w * RATIO_W).max(MIN_W).round(),
        (screen_h * RATIO_H).max(MIN_H).round(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// 右边和下边不算在内。
    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug, PartialEq)]
pub struct Placement {
    /// 逻辑尺寸
    pub size: (f32, f32),
    /// 左上角，单位与 screen 相同
    pub pos: (f32, f32),
}

/// 弹窗放在光标右下方，超出屏幕时往回收；下方放不下就放到光标上方。
/// cursor 与 screen 用同一种坐标单位，unit 是该单位与逻辑像素之比
/// （Windows 物理像素时等于缩放比例，macOS 逻辑点时为 1）。
pub fn place(cursor: (f32, f32), screen: Rect, unit: f32) -> Placement {
    let (lw, lh) = size(screen.w / unit, screen.h / unit);
    let (w, h, gap) = (lw * unit, lh * unit, GAP * unit);

    let x = (cursor.0 + gap).min(screen.x + screen.w - w).max(screen.x);
    let below = cursor.1 + gap;
    let y = if below + h <= screen.y + screen.h {
        below
    } else {
        (cursor.1 - gap - h).max(screen.y)
    };
    Placement {
        size: (lw, lh),
        pos: (x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qhd_at_100_percent() {
        assert_eq!(size(2560.0, 1440.0), (800.0, 400.0));
    }

    #[test]
    fn qhd_at_125_percent() {
        // 2560×1440 @125% 的逻辑尺寸是 2048×1152，正好落在下限
        assert_eq!(size(2048.0, 1152.0), (640.0, 320.0));
    }

    #[test]
    fn high_dpi_uses_floor() {
        assert_eq!(size(1280.0, 800.0), (640.0, 320.0));
    }

    #[test]
    fn large_screen_scales_up() {
        assert_eq!(size(3840.0, 2160.0), (1200.0, 600.0));
    }

    const QHD: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 2560.0,
        h: 1440.0,
    };

    #[test]
    fn below_right_of_cursor() {
        let p = place((100.0, 100.0), QHD, 1.0);
        assert_eq!(
            p,
            Placement {
                size: (800.0, 400.0),
                pos: (112.0, 112.0)
            }
        );
    }

    #[test]
    fn clamped_at_right_edge_and_flipped_above() {
        let p = place((2500.0, 1400.0), QHD, 1.0);
        assert_eq!(p.pos, (2560.0 - 800.0, 1400.0 - 12.0 - 400.0));
    }

    #[test]
    fn contains_point() {
        let r = Rect {
            x: 100.0,
            y: 50.0,
            w: 800.0,
            h: 400.0,
        };
        assert!(r.contains((100.0, 50.0)));
        assert!(r.contains((899.0, 449.0)));
        assert!(!r.contains((900.0, 200.0)));
        assert!(!r.contains((500.0, 49.0)));
    }

    #[test]
    fn physical_pixels_on_second_monitor() {
        // 右侧第二块屏幕，2560×1600 物理像素 @200%
        let screen = Rect {
            x: 2560.0,
            y: 0.0,
            w: 2560.0,
            h: 1600.0,
        };
        let p = place((2600.0, 100.0), screen, 2.0);
        assert_eq!(
            p,
            Placement {
                size: (640.0, 320.0),
                pos: (2624.0, 124.0)
            }
        );
    }
}
