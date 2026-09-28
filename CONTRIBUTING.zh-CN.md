# 参与贡献

[English](CONTRIBUTING.md)

感谢你愿意为 mouseshare 做贡献。

## 开始之前

这个项目有一条贯穿大部分设计决策的硬性约束：**必须保持轻量**，无论是二进制体积
还是运行时内存占用。在添加一个依赖之前，先想想标准库或者项目里已有的 crate
（`x11rb`、精简 feature 集的 `tokio` 等）能不能搞定。如果一个 PR 为了图方便引入了
较重的依赖，很可能会被要求先寻找更轻量的方案——已经做过的体积/依赖取舍可以参考
根目录 `Cargo.toml` 里的 `[profile.release]` 部分，以及 `README.md`
"项目结构" 一节里逐个 crate 的说明。

## 开发环境搭建

```bash
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --workspace
```

跑测试套件需要几个系统工具，因为这个项目里的大多数测试是针对一个临时的 `Xvfb`
显示器验证真实的 X11 行为，而不是靠 mock：

```bash
sudo apt-get install xvfb xdotool xmodmap xclip   # Debian/Ubuntu
cargo test --workspace
```

具体哪个测试文件需要哪个工具、为什么需要，见 `README.md` 的"运行测试"一节
（mDNS 发现相关的测试还额外需要某个网络接口，包括回环接口，允许 IP 组播）。

提交 PR 之前：

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

这三步在每个 PR 上都会跑 CI；CI 报红会阻止合并。

## 改动代码时

- PR 尽量聚焦——一个 PR 只做一件事，比把一堆不相关的修复揉在一起更容易被审查。
- 保持和现有代码风格一致：文档注释解释的是"为什么"，而不是"做了什么"；
  如果代码本身已经足够清楚，就不需要写注释。
- 为你改动的行为补充或更新测试。这个代码库里的大部分测试都是针对真实的 X11
  客户端/服务端跑的，而不是 mock（见 `tests/e2e.rs` 和
  `crates/x11input/tests/`）——请遵循这个模式，不要为了 mock 而引入 mock。
- 如果你改动了用户可见的行为、新增了 crate，或者改动了某个依赖的 feature
  flag，请同步更新 `README.md`。

## 报 bug / 提功能需求

请使用 issue 模板——它们只会问一些行动所必需的最少信息（操作系统/桌面环境、
mouseshare 版本、复现步骤）。

## 平台支持范围

目前只实现了 Linux/X11（见 `README.md` 的"现状"一节）。欢迎为 Wayland 和
Windows/macOS 提供支持，但这是相当大的工作量——在投入大量时间之前，建议先开一个
issue 讨论方案。目前 `mouseshare-core` 是直接调用 `mouseshare-x11input` 里的
具体类型的；要支持第二个平台，得先把这层边界改造成一个 trait，而这本身就值得
单独作为一个 PR 来讨论，再在此基础上叠加具体平台的实现。
