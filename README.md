# mkd — 超轻量级 macOS Markdown 阅读器 & 编辑器

基于 **Rust + GPUI** 的原生 Markdown 阅读器与所见即所得编辑器，release 二进制仅 **3.5 MB**（远小于 20 MB 目标）。

![mkd 编辑模式截图](docs/screenshot.png)

## 特性

- macOS 原生窗口（Metal 渲染，Retina 适配），中文混排友好
- **所见即所得编辑**（`Cmd+E` 切换）：直接在渲染视图上编辑，粗体 / 斜体 / 标题 / 列表 / 代码块即时显示
- 编辑时实时行级解析：`**粗体**`、`*斜体*`、`***粗斜体***`、`` `行内代码` ``、`~~删除线~~`、`==高亮==`、`[链接](url)` 直接显示渲染效果
- `Cmd+S` 保存，未保存时显示 `●` 标记
- `Cmd+Q` 退出，`Cmd+R` 重新加载（预览模式）
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
./target/release/mkd 文件.md            # 命令行直接打开（预览）
./target/release/mkd --edit 文件.md     # 直接进入编辑模式
open -a dist/mkd.app 文件.md            # 通过 Finder / Launch Services 打开
```

## 快捷键

| 快捷键 | 功能 |
| ---- | ---- |
| `Cmd+E` | 编辑 / 预览切换 |
| `Cmd+S` | 保存（编辑模式） |
| `Cmd+Q` | 退出 |
| `Cmd+R` | 重新加载文件（预览模式） |
| 方向键 / `Shift`+方向键 | 移动光标 / 扩展选区 |
| `Cmd+A` / `Cmd+C` / `Cmd+X` / `Cmd+V` | 全选 / 复制 / 剪切 / 粘贴 |

## 技术栈

| 组件 | 说明 |
| ---- | ---- |
| [gpui](https://crates.io/crates/gpui) 0.2 | Zed 的 GPU UI 框架，macOS 原生窗口 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13 | CommonMark / GFM / 扩展解析 |
| [emojis](https://crates.io/crates/emojis) 0.9 | Emoji 短代码查询 |
| 编辑器 | 自写行内解析器（渲染文本 ↔ 源码映射）+ `EntityInputHandler`（IME 支持） |

## 体积优化

`Cargo.toml` 的 release profile 使用 `lto = "thin"`、`codegen-units = 1`、`opt-level = "z"`、`strip = true`、`panic = "abort"`。
