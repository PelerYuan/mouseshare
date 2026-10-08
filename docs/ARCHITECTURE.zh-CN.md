# 架构说明

[English](ARCHITECTURE.md)

mouseshare 是一个由多个小型 crate 组成的 Cargo 工作区，依赖关系是单向的。纯逻辑（protocol、layout）
与 I/O（net、x11input）分开，因此无需显示器或网络就能测试。

```text
                    ┌─────────┐   ┌────────────┐
   mouseshare (CLI) │  gui    │   │  tests/e2e │
                    └────┬────┘   └─────┬──────┘
                         │              │ 驱动真实二进制
                    ┌────▼──────────────▼───┐
                    │ config  （设置、配对码、自启动）
                    └────┬──────────────────┘
                    ┌────▼────┐
                    │  core   │  控制端 / 被控端运行时、遥测
                    └─┬──┬──┬─┘
        ┌─────────────┘  │  └───────────────┐
   ┌────▼────┐     ┌─────▼─────┐      ┌─────▼─────┐   ┌───────────┐
   │ layout  │     │    net    │      │ x11input  │   │ discovery │
   │ （纯逻辑）│     │ SPAKE2+AEAD      │ 捕获/注入/ │   │   mDNS    │
   └─────────┘     └─────┬─────┘      │ 剪贴板     │   └───────────┘
                         │            └───────────┘
                    ┌────▼────┐
                    │protocol │ （纯逻辑）线上消息与分帧
                    └─────────┘
```

## 各 crate

| Crate | 职责 | 说明 |
|---|---|---|
| `protocol` | `Message` 枚举、`MonitorInfo`、带长度前缀的分帧 | 纯逻辑；bincode + serde。 |
| `layout` | `Device`/`Monitor` 模型、`LayoutConfig`（TOML）、`EdgeDetector` | 纯逻辑。在设备所有显示器的并集上检测边缘；入口点；拖拽保护。 |
| `net` | TCP 传输、SPAKE2 密钥交换、ChaCha20-Poly1305 记录层、`PairingCode` | tokio。配对码错误 → `AuthFailed`。 |
| `x11input` | RandR 显示器列表、捕获（抓取 + 回中）、XTest 注入、ICCCM 剪贴板 | 仅依赖 `x11rb`；唯一了解 X11 的地方。 |
| `discovery` | mDNS 广播 / 浏览 | `mdns-sd`。 |
| `core` | 运行时：`run_controller`、`run_target`、`Telemetry`、快捷键 | CLI 与 GUI 共用同一份实现。 |
| `config` | `Settings`、`PairingStore`、自启动项 | `~/.config/mouseshare/` 下的纯 TOML；配对文件权限为 `0600`。 |
| `gui`（`mouseshare-gui`） | egui/eframe 应用 | 主题、多语言、引导、画布、设置。 |
| 根（`mouseshare`） | clap 命令行 | 对 `core` 和 `config` 的薄封装。 |

## 控制端运行时

```text
 本地输入 ──► 输入循环（8 毫秒节拍）──► EdgeDetector ──► 当前活动设备？
                    ▲        │                                  │
   监督任务 ◄───────┘        └─ MouseMove/Button/Scroll/Key ────┤
   （每个被控端一个）            通过活动对端的通道发送           ▼
   - 解析地址                                              远程设备
   - 拨号 + 握手
   - ping / 延迟
   - 重连（1 秒→15 秒）
```

- **每个被控端一个监督任务**，负责连接的生命周期，并通过通道把它暴露给输入循环，
  所以某个缓慢或失效的对端不会拖住其他设备的输入处理。
- **输入循环**轮询本地指针。控制权在本地时，把位置交给 `EdgeDetector`；在远程时，消费捕获到的位移，
  应用每台设备的指针速度，转发事件，并跟踪虚拟光标以检测返回路径（或穿越到第三台设备）。
- 对端丢失或触发紧急快捷键时，远程上按住的按键会被释放，光标回到本机设备的**中心**，
  这样不会在边缘立即再次触发切换。
- `Telemetry` 是共享快照（对端状态、延迟、显示器、光标在谁那里）加一个通知队列。前端轮询它，
  `core` 从不调用 UI。

## 被控端运行时

`run_target` 绑定端口，通过 mDNS 广播自己，同一时间接受一个已认证的控制端。错误的配对码按来源 IP 计数
（每分钟 5 次）。收到的事件用 XTest 注入；绝对定位使用 `WarpPointer`；被控端在连接时以及显示器变化时上报显示器。

## 图形界面

```text
App（状态 + 每帧 update）
 ├─ theme      设计令牌、浅色/深色调色板、egui 样式
 ├─ widgets    按钮、卡片、开关、分段控件、状态胶囊
 ├─ canvas     设备组排列视图（平移/缩放/吸附/对齐）
 ├─ rail       角色切换、设备列表 + 详情编辑、开始/停止
 ├─ target_view  被控电脑的配对码页面
 ├─ dialogs    模态框架、添加设备、确认移除
 ├─ settings_view / onboarding
 ├─ i18n + strings   中英文表、键覆盖测试
 ├─ fonts      系统 UI 字体、Phosphor 图标、运行时 CJK 字体
 └─ session / scan   后台 tokio 运行时、mDNS 扫描线程
```

界面是即时模式：状态保存在 `App` 和 `Settings` 里；`Settings` 一有变化就会自动保存（带防抖）。
网络工作运行在后台 tokio 运行时上，因此 UI 线程从不阻塞。

## 设计决策

- **为什么暂时只支持 X11。** 全局捕获和注入就是整个产品的核心。X11 两者都允许；Wayland 需要特定合成器的协议。
  所有 X11 代码都在 `x11input` 后面，第二个后端将从这里接入（见[路线图](ROADMAP.zh-CN.md)）。
- **为什么用 SPAKE2 而不是 TLS。** 手输的配对码熵很低；PAKE 让它变得安全，也避免了证书管理。
  见[协议与安全设计](PROTOCOL.zh-CN.md#安全设计)。
- **为什么显示器是刚性的整体。** 用户想的是“电脑”而不是“显示器”；移动整台电脑可保持内部几何正确，
  也让穿越边缘的行为可预期。
- **为什么两个前端共用 `core`。** 行为只实现并测试一次；CLI 和 GUI 的区别只在于如何收集设置和展示状态。
- **为什么端到端测试驱动真实 X 服务器。** 模拟的输入抓不到真正重要的 bug（抓取顺序、XTest 的怪癖、剪贴板所有权）。

## 测试

| 层级 | 方式 |
|---|---|
| `protocol`、`layout`、`config`、`net` | 普通单元测试（无需显示器）。 |
| `x11input` | 在临时 `Xvfb` 显示上，用 `xdotool` / `xclip` 作为独立客户端。 |
| `gui` | i18n 键覆盖测试，加人工视觉检查。 |
| 端到端（`tests/e2e.rs`） | 在两个 Xvfb 显示上分别以控制端和被控端启动编译好的 `mouseshare`：切换、按键、滚轮、键盘、剪贴板、mDNS 发现、错误配对码被拒、重连、返回快捷键、多显示器跨屏。 |

`cargo test --workspace` 会运行全部测试，需要 `xvfb xdotool x11-xserver-utils xclip`。
