# 路线图

[English](ROADMAP.md)

mouseshare 目前能做什么、接下来计划做什么，以及最欢迎哪些帮助。标有 🙋 的条目欢迎贡献者认领——
开始较大的工作之前，请先在对应 issue 下留言（或新建一个）。

## 已完成

- 任意数量的电脑，每台任意数量的显示器（RandR），在可拖拽吸附的画布上排列，并支持一键对齐。
- 指针移动、按键、滚轮和键盘；纯文本剪贴板。
- 配对码 + SPAKE2 认证 + ChaCha20-Poly1305 加密；在线猜测限速。
- 自动重连、每台设备实时状态与延迟、失效链路检测。
- 紧急返回快捷键、拖拽保护、边缘停留时间、每台设备的指针速度、自然滚动选项、剪贴板大小上限。
- 图形界面：深色 / 浅色 / 自动主题、六种强调色、界面缩放、英文 + 简体中文、首次引导、设置窗口、消息提示、开机自启。
- 与图形界面共用设置的命令行、`.deb` 和压缩包发布、基于真实 X 服务器的端到端测试、每个 Pull Request 都运行 CI。

## 接下来

| 条目 | 说明 |
|---|---|
| 🙋 系统托盘 / 最小化到托盘 | 需要 StatusNotifierItem（D-Bus）或 GTK AppIndicator；在不同桌面上表现不一。 |
| 🙋 更多语言翻译 | 界面文字在 `crates/gui/src/strings.rs`；`*.zh-CN.md` 文档可作为其他语言的范本。 |
| 🙋 图形界面中的排列导入/导出 | `LayoutConfig::to_toml_string` 已能写出命令行使用的格式。 |
| 多套排列方案 | 家里 / 办公室预设。 |
| 跳转到指定屏幕的快捷键 | 用快捷键把光标送到指定电脑。 |
| 图片和富文本剪贴板 | 需要感知大小的剪贴板协议。 |
| 更多发行版的软件包 | `.rpm`、AUR、Flatpak/AppImage。 |

## 之后 / 较大的项目

| 条目 | 说明 |
|---|---|
| 🙋 **Wayland** | 需要第二个输入后端（注入用 libei / XDG RemoteDesktop portal，捕获用合成器协议）。接缝在 `x11input` crate，第一步是把它的具体类型抽成 trait。 |
| 🙋 **Windows 和 macOS** | 同样走后端接缝；键码需要跨平台表示（目前 `KeyEvent` 携带原始 X11 键码）。 |
| 🙋 电脑之间拖放文件 | 需要各平台各自的文件传输协议。 |
| 多个控制端 | 目前是一个控制端操控多个被控端。 |

## 明确不做

- 不安全 / 免认证模式：在共用网络里是个陷阱。
- 云中继或账号：mouseshare 设计上只用于局域网。
- 远程桌面 / 屏幕串流：请使用专门的远程桌面工具。

## 如何帮忙

见 [CONTRIBUTING.zh-CN.md](../CONTRIBUTING.zh-CN.md)。适合入门的贡献：一份翻译、一处文档修正、
一份附日志的 bug 报告，或在我们没试过的桌面环境上测试。标有
[`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted)
和 [`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
的 issue 是最好的起点。
