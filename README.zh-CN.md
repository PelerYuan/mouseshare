<div align="center">

<img src="packaging/mouseshare.svg" width="96" alt="mouseshare 标志">

# mouseshare

**一套鼠标、一套键盘、一份剪贴板，操作你的所有电脑。**

[![CI](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml/badge.svg)](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/PelerYuan/mouseshare?include_prereleases&sort=semver)](https://github.com/PelerYuan/mouseshare/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform: Linux/X11](https://img.shields.io/badge/platform-Linux%20%7C%20X11-lightgrey.svg)](docs/ROADMAP.zh-CN.md)

[English](README.md) · **简体中文**

</div>

mouseshare 让一台电脑的鼠标和键盘同时控制局域网内的多台电脑。把光标移出屏幕边缘，
它就出现在下一台电脑上；在一台电脑上复制文字，可以直接在另一台上粘贴。它是
Synergy / Barrier / Input Leap 的小巧、快速、加密的替代品，配有现代化的图形界面，
从安装到可以使用大约只要两分钟。

## 亮点

- **任意数量的电脑**，每台可以有任意数量的显示器，把屏幕拖成和桌面上一样的摆放方式。
- **用短配对码配对。** 被控电脑显示形如 `XXXXX-XXXXX` 的配对码，在控制端输入一次即可。
  连接经过认证（SPAKE2）和端到端加密（ChaCha20-Poly1305），无需管理证书，
  配对码也无法被离线暴力破解。
- **该有的都有：** 鼠标移动、按键、滚轮、键盘，以及纯文本剪贴板同步。
- **稳定可靠。** 断线自动重连（退避重试）、每台设备实时显示延迟、光标卡住时可用紧急返回快捷键
  （`Ctrl+Alt+Esc`），拖拽时自动忽略屏幕边缘。
- **现代化界面：** 深色 / 浅色 / 自动主题，六种强调色，可调界面大小，支持英文和简体中文，
  带首次使用引导和消息提示。命令行工具 `mouseshare` 与图形界面共用同一份设置，方便无界面使用。
- **轻量。** 纯 Rust 实现，无后台服务；命令行程序只有约 1.5 MB。

## 安装

在 [Releases 页面](https://github.com/PelerYuan/mouseshare/releases) 下载最新的 `.deb` 或压缩包。

```bash
# Debian / Ubuntu
sudo apt install ./mouseshare_<版本>_amd64.deb

# 任意 Linux：安装到当前用户的 ~/.local
tar xzf mouseshare-<版本>-x86_64-linux.tar.gz
cd mouseshare-<版本>-x86_64-linux && ./install.sh
```

也可以从源码构建（Rust 1.88+）：

```bash
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --release          # 生成 target/release/mouseshare 和 mouseshare-gui
```

## 快速上手

1. **在每台电脑上**打开 *mouseshare*。
2. **在被控电脑上**选择 **被控端**，点击 **开始共享**，屏幕上会出现配对码。
3. **在拥有鼠标键盘的电脑上**选择 **控制端**，点击 **添加设备**，选择另一台电脑
   （会自动发现），输入配对码。
4. 把屏幕拖成与桌面上一致的位置，然后点击 **开始共享**。
5. 把光标推出屏幕边缘即可。🎉

光标如果卡在另一台电脑上，按 **Ctrl+Alt+Esc** 即可回到本机。

喜欢命令行？

```bash
mouseshare target                      # 显示配对码
mouseshare controller --connect desk=192.168.1.20:7878 --pair-code desk=7F3KD-9X2MQ
```

完整教程、所有设置和命令行参考见 [用户指南](docs/USER_GUIDE.zh-CN.md)。

## 文档

| | |
|---|---|
| [用户指南](docs/USER_GUIDE.zh-CN.md) | 安装设置、日常使用、各项设置、命令行 |
| [故障排查与常见问题](docs/TROUBLESHOOTING.zh-CN.md) | 遇到问题时看这里 |
| [架构说明](docs/ARCHITECTURE.zh-CN.md) | 代码如何组织 |
| [协议与安全设计](docs/PROTOCOL.zh-CN.md) | 数据格式、握手、密码学 |
| [路线图](docs/ROADMAP.zh-CN.md) | 已完成、计划中，以及如何参与 |
| [安全策略](SECURITY.zh-CN.md) | 如何报告安全漏洞 |
| [更新日志](CHANGELOG.zh-CN.md) | 版本历史 |
| [贡献指南](CONTRIBUTING.zh-CN.md) | 构建、测试、提交 Pull Request |

每份文档都有对应的英文版本。

## 运行要求

- 每台电脑都需要运行 **X11** 会话（Wayland 在[路线图](docs/ROADMAP.zh-CN.md)中；
  在 Wayland 桌面上，通常可以在登录界面选择 “X11 / Xorg” 会话）。
- 电脑之间需要能通过 TCP（默认端口 `7878`）互相访问；若网络允许 mDNS（多播 DNS），
  可自动发现设备，否则手动输入地址即可。

## 项目状态

mouseshare 还很年轻，但在目标范围内功能完整：端到端测试会驱动真实的 X 服务器，覆盖每项功能
（移动、点击、滚轮、按键、剪贴板、错误配对码被拒、断线重连、多显示器跨屏）。目前仅支持 Linux/X11，
后续计划见[路线图](docs/ROADMAP.zh-CN.md)。

## 参与贡献

欢迎各种形式的贡献：问题反馈、翻译、文档和代码。请先阅读 [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md)，
并查看标记为
[`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
或 [`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted) 的议题。
另请阅读[行为准则](CODE_OF_CONDUCT.zh-CN.md)。

## 许可证

[MIT](LICENSE)
