# select-assist

选中文本，按快捷键，即可纠错或翻译。

> 暂定名称，需求对齐中。

## 构建与启动

```bash
cargo build --release
```

双击 `target/release/select-assist.exe` 启动。程序没有主窗口，只在任务栏托盘显示图标：右键图标进入设置或退出。重复双击不会多开。

配置文件：Windows 在 `%APPDATA%\select-assist\config.toml`，macOS 在 `~/Library/Application Support/select-assist/config.toml`。

## 许可证

本项目按 [PolyForm Noncommercial 1.0.0](LICENSE.md) 授权：可以免费用于个人、学习、研究和非营利组织，**禁止商业使用**。

界面基于 [Slint](https://slint.dev)，按 Slint Royalty-free 2.0 许可使用。该许可要求署名：程序内不放"关于 Slint"页面，改为在下载页（本 README）展示下面的徽章。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
