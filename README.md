# mkd — 超轻量级 macOS Markdown 阅读器 & 编辑器

基于 **Rust + GPUI** 的原生 Markdown 阅读器与所见即所得编辑器，release 二进制仅 **3.5 MB**（远小于 20 MB 目标）。

![mkd 编辑模式截图](docs/screenshot.png)

## 特性

- macOS 原生窗口（Metal 渲染，Retina 适配），中文混排友好
- **所见即所得编辑**（`Cmd+E` 切换）：直接在渲染视图上编辑，粗体 / 斜体 / 标题 / 列表 / 代码块即时显示
- 行内实时解析：`**粗体**`、`*斜体*`、`***粗斜体***`、`` `行内代码` ``、`~~删除线~~`、`==高亮==`、`~下标~`、`^上标^`、`[链接](url)` 直接显示渲染效果
- **撤销/重做**：连续键入合并为一步、redo 栈、光标恢复
- **输入规则**：`- ` / `1. ` / `# ` / `> ` / ` ``` ` / `---` 自动识别为块结构，Backspace 可回退
- **列表行为**：回车续行、空项退出、Tab 缩进 / Shift+Tab 提升、有序序号自动递增
- **格式化命令**：Cmd+B/I/Code/Strike 包裹与去包裹、光标处输入态、Cmd+K 链接、块类型切换（Cmd+Alt+1-6/C/Q）、选区转列表
- **段落语义**：段落中回车自动空行分隔，Shift+Enter 硬换行
- **选区**：拖选、双击选词、三击选行、Shift 扩展
- **光标**：上下移动记忆列（stick column）、自动滚动跟随、闪烁
- **macOS 编辑键**：Ctrl+H/D/K/U/W、Alt+Backspace/Delete
- `Cmd+S` 保存（未保存 `●` 标记），`Cmd+Q` 未保存确认
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
./target/release/mkd --autosave 文件.md  # 编辑时每 30s 自动保存
open -a dist/mkd.app 文件.md            # 通过 Finder / Launch Services 打开
```

## 快捷键

| 快捷键 | 功能 |
| ---- | ---- |
| `Cmd+E` | 编辑 / 预览切换 |
| `Cmd+S` | 保存（编辑模式） |
| `Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Y` | 撤销 / 重做 |
| `Cmd+B` / `Cmd+I` / `Cmd+\`` / `Cmd+Shift+X` | 加粗 / 斜体 / 代码 / 删除线 |
| `Cmd+K` | 插入链接 |
| `Cmd+Alt+0-6` | 段落 / 标题 1-6 |
| `Cmd+Alt+C` / `Cmd+Alt+Q` | 代码块 / 引用 |
| `Cmd+Shift+7/8/9` | 任务 / 无序 / 有序列表 |
| `Cmd+F` / `Enter` / `Esc` | 查找 / 下一个匹配 / 关闭 |
| 从 Finder 拖入 .md 文件 | 打开该文件（编辑模式内） |
| `Option+←/→` | 按词移动光标 |
| 拖选文本后拖动 | 移动（`Option` 复制）选中文本 |
| `Cmd+Q` | 退出（未保存确认） |
| `Cmd+R` | 重新加载文件（预览模式） |
| `Enter` / `Shift+Enter` | 段落回车 / 硬换行 |
| `Tab` / `Shift+Tab` | 列表缩进 / 提升 |
| `Ctrl+H/D/K/U/W`、`Alt+Backspace/Delete` | macOS 编辑键 |
| 方向键 / `Shift`+方向键 | 移动光标 / 扩展选区（记忆列） |
| `Cmd+A` / `Cmd+C` / `Cmd+X` / `Cmd+V` | 全选 / 复制 / 剪切 / 粘贴 |
| 双击 / 三击 | 选词 / 选行 |

## 技术栈

| 组件 | 说明 |
| ---- | ---- |
| [gpui](https://crates.io/crates/gpui) 0.2 | Zed 的 GPU UI 框架，macOS 原生窗口 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13 | CommonMark / GFM / 扩展解析 |
| [emojis](https://crates.io/crates/emojis) 0.9 | Emoji 短代码查询 |
| 编辑器 | 自写行内解析器（渲染文本 ↔ 源码映射）+ `EntityInputHandler`（IME 支持） |
| 测试 | 106 个单元测试：Op 模型 / 撤销 / 输入规则 / 列表 / 格式化 / 选区 / 拖放 / 查找 / 渲染一致性对照 |

## 体积优化

`Cargo.toml` 的 release profile 使用 `lto = "thin"`、`codegen-units = 1`、`opt-level = "z"`、`strip = true`、`panic = "abort"`。
