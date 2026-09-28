# mouseshare

[![CI](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml/badge.svg)](https://github.com/PelerYuan/mouseshare/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md)

一款局域网鼠标键盘共享工具，设计思路类似 Synergy / Barrier / Input
Leap：其中一台机器拥有物理鼠标和键盘，当光标移动到某块屏幕预先配置好的边缘时，
控制权就会通过网络交给相邻的那台机器。

## 现状

MVP 已经端到端跑通：两台机器之间的鼠标 + 键盘共享、剪切板同步，目前仅支持
Linux X11，支持通过 mDNS 在局域网内自动发现对方（手动填 IP 依然可用），暂无加密。
已经在两台真实的无头（headless）X server 上验证过（见 `tests/e2e.rs`）——边缘检测、
捕获、网络转发、注入、键盘转发、剪切板同步、mDNS 发现，以及交还控制权给本机，这些
环节是作为一个整体串联验证的，而不只是各自独立测试。项目最初的需求分析里有完整的
分阶段路线图（身份验证/加密、多屏幕 DPI 处理、Wayland 支持、文件传输)——这些都是
第一阶段有意排除在范围之外的。

## 手动运行

在目标机器上（接收转发过来的鼠标和键盘输入的那台——复制一份
`layout.example.toml`，把 `local_id` 改成这台机器的 screen id）。它启动后也会开始
通过 mDNS 广播自己，方便局域网内的控制端自动发现它：

```bash
mouseshare --config layout.toml target --listen 0.0.0.0:7878
```

在控制端机器上（拥有物理鼠标/键盘的那台；用同一份布局文件，`local_id` 设成它自己
的 screen id），可以让它通过 mDNS 自动寻找目标机器：

```bash
mouseshare --config layout.toml controller
```

也可以直接连接一个已知地址，跳过自动发现过程（省略 `--connect` 时，
`--discover-timeout-secs` 用来控制 mDNS 查找的等待时长，默认 3 秒）：

```bash
mouseshare --config layout.toml controller --connect <目标机器IP>:7878
```

把鼠标移动到控制端屏幕上、与布局配置中目标屏幕相邻的那条边，控制权（鼠标和键盘）
就会交接过去。

## 剪切板

剪切板文本会自动双向同步，与当前谁持有鼠标/键盘控制权无关——在任意一台机器上复制
的内容，几乎瞬间就能在另一台机器上粘贴。不需要任何额外配置或参数，只要控制端/目标
端建立连接就会自动开始同步。目前只支持纯文本（不支持图片、文件等其他格式），通过
X11 的 `CLIPBOARD` selection 实现——具体的 ICCCM 机制见
`crates/x11input/src/clipboard.rs`。

## 图形界面

`mouseshare-gui` 是一个可选的、单独编译的二进制程序，给不想手动编辑 TOML 文件的人
使用：它会自动探测本机屏幕分辨率和主机名，可以选择控制端/目标端角色，通过拖拽把
远端屏幕贴靠到本机屏幕的任意一条边上（拖拽会始终贴合对齐，以满足 layout crate
"边缘必须相接" 的约束），可以通过 mDNS 扫描局域网内的其他 mouseshare 实例并点击
填入，还有一个实时日志面板。它构建在与 CLI 相同的 `mouseshare-core` 运行时之上，
只是换了一个前端界面。

```bash
cargo run --release -p mouseshare-gui
```

这是项目"默认保持轻量"原则的一个刻意例外：引入 `egui`/`eframe` 会带来实打实的体积
开销（release 版、strip 后约 5M，而 CLI 只有约 1.4M），换来的是不用手动编辑配置
文件的便利。CLI 二进制完全不受影响——它是独立的 crate/二进制程序，完全不依赖
GUI 这个 crate。

## 项目结构

工作区（workspace）内的 crate：

- `crates/protocol` —— 线上消息类型（`Message` 枚举：`Hello`、`HelloAck`、
  `MouseMove{dx,dy}`、`KeyEvent{keycode,pressed}`、`ClipboardText(String)`、
  `Heartbeat`）以及带长度前缀的帧编解码（`encode_frame`/`decode_body`）。纯逻辑，
  不涉及 I/O。
- `crates/layout` —— 屏幕几何信息（`ScreenRect`、从 TOML 加载的 `LayoutConfig`）
  以及 `EdgeDetector` 状态机，它在整个已配置的虚拟桌面范围内追踪一个虚拟光标位置，
  并决定何时应该在各屏幕之间交接控制权。纯逻辑，不涉及 I/O——只有控制端机器会运行
  这部分逻辑，因为转发给对端的每一个位移增量本来就是从本地产生的，对端不需要把
  位置回报回来。
- `crates/net` —— 基于 protocol crate 帧格式构建的 tokio TCP 传输层，加上
  `Hello`/`HelloAck` 握手，以及 `Connection::into_split`（拆分出独立的读/写两半，
  这样一个任务可以专门跑一个单纯的 `recv()` 循环，另一个任务负责发送，从而避免在
  可取消的 `select!` 分支里对 `recv()` 进行竞速——细节见 `recv_from` 上关于
  取消安全性的说明）。
- `crates/x11input` —— X11 指针/键盘捕获（用"抓取 + 重新居中"技术提取原始的移动
  增量，同时不让真实光标移动）、注入（相对/绝对 `WarpPointer`，按键用 XTest 的
  `fake_input`），以及剪切板同步（`Clipboard`：ICCCM `CLIPBOARD` selection 的
  所有权管理、所有权变化轮询、`SelectionRequest` 响应）——全部基于 `x11rb` 实现。
- `crates/discovery` —— 基于 mDNS/DNS-SD（`mdns-sd`）的局域网自动发现：
  `Announcement::start` 用来广播目标端的 `screen_id`/端口，`discover` 用来
  搜索目标端。
- `crates/core` —— 真正的控制端/目标端运行时循环（`run_controller`、`run_target`、
  `resolve_target_addr`），被所有前端共用，这样就只需要维护一份实现并保证它被
  充分测试。可观测性交给 `tracing` 处理，而不是自定义一套回调 API，这样每个前端
  都可以自行安装适合自己的 subscriber。
- `crates/gui` —— 可选的 `mouseshare-gui` 二进制程序（见上文"图形界面"一节）：
  `eframe`/`egui` 界面、一个拖拽贴靠的屏幕排布控件，以及一个把日志喂给实时日志
  面板的 `tracing_subscriber` writer。

`KeyEvent` 里的 keycode 是未经映射的原始 X11 keycode：只要两端都是 X11（目前唯一
存在的后端）就没问题，但还谈不上真正的跨平台按键转换——详见
`crates/protocol/src/lib.rs` 中该枚举成员的文档注释。

`src/main.rs` 是 `mouseshare-core` 的一个薄薄的 CLI 包装层（负责参数解析、加载
布局文件、选择控制端/目标端角色）；`crates/gui` 的 `main.rs` 是构建在同一个 core
之上的另一个前端。

## 配置

参见 `layout.example.toml`——复制一份，按每台机器调整 `local_id`/坐标。屏幕矩形
共享同一个虚拟桌面坐标空间；边缘必须严格相接，交接才会感觉顺滑（如果留有空隙，
光标会卡在墙边而不是穿过去）。

### 混合分辨率与 DPI

布局里的屏幕宽/高是原始像素数（使用 GUI 时通过 XRandR 自动探测），和真实操作系统
自带的多屏幕设置采用的坐标模型是一样的——而不是物理尺寸或 DPI。两块分辨率或物理
DPI 不同的屏幕，处理方式和现实中两块不匹配的显示器完全一样：在配置文件里（或者
在 GUI 里拖拽，GUI 允许任意方向的垂直偏移，不仅仅是角对齐或居中对齐）把它们摆放到
你想要的交接边缘对齐方式——比如把一块 4K 屏幕的左边缘和一块 1080p 屏幕的右边缘
顶部对齐，如果你想让光标落在那个位置的话，或者按你实际桌面摆放情况把它们偏移到
合适的高度对齐。交接点不会自动按比例缩放，这一点同样是在模仿真实的多屏幕排布方式
（X11/Windows/macOS 都是按像素偏移来摆放输出设备的，而不是互相按比例缩放）——如果
你希望从 1080p 屏幕 50% 高度处穿过去、落在旁边 4K 屏幕 50% 高度的位置，那就把两块
屏幕垂直居中对齐，而不是顶部对齐。

## 运行测试

```bash
cargo test --workspace
```

`x11input` crate 的测试，以及根目录下的 `tests/e2e.rs`（会把编译好的真实二进制
程序分别当作控制端和目标端、跑在两个临时的 Xvfb 实例上）需要安装 `Xvfb`、
`xdotool` 和 `xmodmap`，但不需要真实的显示器。剪切板相关测试
（`crates/x11input/tests/clipboard.rs` 以及 `e2e.rs` 里的
`clipboard_syncs_bidirectionally_between_two_real_processes`）还额外需要
`xclip`，作为 ICCCM 交互另一端的真实独立 X 客户端。`discovery` crate 的测试
以及 `e2e.rs` 里的 mDNS 发现测试，还额外需要运行环境本身允许在某个网络接口
（包括回环接口）上进行 IP 组播——详见
`crates/discovery/tests/announce_and_discover.rs`。

## 参与贡献

开发环境搭建、项目的"保持轻量"约束，以及提交 PR 前需要检查的事项，见
`CONTRIBUTING.md`。报 bug 和提功能需求请使用 `.github/ISSUE_TEMPLATE/` 下的
模板。

## 许可证

MIT —— 见 `LICENSE`。
