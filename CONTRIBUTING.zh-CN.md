# 为 mouseshare 做贡献

[English](CONTRIBUTING.md)

感谢你考虑参与贡献！问题反馈、翻译、文档修正和代码都很有价值，小的贡献同样欢迎。
参与即表示你同意遵守[行为准则](CODE_OF_CONDUCT.zh-CN.md)。

## 参与方式

- 使用 [Issue 表单](https://github.com/PelerYuan/mouseshare/issues/new/choose)**报告 bug** 或**提出功能建议**，
  请先搜索已有 issue。
- 在 [Discussions](https://github.com/PelerYuan/mouseshare/discussions) **提问**（见 [SUPPORT.zh-CN.md](SUPPORT.zh-CN.md)）。
- **改进文档或翻译。** 每份文档都有英文原文和对应的 `*.zh-CN.md`。界面文字在 `crates/gui/src/strings.rs`。
- **认领 issue**：标有
  [`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
  或 [`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted)
  的条目，并留言说明你在处理。
- **报告安全问题**请私下进行，见 [SECURITY.zh-CN.md](SECURITY.zh-CN.md)。

对于比小修复更大的改动，请先创建 issue，在你投入时间之前先就方案达成一致。

## 开发环境

需要较新的稳定版 Rust（1.88+），测试还需要一些 X11 工具：

```bash
sudo apt-get install xvfb xdotool x11-xserver-utils xclip   # Debian/Ubuntu
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --workspace
cargo test --workspace
```

多数测试针对临时的 `Xvfb` 显示测试真实的 X11 行为，而不是使用模拟。每个测试文件开头注明了需要哪些工具。
mDNS 测试还要求某个网络接口（包括回环）允许 IP 多播。

从源码运行图形界面：`cargo run -p mouseshare-gui`；运行命令行：`cargo run -p mouseshare -- --help`。

本地构建发布包：`packaging/build.sh`（输出到 `dist/`）。

## 项目结构

见 [docs/ARCHITECTURE.zh-CN.md](docs/ARCHITECTURE.zh-CN.md)。简而言之：`protocol` 和 `layout` 是纯逻辑，
`net` 是安全传输，`x11input` 是唯一与 X11 打交道的 crate，`core` 是命令行和图形界面共用的运行时，
`config` 负责持久化设置，`gui` 是 egui 应用。

## Pull Request 流程

1. **Fork** 仓库（有写权限则直接建分支），从 `master` 创建分支：`git switch -c fix/简短描述`。
2. 保持改动聚焦：每个 Pull Request 只做一件事。
3. 推送之前运行：
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
4. 为你改变的行为添加或更新**测试**；对用户可见的改动，更新**文档**（英文*和*对应的 `.zh-CN.md`；
   如果你不会写中文，请在 PR 中说明，我们会帮忙）以及 `CHANGELOG.md` 的 `[Unreleased]` 部分。
5. 使用模板创建 Pull Request，说明*做了什么*和*为什么*，并关联 issue（`Fixes #123`）。
6. CI 必须通过，并且需要维护者批准。评审意见是一场对话而不是裁决，请向同一分支推送后续提交。
7. Pull Request 采用**压缩合并（squash）**，所以分支上的提交历史不必整洁，但 PR 标题要清晰。

`master` 分支受保护：改动只能通过检查全部通过的 Pull Request 合入。

### 提交与 PR 标题

使用简短的祈使句概述，可加上模块前缀：`gui: fix canvas snapping across monitors`、
`net: reject oversized handshake`、`docs: clarify firewall setup`。

### 代码风格

- `cargo fmt`，且 clippy 无警告（`-D warnings`）。
- 与周围代码保持一致。注释解释*为什么*而不是*做什么*；代码自明时不写注释。
- 保持**轻量**。添加依赖前，先确认标准库或现有依赖能否完成；尽量关闭默认特性。
  为了方便而引入大依赖的 PR 会被要求寻找更轻的方案。
- 除非有充分理由并加以注释，否则不使用 `unsafe`。
- 用户可见的字符串必须通过 `strings.rs` 并同时提供英文和中文（缺少键时单元测试会失败）。

## 添加翻译

1. 复制 `crates/gui/src/strings.rs` 中的英文字符串，在 `Lang`/`Language` 中加入你的语言代码，翻译每一条。
2. 把文档翻译为与原文并列的 `*.<语言>.md`，并在每个文件顶部的语言切换行中加上链接。
3. 创建 PR：欢迎以草稿形式提交部分翻译。

## 平台范围

目前只实现了 Linux/X11。欢迎支持 Wayland、Windows 和 macOS，但工作量较大；
请参阅[路线图](docs/ROADMAP.zh-CN.md)，并在投入大量时间前先创建 issue 讨论方案。
第二个后端的接缝是 `x11input` crate，`core` 目前通过具体类型使用它；把它抽成 trait 本身就是很好的第一个 PR。

## 维护者：发布流程

1. 把 `[Unreleased]` 的更新日志条目（两种语言）移到新的版本标题下，并提升根 `Cargo.toml` 和每个
   `crates/*/Cargo.toml` 中的 `version`。
2. 通过 Pull Request 合入。
3. 给合并提交打标签 `vX.Y.Z` 并推送。*Release* 工作流会构建压缩包和 `.deb`、生成 `SHA256SUMS`，
   并使用更新日志对应章节发布 GitHub Release。
