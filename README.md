# mkd — 超轻量级 macOS Markdown 阅读器

基于 **Rust + GPUI** 的原生 Markdown 阅读器，release 二进制仅 **3.5 MB**（远小于 20 MB 目标）。

![mkd 界面截图](docs/screenshot.png)

## 特性

- macOS 原生窗口（Metal 渲染，Retina 适配），中文混排友好
- 快捷键：`Cmd+Q` 退出，`Cmd+R` 重新加载当前文件
- 支持 Finder 打开：双击 .md 文件或用 `open -a mkd 文件.md`（冷启动 / 热启动均可）

## VitePress Markdown 支持

完整覆盖 VitePress 文档常用 Markdown 特性：

| 类别 | 特性 |
| ---- | ---- |
| GFM | 表格、删除线、任务列表、自动链接、脚注 |
| 文本 | `==高亮标记==`、`:tada:` Emoji 短代码、上标 `^x^` / 下标 `~x~`、粗体 / 斜体 / 行内代码 |
| 数学 | 行内 `$...$` 与块级 `$$...$$` |
| Frontmatter | `title` / `description` 渲染为文档头部 |
| 目录 | `[[toc]]` 自动生成目录 |
| 自定义容器 | `::: tip / warning / danger / info / note / details`（支持标题），`::: code-group` |
| 代码块 | 行高亮 `{1,3-5}`、行号 `:line-numbers`、标题 `title="..."`、语言标签 |
| 导入 | `<<< @/snippets/file.js` 代码片段导入（含行范围选择）、`<!--@include: -->` 文档包含 |
| 其他 | 定义列表、HTML 块（按纯文本展示）、标题锚点属性 `{#id}` |

## 构建

```sh
cargo build --release        # 产出 target/release/mkd (3.5 MB)
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
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13 | CommonMark / GFM / 扩展解析 |
| [emojis](https://crates.io/crates/emojis) 0.9 | Emoji 短代码查询 |
| 渲染 | `StyledText` + `HighlightStyle` 内联富文本，`div` 布局 |

## 体积优化

`Cargo.toml` 的 release profile 使用 `lto = "thin"`、`codegen-units = 1`、`opt-level = "z"`、`strip = true`、`panic = "abort"`。
