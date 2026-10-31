# Dotlish（点词）

给英语学习者的划词助手：在任何程序里选中文本，按快捷键，即可翻译、查词、听发音或纠错，还能把结果直接替换回原处。

## 下载

在 [Releases](https://github.com/xsvm/dotlish/releases) 下载对应平台的压缩包，解压后运行：

- Windows：`dotlish.exe`
- macOS：`dotlish`（Apple 芯片选 `aarch64`，Intel 选 `x86_64`）。**macOS 版尚未实测**；程序没有签名，第一次运行前需要执行 `xattr -d com.apple.quarantine dotlish`，并在“系统设置 → 隐私与安全性 → 辅助功能”里允许它（读取选中文本和监听快捷键需要）。

## 功能

- **翻译**：Bing → 火山 → 阿里 → Google → MyMemory 依次兜底，都不需要 API key。
- **查词**：选中单个英文单词时显示有道词典释义；翻译结果里点任意英文单词也能查，附英式、美式发音。
- **纠错**：LanguageTool 公共接口，逐条选择候选后一键修正回原处。
- **替换**：翻译或修正结果可以直接替换掉原来选中的文本（例如用中文写好一句话，翻译成英文后替换）。
- 快捷键可以是任意键、组合键或鼠标侧键；弹窗开着时按另一个快捷键直接切换功能，点弹窗外面关闭。

## 构建与启动

```bash
cargo build --release
```

双击 `target/release/dotlish.exe` 启动。程序没有主窗口，只在任务栏托盘显示图标：右键图标进入设置或退出。重复双击不会多开。

配置文件：Windows 在 `%APPDATA%\dotlish\config.toml`，macOS 在 `~/Library/Application Support/dotlish/config.toml`。旧版（select-assist）的配置会在第一次启动时自动搬过来。

## 许可证

本项目按 [PolyForm Noncommercial 1.0.0](LICENSE.md) 授权：可以免费用于个人、学习、研究和非营利组织，**禁止商业使用**。

界面基于 [Slint](https://slint.dev)，按 Slint Royalty-free 2.0 许可使用。该许可要求署名：程序内不放"关于 Slint"页面，改为在下载页（本 README）展示下面的徽章。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
