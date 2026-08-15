# mkd — 超轻量级 macOS Markdown 阅读器

基于 **Rust + GPUI** 的原生 Markdown 阅读器，release 二进制仅 **2.9 MB**（远小于 20 MB 目标）。

![mkd 界面截图](docs/screenshot.png)

## 特性

- macOS 原生窗口（Metal 渲染，Retina 适配）
- CommonMark + GFM 语法：1–6 级标题、粗体 / 斜体 / 删除线、行内代码、链接、有序 / 无序 / 嵌套列表、代码块（带语言标签）、引用块、分隔线、表格
- 中文混排友好，正文自动换行，代码块水平滚动
- 快捷键：`Cmd+Q` 退出，`Cmd+R` 重新加载当前文件
- 支持 Finder 打开：双击 .md 文件或用 `open -a mkd 文件.md`（冷启动 / 热启动均可）

## 构建

```sh
cargo build --release        # 产出 target/release/mkd (2.9 MB)
./make-app.sh                # 打包为 dist/mkd.app
```

## 使用

```sh
./target/release/mkd 文件.md      # 命令行直接打开
open -a dist/mkd.app 文件.md      # 通过 Finder / Launch Services 打开
```

## 技术栈

| 组件 | 说明 |
| ---- | ---- |
| [gpui](https://crates.io/crates/gpui) 0.2 | Zed 的 GPU UI 框架，macOS 原生窗口 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13 | CommonMark / GFM 解析器 |
| 渲染 | `StyledText` + `HighlightStyle` 内联富文本，`div` 布局 |

## 体积优化

`Cargo.toml` 的 release profile 使用 `lto = "thin"`、`codegen-units = 1`、`opt-level = "z"`、`strip = true`、`panic = "abort"`。
