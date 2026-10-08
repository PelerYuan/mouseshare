# 更新日志

[English](CHANGELOG.md)

本项目所有值得注意的变更都记录在这里。格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

## [0.2.0]

一次重建的、可用于生产的版本：安全、多设备、多显示器，并带有全新的图形界面。

### 新增

- **任意数量的电脑**，每台任意数量的显示器。布局模型把每台电脑视为刚性的显示器组；
  显示器通过 RandR 自动检测，并在连接和热插拔时刷新。
- **配对码认证与加密**：SPAKE2 密钥交换、ChaCha20-Poly1305 记录、在线猜测限速。每台电脑随机生成
  `XXXXX-XXXXX` 配对码，保存在 `~/.config/mouseshare/pairing.toml`（权限 `0600`）。
- **鼠标按键和滚轮**转发（此前只转发移动和键盘），包括侧键和横向滚动。
- 光标现在会**出现在穿越边缘的位置**，而不是远程电脑上的任意位置。
- **自动重连**（指数退避）、每台设备**实时延迟**与状态、失效链路检测。
- **紧急返回快捷键**（默认 `Ctrl+Alt+Esc`）、**拖拽保护**、**边缘停留时间**、每台设备的**指针速度**、
  **自然滚动**、剪贴板开关与大小上限。
- 新的 **`config` crate**：设置和配对码被持久化，并由命令行和图形界面共用。`mouseshare pair-code`
  可显示 / 重新生成配对码。
- **图形界面重写**：深色 / 浅色 / 自动主题、六种强调色、界面缩放、英文和简体中文（运行时发现 CJK 字体）、
  首次引导、设置窗口、消息提示、可逐设备编辑的设备列表、带吸附和一键对齐的多显示器画布、
  被控电脑的配对码页面、开机自启。
- **打包**：`.deb`、带 `install.sh` 的压缩包、桌面入口和图标；版本标签触发的发布工作流。
- **项目健康**：`docs/`（用户指南、架构、协议与安全设计、故障排查、路线图）、每份文档的简体中文翻译、
  安全策略、Issue 表单、Dependabot、CODEOWNERS、扩充后的 CI（格式、静态检查、Xvfb 下的测试、依赖审计、发布构建）。

### 变更

- **协议版本 2**（与 0.1.x 不兼容）：握手中包含显示器列表，新增按键、滚轮、绝对定位、ping/pong 以及加密传输。
- `layout.example.toml` 改用多显示器的 `[[devices]]` 形式；旧的 `[[screens]]` 形式仍可加载。
- 命令行支持 `--pair-code`、可重复的 `--connect id=host:port`，未指定 `--config` 时使用已保存的设置。
- 日志输出到 stderr，使 stdout 保持干净。

### 移除

- 无认证、无加密的传输。刻意不提供“不安全模式”。

## [0.1.0]

首次公开发布。在两台 Linux/X11 电脑之间通过局域网共享鼠标和键盘，包括：

- 基于边缘的控制权切换（`crates/layout`），使用可配置的虚拟桌面布局
- X11 指针/键盘捕获与注入（`crates/x11input`）
- 带长度前缀的 TCP 线上协议（`crates/protocol`、`crates/net`）
- 同一连接上的双向纯文本剪贴板同步
- 通过 mDNS 在局域网自动发现（`crates/discovery`），仍支持手动输入 IP
- 可选的 `mouseshare-gui`（`crates/gui`，`egui`/`eframe`），用于可视化配置和连接管理，与轻量的 `mouseshare` 命令行并存
- 尚无认证和加密：此版本仅适用于可信局域网

[未发布]: https://github.com/PelerYuan/mouseshare/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/PelerYuan/mouseshare/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/PelerYuan/mouseshare/releases/tag/v0.1.0
