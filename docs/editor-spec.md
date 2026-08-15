# mkd 所见即所得编辑器：行为规范与测试方案

> 调研来源：ProseMirror 官方文档与源码（commands / keymap / inputrules / history / schema-list / test-builder）、Lexical 官方测试文档、contenteditable 生态实践经验。
> 目标：为 mkd 的行级 markdown WYSIWYG 编辑器定义「标准编辑器应支持的行为清单」，以及可执行的分层测试方案。

## 0. 参考来源

| 来源 | 内容 | 用途 |
| ---- | ---- | ---- |
| [ProseMirror Reference](https://prosemirror.net/docs/ref/) | 文档模型、事务、选区、命令、插件、view 事件钩子 | 行为语义的权威定义 |
| [prosemirror-commands](https://prosemirror.net/docs/ref/#commands) | 29 个命令 + pc/mac baseKeymap | 键绑定行为基线 |
| [prosemirror-example-setup](https://github.com/ProseMirror/prosemirror-example-setup) | buildKeymap / buildInputRules 完整清单 | 输入规则与快捷键基线 |
| [prosemirror-history](https://prosemirror.net/docs/ref/#history) | undo/redo 事务合并与深度 | 撤销语义 |
| [prosemirror-schema-list](https://prosemirror.net/docs/ref/#schema-list) | wrapInList / splitListItem / liftListItem / sinkListItem | 列表行为 |
| [prosemirror-test-builder](https://github.com/ProseMirror/prosemirror-test-builder) | `doc(p("foo<a>"))` 位置标注测试 DSL | 测试基础设施设计 |
| [Lexical Testing](https://lexical.dev/docs/testing) | jsdom 单元 / 浏览器 / E2E 三层测试 | 测试分层方法论 |
| [ContentEditable 生态文章](https://medium.engineering/why-contenteditable-is-terrible-122d8a40e480) | contenteditable 的坑 | 为什么需要显式文档模型 |

## 1. 核心原则（来自 ProseMirror / Lexical 共识）

1. **显式文档模型**：编辑器内部必须有结构化模型（mkd：行级 markdown 源码 + 解析结果），不能依赖 DOM/contenteditable 引擎，否则行为不可控。
2. **所有变更走单一入口**（ProseMirror transaction / Lexical update）：可撤销、可记录、可测试。
3. **命令是可测试的纯函数**：`fn(state) -> (是否适用, 结果)`，dry-run 无副作用——这是单元测试的基础。
4. **输入与渲染分离**：键盘/IME/鼠标事件 → 语义操作；渲染只消费模型状态。
5. **IME/组合输入是一等公民**：中/日/韩输入必须走组合路径，不能被普通字符处理打断。

## 2. 行为清单（分模块）

状态标注：
- **[P0]** 必须（标准编辑器核心，缺了不算编辑器）
- **[P1]** 重要（常见编辑器都有）
- **[P2]** 增强（ProseMirror/Lexical 有，可后置）

### 2.1 光标与导航

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| NAV-01 | 左右移动按「字符/组合字符」边界，不切开多字节字符与 emoji | P0 | 现有 `prev/next_boundary` |
| NAV-02 | 上下移动保持目标列（x 坐标），行尾截断 | P0 | ProseMirror 行为；现有实现按字节列，应改为按「视觉列」 |
| NAV-03 | Home/End：行首/行尾（ProseMirror 为文本块首尾） | P0 | 现有 |
| NAV-04 | 跨行移动：行尾→下一行开头、行首→上一行尾 | P0 | 现有 |
| NAV-05 | 文档首/尾边界：不越界 | P0 | 现有 |
| NAV-06 | 光标点击定位：鼠标 → 最近字符边界（字符级，非字节级） | P0 | 现有 `pos_for_point`，需验证 emoji 边界 |
| NAV-07 | 单词级移动（Option+方向键，macOS） | P1 | ProseMirror 无内置，但系统编辑器都有 |
| NAV-08 | 双向文本（bidi）感知的行首/尾 | P2 | ProseMirror joinBackward 提及 bidi-aware |
| NAV-09 | 光标自动滚动进可视区（scrollIntoView） | P1 | ProseMirror `tr.scrollIntoView()` |

### 2.2 选区

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| SEL-01 | Shift+方向键扩展/收缩选区（含跨行） | P0 | 现有 |
| SEL-02 | 鼠标拖选（按下→移动→释放） | P0 | 现有 `on_mouse_down` 定位，**缺拖选** |
| SEL-03 | Cmd+A 全选 | P0 | 现有 |
| SEL-04 | 双击选词、三击选段 | P1 | Lexical/系统编辑器标准 |
| SEL-05 | 选区渲染（倒置方向、覆盖高亮） | P0 | 现有整块高亮，需按字符级边界精化 |
| SEL-06 | 选区非空时输入替换选区 | P0 | 现有 `delete_selection` |
| SEL-07 | 选区与光标互转：Esc 收起选区回到光标 | P1 | ProseMirror selectParentNode/Escape 语义 |

### 2.3 文本输入

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| INP-01 | 普通字符插入（ASCII、CJK、emoji） | P0 | 现有，需回归测试 |
| INP-02 | 插入发生在「渲染位置 ↔ 源码位置」映射正确处（如 `**粗体**` 中间插入不破坏标记） | P0 | 核心，需专门测试 |
| INP-03 | IME 组合输入：marked text 高亮/下划线、候选窗定位（bounds_for_range）、组合中编辑不打断 | P0 | 现有 EntityInputHandler，**缺组合中绘制与完整回归** |
| INP-04 | 组合输入跨行/跨选区 | P1 | 边界情况 |
| INP-05 | 回车拆分当前行（含列表/代码块内的特殊行为） | P0 | 现有；**列表续行未实现**（见 2.6） |
| INP-06 | Tab：代码块内插缩进；列表内升降级；否则插入空格/跳过 | P1 | 现有固定插 4 空格，**需按上下文** |
| INP-07 | 退格：删字符/删标记/合并行；选区优先 | P0 | 现有 |
| INP-08 | Delete：删字符/合并行；选区优先 | P0 | 现有 |
| INP-09 | 输入时脏标记（dirty）正确触发 | P0 | 现有 |

### 2.4 格式化与标记（markdown 特有）

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| FMT-01 | 在 `**加粗**` 内部输入，插入后标记仍闭合 | P0 | 核心不变量 |
| FMT-02 | 光标紧邻标记符（`**` 前/后）时的插入归属明确 | P0 | 需定义：光标在 `**` 后 = 标记内 |
| FMT-03 | 退格删到标记边界：删整个标记符（`**` 两字符一起删） | P1 | ProseMirror 无此概念（富文本无标记符），markdown 编辑器需要 |
| FMT-04 | 格式化命令：Cmd+B/I 切换加粗/斜体（对选区应用/移除，光标处切换输入态） | P1 | ProseMirror toggleMark + storedMarks |
| FMT-05 | 格式化后的光标状态（typed marks）：输入延续当前标记 | P1 | ProseMirror storedMarks |
| FMT-06 | 链接的编辑：光标在链接内输入不破坏 `[text](url)` | P1 | 与 FMT-01 同族 |

### 2.5 输入规则（自动格式化）

ProseMirror buildInputRules 的行为，markdown 编辑器天然需要：

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| IR-01 | `- ` / `* ` / `+ ` → 无序列表 | P0 | markdown 编辑器核心 |
| IR-02 | `1. ` → 有序列表 | P0 | |
| IR-03 | `# ` / `## ` … → 标题（1-6） | P0 | |
| IR-04 | `> ` → 引用块 | P0 | |
| IR-05 | `\`\`\`` → 代码块 | P0 | |
| IR-06 | `---` → 分隔线 | P1 | |
| IR-07 | 输入规则可撤销（Backspace 回退） | P1 | ProseMirror：Backspace 撤销最近输入规则 |
| IR-08 | 智能引号/破折号（可选，中文场景默认关） | P2 | |

### 2.6 列表与块结构

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| LST-01 | 列表项内回车 → 新列表项 | P0 | markdown 编辑器核心 |
| LST-02 | 空列表项回车 → 退出列表（恢复普通段落） | P0 | |
| LST-03 | Tab/Shift+Tab：列表项缩进/提升（嵌套） | P1 | ProseMirror sinkListItem / liftListItem |
| LST-04 | 有序列表序号自动递增/重排 | P1 | markdown 源码层需要同步重排数字 |
| LST-05 | 任务列表（`- [ ]`）回车行为 | P2 | |
| LST-06 | 引用块内回车/续行 | P1 | |
| LST-07 | 代码块内回车 → 普通换行（非拆分块） | P0 | ProseMirror newlineInCode |
| LST-08 | 代码块结尾退出（Cmd+Enter / 末尾回车两次） | P1 | ProseMirror exitCode / createParagraphNear |

### 2.7 剪贴板

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| CLP-01 | 复制选中文本（保留 markdown 标记语义） | P0 | 现有 |
| CLP-02 | 剪切 = 复制+删除 | P0 | 现有 |
| CLP-03 | 粘贴多行文本拆分为多行 | P0 | 现有 insert_text 支持 |
| CLP-04 | 粘贴外部纯文本 → 按 markdown 解析粘贴（保留格式） | P1 | ProseMirror clipboardTextParser |
| CLP-05 | 粘贴 HTML → 转 markdown（可选） | P2 | 依赖 html→md 转换 |
| CLP-06 | 复制富文本到其他应用（生成 HTML/RTF） | P2 | |

### 2.8 撤销 / 重做

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| UND-01 | Cmd+Z 撤销、Cmd+Shift+Z / Cmd+Y 重做 | P0 | **当前缺失** |
| UND-02 | 连续键入合并为一个撤销步（时间窗口/字符合并） | P0 | ProseMirror history 的 merge 语义 |
| UND-03 | 撤销步同时恢复光标位置 | P0 | ProseMirror selection bookmark |
| UND-04 | 撤销不破坏标记结构（undo 后 `**` 仍配对） | P0 | |
| UND-05 | 撤销历史上限 / 溢出策略 | P2 | |

### 2.9 保存与往返保真

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| SAV-01 | 编辑→保存→重新加载，内容字节级一致 | P0 | 往返测试 |
| SAV-02 | 未保存标记（dirty）与外部修改冲突提示 | P1 | |
| SAV-03 | 空行、尾部换行、CRLF 的保留策略明确 | P1 | 当前 `join("\n")` 会丢尾部换行 |
| SAV-04 | 编辑视图与预览视图渲染一致（同一份 markdown 两个渲染器结果对齐） | P1 | 需要「预览快照」对照测试 |

### 2.10 焦点 / 滚动 / 可用性

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| FOC-01 | 切换编辑模式后自动聚焦编辑器 | P0 | 现有 |
| FOC-02 | 点击编辑器外焦点转移不丢数据 | P0 | 现有（dirty 保留） |
| SCR-01 | 光标移出可视区自动滚动 | P1 | 缺失 |
| SCR-02 | 编辑/预览切换保留滚动位置（可选） | P2 | |
| USR-01 | 光标形状（IBeam）、选区反色、组合下划线 | P1 | 部分现有 |
| USR-02 | 长文档性能：万行级不卡顿 | P1 | 需基准 |

### 2.11 渲染一致性（WYSIWYG 本质）

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| REN-01 | 编辑视图行内解析与预览视图对同一 markdown 产生一致语义 | P0 | **当前两套解析器（editor 自写 + pulldown）**，需对照测试 |
| REN-02 | 所有支持的行内语法（粗/斜/粗斜/删/码/高亮/链/上下标/emoji）在编辑视图可见 | P0 | 现有行内解析器需对照清单 |
| REN-03 | 未闭合标记的降级显示（不崩溃、可读） | P1 | 如只输入 `**` 未闭合 |
| REN-04 | 空文档 / 空白行 / 全角空格处理 | P1 | |

## 3. 测试方案

### 3.1 分层（参照 Lexical 三层 + ProseMirror test-builder）

| 层 | 覆盖 | 工具 | mkd 现状 |
| ---- | ---- | ---- | ---- |
| L1 纯逻辑单元 | 解析、映射、编辑操作、命令、撤销、往返 | Rust `cargo test` | 27 个，需扩展 |
| L2 编辑器状态测试 | 键盘/IME/命令序列作用于模型，无真实窗口 | GPUI test-support（`cx.simulate_keystroke`）或自建「命令执行器」 | 无，需建立 |
| L3 E2E / 手动回归 | 真实 UI、光标绘制、滚动、IME 候选窗 | 截图 + 行为脚本（受限环境） | 有 ad-hoc 截图，需 checklist 化 |

关键认知（来自 Lexical 文档，同样适用于 mkd）：
- **真实键盘/IME 无法在纯逻辑层测试**；逻辑层用「编辑器 API 直接驱动」，真实输入留给 E2E。
- **所有状态变更走单一入口**，测试才能回放。

### 3.2 L1 单元测试：位置标注 DSL（移植 prosemirror-test-builder 思路）

ProseMirror 用 `doc(p("foo<a>"))` 标注位置避免数 token。Rust 版建议：

```rust
// 输入：markdown 源码 + 光标标注
// "**加粗|** 文本"   → cursor 在 | 处（源码坐标）
// "**加粗<b></b>** 文本" → 双端标注选区
fn edit(src: &str, ops: &[Op]) -> String;  // Op 序列：Type("x") / Backspace / Enter / Left(n)...

// 断言：编辑后源码 + 光标位置
assert_edit("|abc", [Type("X")], "X|abc");
assert_edit("a|bc", [Backspace], "|bc");
assert_edit("**bo|ld**", [Type("X")], "**boX|ld**");   // FMT-01
assert_edit("|a\nb", [Enter], "|a\n|b");                // NAV/INP-05 拆分
```

设计要点：
- `Op` 枚举：`Type(&str)`、`Backspace`、`Delete`、`Enter`、`Tab`、`Left/Right/Up/Down/Home/End`、`SelectAll`、`Format(Bold)`、`Undo`、`Redo`、`Paste(&str)`、`Compose(&str, ime_events)`。
- 标注字符：`|`（光标）、`⟨a⟩...⟨/a⟩`（选区锚点）或 `<a>...</a>`（与 PM 一致）。
- 每个行为清单条目至少 1 个正向 + 1 个边界测试。

### 3.3 L1 命令与不变量测试

1. **标记配对不变量**：任意编辑操作后，扫描源码断言 `**` `*` `` ` `` `~~` `==` `[` `]` `(` `)` 配对平衡（FMT-01/02、UND-04）。
2. **往返不变量**：`load(md) → editor → to_source() == md`（SAV-01，对测试夹具集）。
3. **渲染-源码映射不变量**：对随机生成的标记文本，`cursor_display_col ∘ source_col_for_display == id`（NAV/REN）。
4. **命令 dry-run**：命令在无 dispatch 时不应修改状态（ProseMirror Command 契约）。
5. **撤销不变量**：`op ∘ undo ∘ redo == id`；连续键入合并为一步（UND-02/03）。

### 3.4 L1 输入规则测试

对 IR-01~06 逐个：输入触发序列（如依次 `Type("-") Type(" ") Type("item")`）→ 断言行首变为 `- item` 且样式为列表；Backspace 一步回退到纯文本（IR-07）。

### 3.5 L2 集成：GPUI test-support 或命令执行器

- 首选：给 Editor 加一个**纯状态 API**（`apply_op(&mut self, op)`），所有 UI 动作（键盘 handler、IME handler、菜单）都翻译成 Op。这样 L2 直接调 `apply_op`，不依赖窗口。
- 若需要真实 GPUI 事件流，用 GPUI `feature = "test-support"` 的 `simulate_keystroke` / `simulate_mouse_move`（需在测试内建 Application headless 模式）。
- IME 组合序列测试：构造 `Compose(["ㅎ","하","한"])` 风格的组合步骤（参照 Lexical `compose()` helper），断言最终文本 + marked 状态。

### 3.6 L3 E2E / 手动回归 checklist

受宿主环境限制（真实键盘注入不稳定），L3 采用「截图对照 + 半自动脚本」：
- 对每个行为清单 P0 项，准备一个 md 夹具 + 预期截图/文本断言。
- 用 `--edit 文件` 直接进入编辑态，截图比对（已有 `screencapture -l` 方案）。
- IME 候选窗定位（INP-03）必须在真实 mac 上人工验证一次。

### 3.7 测试夹具集（fixtures）

`tests/fixtures/` 下放行为用例：
```
01-plain-insert.md        02-marks-intraword.md     03-ime-chinese.md
04-lists-nested.md        05-codeblock.md            06-undo-redo.md
07-clipboard.md           08-roundtrip/*.md          09-render-parity/*.md
```

## 4. 优先级路线（建议实施顺序）

1. **P0 批量 1（模型与不变量）**：Op 模型 + 测试 DSL + 标记配对不变量 + 往返不变量 —— 先建立测试地基。
2. **P0 批量 2（输入规则）**：IR-01~06 自动列表/标题/引用/代码块 + IR-07 可撤销。
3. **P0 批量 3（撤销/重做）**：UND-01~04（合并策略 + 光标恢复）。
4. **P1 批量**：格式化命令 FMT-04/05、列表 Tab 缩进 LST-03/04、双击选词、滚动跟随。
5. **P1 一致性**：编辑/预览双渲染器对照测试（REN-01）与修复。
6. **P2**：bidi、HTML 粘贴、CRLF 保真、性能基准。

## 5. 与现有代码的差距摘要

| 能力 | 现有 | 差距 |
| ---- | ---- | ---- |
| 光标/选区基础 | ✅ | 上下列需视觉列；缺拖选 |
| 字符/IME 输入 | ✅（EntityInputHandler） | 缺组合中绘制与系统测试 |
| 编辑操作 | ✅ | 需 Op 化以便测试 |
| 输入规则 | ❌ | 全缺（IR-01~07） |
| 撤销/重做 | ❌ | 全缺 |
| 格式化命令 | ❌ | Cmd+B/I 等 |
| 列表续行/缩进 | 部分 | 回车续行、Tab 缩进 |
| 测试 | 27 单元 | 需 DSL + 不变量 + fixtures |
