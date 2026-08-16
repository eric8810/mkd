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
6. **行级 markdown 模型**（mkd 特有）：行是编辑单元，段落由「空行」分隔而非「回车」；任何「换行/回车」的语义都必须以 markdown 语法为准（见 2.12），不能照搬富文本编辑器的「回车=新段落」。

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
| NAV-10 | 光标闪烁（active 时可见、失焦隐藏） | P1 | 系统编辑器标准 |
| NAV-11 | IME 组合期间方向键/退格不打断组合，光标在组合区间边界内移动 | P0 | Lexical/系统输入法行为 |

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
| SEL-08 | 块级选择（Node selection）：点击/双击选整个列表项、代码块、段落；可删除/复制/粘贴/移动 | P1 | ProseMirror `NodeSelection`；markdown 行级模型下=选择整行或多行块 |
| SEL-09 | 拖选超出视口时自动滚动 | P1 | 鼠标拖选到边缘继续滚动 |
| SEL-10 | 跨块选区渲染：选区覆盖段落/列表/代码块时绘制连贯 | P1 | 现有整块高亮需按块边界精化 |
| SEL-11 | 选区拖动（drag-selection 中键/触摸） | P2 | |

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
| INP-10 | IME 组合期间输入/退格/光标移动不打断组合，组合文本实时更新 | P0 | 现有基础路径，需组合中编辑回归 |
| INP-11 | 组合文本的视觉标记（下划线/高亮）+ 候选窗跟随光标 | P0 | `bounds_for_range` 已实现，需验证 |
| INP-12 | macOS 系统编辑键：Ctrl+H=退格、Ctrl+D=删除、Ctrl+K=删至行尾、Ctrl+W=删词、Ctrl+U=删行 | P2 | ProseMirror macBaseKeymap |

### 2.4 格式化与标记（markdown 特有）

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| FMT-01 | 在 `**加粗**` 内部输入，插入后标记仍闭合 | P0 | 核心不变量 |
| FMT-02 | 光标紧邻标记符（`**` 前/后）时的插入归属明确 | P0 | 需定义：光标在 `**` 后 = 标记内 |
| FMT-03 | 退格删到标记边界：删整个标记符（`**` 两字符一起删） | P1 | ProseMirror 无此概念（富文本无标记符），markdown 编辑器需要 |
| FMT-04 | 格式化命令：Cmd+B/I 切换加粗/斜体（对选区应用/移除，光标处切换输入态） | P1 | ProseMirror toggleMark + storedMarks |
| FMT-05 | 格式化后的光标状态（typed marks）：输入延续当前标记 | P1 | ProseMirror storedMarks |
| FMT-06 | 链接的编辑：光标在链接内输入不破坏 `[text](url)` | P1 | 与 FMT-01 同族 |
| FMT-07 | Cmd+K 插入/编辑链接（选中文本包成 `[text](url)`） | P1 | 标准编辑器命令 |
| FMT-08 | 选区转列表（wrapInList）：把多段文字包成列表项 | P1 | ProseMirror `wrapInList` |
| FMT-09 | 块类型切换快捷键：标题 1-6 / 段落 / 代码块（Cmd+Opt+1…6、Cmd+Opt+C 等） | P1 | ProseMirror example-setup `Ctrl-Shift-0..6` 的 mac 版 |
| FMT-10 | 相邻同类块自动合并（autoJoin）：相邻两个 `- ` 列表合并为一个列表 | P1 | ProseMirror `autoJoin` |
| FMT-11 | 选区为空时切换格式化=设置后续输入状态（stored marks） | P1 | ProseMirror `toggleMark` + storedMarks |

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
| IR-09 | 自动链接输入规则：输入裸 URL/邮箱后自动包成链接 | P1 | ProseMirror / 常见 markdown 编辑器 |
| IR-10 | 任务列表输入规则：`- [ ]` / `- [x]` | P1 | |
| IR-11 | emoji 输入转换：`:smile:` 输入中实时补全/转换（或输入后转换） | P2 | 已有渲染期转换，输入期可选 |
| IR-12 | 输入规则触发时只替换「输入的一部分」（光标前的 token），不误伤已存在文本 | P0 | 如 `- ` 只在行首/列表上下文触发 |

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
| LST-09 | 有序列表 start 序号：`3.` 开头时后续序号从 3 递增，不强制从 1 | P1 | markdown 语义 |
| LST-10 | 跨块删除/合并语义：退格合并「段落↔列表项」「引用↔段落」「代码块↔段落」时的结构结果明确 | P1 | ProseMirror join 系列命令的 markdown 版 |
| LST-11 | 列表内「回车+连续输入」自动延续新列表项；手动打断（两次回车）回普通段落 | P0 | Typora/常见 md 编辑器 |

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
| UND-06 | 撤销后新的编辑操作清空 redo 栈 | P0 | 标准行为 |
| UND-07 | 输入规则（自动列表/标题）作为单个撤销步，Backspace 可回退 | P1 | ProseMirror inputrules undo |
| UND-08 | 合并策略细节：时间窗口 + 相邻可合并步；跨块/结构变化不合并 | P1 | ProseMirror history merge |

### 2.9 保存与往返保真

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| SAV-01 | 编辑→保存→重新加载，内容字节级一致 | P0 | 往返测试 |
| SAV-02 | 未保存标记（dirty）与外部修改冲突提示 | P1 | |
| SAV-03 | 空行、尾部换行、CRLF 的保留策略明确 | P1 | 当前 `join("\n")` 会丢尾部换行 |
| SAV-04 | 编辑视图与预览视图渲染一致（同一份 markdown 两个渲染器结果对齐） | P1 | 需要「预览快照」对照测试 |
| SAV-05 | 未保存退出保护：关闭窗口/退出时若有 dirty 内容弹确认 | P0 | 当前缺失，会丢编辑 |
| SAV-06 | 保存失败（只读/磁盘满）明确报错且不丢编辑内容 | P0 | 当前静默 |
| SAV-07 | 切换文件（open 新文件/Finder）时对当前 dirty 内容的策略明确（提示保存或丢弃） | P1 | 当前直接重置 |
| SAV-08 | 自动保存 / 定时保存（可选开关） | P2 | |
| SAV-09 | 只读文件/模式：禁止编辑或保存时明确提示 | P2 | |

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

### 2.12 行级 markdown 特有语义（mkd 架构核心）

这一节是**最重要的补充**。mkd 是「行 = 编辑单元」的 markdown 编辑器，与 ProseMirror 的富文本 DOM 模型有本质差异，以下是必须明确定义的语义：

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| MD-01 | 段落模型：相邻非空行 = 同一段落（预览中软换行）；空行 = 段落分隔 | P0 | 行级编辑器的根本语义，光标/回车/选区都依赖它 |
| MD-02 | 回车策略：段落中回车 → 拆出新行，且若两侧都是段落文本则**插入空行分隔**（否则预览仍同段落） | P0 | Typora：Enter 段落分隔、Shift+Enter 硬换行。**当前实现只是拆行，需明确** |
| MD-03 | Shift+Enter → 硬换行（行尾补两个空格） | P1 | markdown 硬换行语法 |
| MD-04 | 空行的插入/删除/光标停留 | P0 | 现有行模型支持，需测试 |
| MD-05 | 跨行标记**明确不支持**（`**bold\nmore**`）：单行解析器限制，编辑时按普通文本处理并文档化 | P1 | 当前限制，需显式行为而非偶然 |
| MD-06 | 行首 4 空格/`\t` 缩进的语义（代码块 vs 列表续行）在输入规则与渲染中一致 | P1 | markdown 缩进歧义 |
| MD-07 | 代码围栏状态跟踪（在 ``` 内输入不解析/不触发输入规则） | P0 | 现有 `in_fence` 状态，需测试 |
| MD-08 | 空行连续回车：代码块/列表/引用 内的「退出」通过额外空行或规则达成 | P1 | 与 LST-02/08 关联 |

### 2.13 拖放

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| DRG-01 | 拖拽选中文本/块移动（移动或按 Option 复制） | P1 | ProseMirror `handleDrop` |
| DRG-02 | 拖拽到视口边缘自动滚动 | P2 | |
| DRG-03 | 列表项/块的拖拽排序与嵌套 | P2 | |

### 2.14 查找与辅助功能

| ID | 行为 | 级别 | 说明 |
| ---- | ---- | ---- | ---- |
| FND-01 | Cmd+F 查找与高亮（含当前匹配滚动） | P1 | 标准编辑器功能 |
| ACC-01 | 可访问性：VoiceOver 可读编辑内容、焦点顺序、Tab 遍历 | P2 | macOS 原生应用应具备 |
| ACC-02 | 原生拼写检查（macOS NSSpellChecker） | P2 | |
| ACC-03 | 深色模式/主题对比度 | P2 | 当前仅浅色主题 |

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
2. **P0 批量 2（回车/段落语义）**：MD-01~08 段落空行分隔、Shift+Enter 硬换行、围栏状态 —— 行级编辑器的地基行为，必须先于输入规则。
3. **P0 批量 3（输入规则）**：IR-01~06 自动列表/标题/引用/代码块 + IR-07 可撤销。
4. **P0 批量 4（撤销/重做）**：UND-01~08（合并策略、redo 栈清空、光标恢复）。
5. **P0 批量 5（退出保护）**：SAV-05 未保存确认、SAV-06 保存失败报错。
6. **P1 批量**：格式化命令 FMT-04~11、列表 Tab 缩进/序号 LST-03/09、双击选词、拖选、滚动跟随。
7. **P1 一致性**：编辑/预览双渲染器对照测试（REN-01）与修复。
8. **P2**：bidi、拖放、HTML 粘贴、查找替换、CRLF 保真、辅助功能、性能基准。

## 5. 实现进度（截至当前）

| 能力 | 状态 | 说明 |
| ---- | ---- | ---- |
| Op 模型 + 测试 DSL | ✅ | 全部编辑收敛为 `Op`，`apply` 单一入口；`\|` 光标标注测试 |
| 光标/选区 | ✅ | 字符边界、记忆列上下移动、Home/End、拖选、双击选词、三击选行、全选、Shift 扩展 |
| 字符/IME 输入 | ✅ | EntityInputHandler + 组合路径（候选窗定位实测受限） |
| 输入规则 | ✅ | IR-01~07（列表/标题/引用/代码块/分隔线 + Backspace 回退）；IR-09/10 源码即语法天然满足 |
| 撤销/重做 | ✅ | UND-01~08（合并、redo 清空、光标恢复、结构不合并） |
| 格式化命令 | ✅ | FMT-01~11（包裹/去包裹/链接/块类型/选区转列表/stored marks/标记原子删除） |
| 列表行为 | ✅ | LST-01~11（续行/空项退出/Tab 缩进提升/序号递增/start 保留/跨行合并） |
| 回车/段落语义 | ✅ | MD-01~08（段落空行分隔、Shift+Enter 硬换行、围栏上下文） |
| 未保存退出保护 | ✅ | SAV-05（Cmd+Q prompt）；SAV-06 保存失败静默（待强化） |
| 滚动/光标 | ✅ | SCR-01 滚动跟随、NAV-10 光标闪烁、stick column |
| macOS 编辑键 | ✅ | INP-12（Ctrl+H/D/K/U/W、Alt+Backspace/Delete） |
| 渲染一致性 | ✅ | REN-01 对照测试（修复编辑视图缺失的上下标） |
| 查找/拖放/保存语义 | ✅ | FND-01 查找+高亮+滚动、DRG-01/02 文本拖移/复制+边缘滚动、SAV-02 外部修改冲突确认、SAV-03 尾部换行保留、SAV-06 保存失败报错、SAV-07 切文件确认 |
| macOS 编辑键与词移动 | ✅ | NAV-07 Option+方向键（CJK 每字一个边界）、INP-12 全套 |
| 组合文本视觉 | ✅ | USR-01 IME 组合下划线、光标闪烁 |
| 测试 | ✅ | 115 个单元测试全绿、零警告；万行解析 0.04s |

### P2 完成情况

| 行为 | 状态 | 方式 |
| ---- | ---- | ---- |
| 自动保存（SAV-08） | ✅ | `--autosave` 每 30s 写盘，外部修改时跳过 |
| 撤销历史上限（UND-05） | ✅ | undo 栈裁剪到 100 步 |
| 列表项拖拽排序（DRG-03） | ✅ | 整行列表项拖拽 → MoveLine 行移动 |
| HTML 粘贴转 markdown（CLP-05） | ✅ | objc2 直读 NSPasteboard HTML + 自研 html_to_md（7 测试） |
| 系统文件拖入 | ✅ | objc2 NSDraggingDestination 桥接到 NSView（真机拖放手势待手动确认） |

### 未实现（P2 / 框架限制）

| 行为 | 原因 |
| ---- | ---- |
| bidi（NAV-08） | 需逻辑↔视觉双向映射重做编辑光标层 |
| VoiceOver / 拼写检查（ACC） | 需 NSAccessibility 树桥接，工程量大 |
| 智能引号（IR-08） | 中文场景默认关闭 |
| 深色模式（ACC-03） | 仅浅色主题 |
