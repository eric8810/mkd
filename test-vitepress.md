---
title: VitePress 特性全览
description: 覆盖 mkd 对 VitePress Markdown 扩展的支持情况
---

[[toc]]

## GFM 基础

- 删除线：~~这是删除的内容~~
- 自动链接：https://example.com 和 <mailto:hi@example.com>
- 任务列表：
  - [x] 已完成事项
  - [ ] 未完成事项
  - [ ] 另一个待办

## 表格

| 特性 | 支持 | 备注 |
| ---- | :---: | ---- |
| 表格 | ✅ | GFM |
| 脚注 | ✅ | 见下文[^1] |
| 上下标 | ✅ | H~2~O 和 E=mc^2^ |

[^1]: 这是脚注内容，会自动显示在文档底部。

## 自定义容器

::: tip
这是一个 **提示** 容器。
:::

::: warning 自定义警告标题
小心使用，这里有警告内容。
:::

::: danger
危险操作，请勿执行。
:::

::: info
信息类容器，用于补充说明。
:::

::: details 点击展开详情
这是可折叠的详情内容。
:::

::: code-group
```rust
fn main() { println!("hello"); }
```

```python
print("hello")
```
:::

## 代码块高级特性

### 行高亮与行号

```ts {2,4-5} :line-numbers
const items = [1, 2, 3];
const doubled = items.map((n) => n * 2);
console.log(doubled);
const sum = doubled.reduce((a, b) => a + b, 0);
console.log(sum);
```

### 代码块标题

```js title="example.js"
export function hello(name) {
  return `Hello, ${name}!`;
}
```

### 导入代码片段

<<< @/snippets/helper.js

### 导入片段并高亮行

<<< @/snippets/helper.js{1,3}

## 文本增强

- 高亮标记：这是 ==重点内容== 需要强调
- Emoji：:tada: :rocket: :sparkles: :heart:
- 数学公式：行内 $E = mc^2$，块级如下

$$
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$

## 定义列表

苹果
: 一种常见的水果
: 也是科技公司名

橘子
: 柑橘类水果

## HTML 与徽章

<VBadge type="tip" text="实验性" />

原生 HTML 块：

<div class="notice">
这是原生 HTML 内容，按纯文本展示。
</div>

## 标题属性

### 自定义标题 {#custom-id}

这个标题带自定义锚点 id。
