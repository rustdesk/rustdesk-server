//! 系统托盘模块
//!
//! 使用 tray-icon 0.24 实现跨平台系统托盘。
//! 托盘图标颜色反映当前连接状态（绿=在线，红=离线）。
//!
//! ## 平台说明
//! - **Windows**：使用 Win32 Shell 通知区图标
//! - **macOS**：使用 NSStatusBar
//! - **Linux**：需要 libayatana-appindicator3 或 libappindicator3
//!   安装：`sudo apt install libayatana-appindicator3-dev`

use core_common::log;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

// ──────────────────────────────────────────────────────────────────────────────
// 菜单项 ID（全局，用于事件匹配）
// ──────────────────────────────────────────────────────────────────────────────

/// 托盘菜单事件类型
#[derive(Debug, Clone, PartialEq)]
pub enum TrayAction {
    /// 显示/隐藏主窗口
    ToggleWindow,
    /// 打开首页
    GoHome,
    /// 打开连接页
    GoConnect,
    /// 打开账户页
    GoAccount,
    /// 退出程序
    Quit,
}

/// 待处理的托盘动作（由主事件循环处理）
pub struct TrayManager {
    _tray: TrayIcon,
    show_hide_item_id: tray_icon::menu::MenuId,
    home_item_id: tray_icon::menu::MenuId,
    connect_item_id: tray_icon::menu::MenuId,
    account_item_id: tray_icon::menu::MenuId,
    quit_item_id: tray_icon::menu::MenuId,
    online: bool,
}

impl TrayManager {
    /// 创建系统托盘图标和菜单
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let icon = make_icon(false);

        // 构建菜单
        let show_hide_item = MenuItem::new("显示/隐藏窗口", true, None);
        let home_item = MenuItem::new("首页", true, None);
        let connect_item = MenuItem::new("连接对端…", true, None);
        let account_item = MenuItem::new("账户", true, None);
        let quit_item = MenuItem::new("退出", true, None);

        let show_hide_id = show_hide_item.id().clone();
        let home_id = home_item.id().clone();
        let connect_id = connect_item.id().clone();
        let account_id = account_item.id().clone();
        let quit_id = quit_item.id().clone();

        let menu = Menu::new();
        menu.append_items(&[
            &show_hide_item,
            &PredefinedMenuItem::separator(),
            &home_item,
            &connect_item,
            &account_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("NAT Client — 离线")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()?;

        Ok(Self {
            _tray: tray,
            show_hide_item_id: show_hide_id,
            home_item_id: home_id,
            connect_item_id: connect_id,
            account_item_id: account_id,
            quit_item_id: quit_id,
            online: false,
        })
    }

    /// 更新托盘图标和 tooltip（根据在线状态）
    pub fn set_online(&mut self, online: bool) {
        if self.online == online {
            return;
        }
        self.online = online;
        let icon = make_icon(online);
        let tooltip = if online {
            "NAT Client — 已上线"
        } else {
            "NAT Client — 离线"
        };
        let _ = self._tray.set_icon(Some(icon));
        let _ = self._tray.set_tooltip(Some(tooltip));
    }

    /// 轮询一次托盘事件（在 Slint timer 中调用，每 50ms 一次）
    ///
    /// 返回 `Some(TrayAction)` 表示有动作需要处理。
    pub fn poll(&self) -> Option<TrayAction> {
        // 先检查图标点击
        if let Ok(event) = TrayIconEvent::receiver().try_recv() {
            use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent as E};
            if let E::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                return Some(TrayAction::ToggleWindow);
            }
        }

        // 再检查菜单事件
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id();
            if id == &self.show_hide_item_id {
                return Some(TrayAction::ToggleWindow);
            }
            if id == &self.home_item_id {
                return Some(TrayAction::GoHome);
            }
            if id == &self.connect_item_id {
                return Some(TrayAction::GoConnect);
            }
            if id == &self.account_item_id {
                return Some(TrayAction::GoAccount);
            }
            if id == &self.quit_item_id {
                return Some(TrayAction::Quit);
            }
        }

        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 图标生成（程序内生成，不依赖外部图标文件）
// ──────────────────────────────────────────────────────────────────────────────

/// 生成 32×32 RGBA 圆形图标
///
/// 在线：蓝色外环 + 绿色内圆
/// 离线：灰色外环 + 红色内圆
fn make_icon(online: bool) -> Icon {
    const SIZE: u32 = 32;
    const R: f32 = SIZE as f32 / 2.0;

    // 颜色
    let outer_color: [u8; 4] = [0x4c, 0x6e, 0xf5, 255]; // 主蓝
    let inner_color: [u8; 4] = if online {
        [0x40, 0xc0, 0x57, 255] // 绿
    } else {
        [0x86, 0x8e, 0x96, 255] // 灰
    };
    let bg: [u8; 4] = [0x1a, 0x1b, 0x1e, 0]; // 透明背景

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - R + 0.5;
            let dy = y as f32 - R + 0.5;
            let dist = (dx * dx + dy * dy).sqrt();

            let color = if dist <= R * 0.46 {
                inner_color // 内圆
            } else if dist <= R - 1.0 {
                outer_color // 外环
            } else {
                bg // 透明背景
            };

            let idx = ((y * SIZE + x) * 4) as usize;
            pixels[idx..idx + 4].copy_from_slice(&color);
        }
    }

    // 在内圆中心叠加字母 "N"（简单像素字，5×7）
    if online {
        stamp_letter_n(&mut pixels, SIZE);
    }

    Icon::from_rgba(pixels, SIZE, SIZE).expect("托盘图标创建失败")
}

/// 在图标中央叠加像素字母 "N"（白色，5×7 像素）
fn stamp_letter_n(pixels: &mut [u8], size: u32) {
    // 5列×7行的"N"点阵（1=白色）
    #[rustfmt::skip]
    let pattern: [[u8; 5]; 7] = [
        [1, 0, 0, 0, 1],
        [1, 1, 0, 0, 1],
        [1, 0, 1, 0, 1],
        [1, 0, 0, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
    ];

    let ox = (size / 2 - 3) as usize;
    let oy = (size / 2 - 4) as usize;

    for (row, cols) in pattern.iter().enumerate() {
        for (col, &on) in cols.iter().enumerate() {
            if on == 1 {
                let px = ox + col;
                let py = oy + row;
                let idx = (py * size as usize + px) * 4;
                if idx + 3 < pixels.len() {
                    pixels[idx] = 255; // R
                    pixels[idx + 1] = 255; // G
                    pixels[idx + 2] = 255; // B
                    pixels[idx + 3] = 200; // A
                }
            }
        }
    }
}
