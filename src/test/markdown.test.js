import { test } from 'node:test';
import assert from 'node:assert/strict';

import { inline, render, toggleTodo, openTodos, continueList } from '../js/markdown.js';

const lookup = (id) => (id === 'a1' ? { text: '被引用的那条内容' } : undefined);

// ── 安全 ────────────────────────────────────────────────────────

test('正文里的 HTML 被转义', () => {
  const out = render('<script>alert(1)</script>');
  assert.ok(!out.includes('<script>'), out);
  assert.ok(out.includes('&lt;script&gt;'), out);
});

test('属性注入不出逃', () => {
  // 标签名会进 data-tag 属性。判据不是「输出里没有 onmouseover 这几个字」——
  // 那几个字作为文本出现是无害的——而是属性值里没有能闭合它的裸引号。
  const out = render('#a"onmouseover="alert(1)');
  const attr = out.match(/data-tag="([^"]*)"/);
  assert.ok(attr, out);
  assert.ok(!attr[1].includes('"'), attr[1]);
  assert.ok(!/<[a-z]+[^>]*\son[a-z]+=/i.test(out), '不该出现任何事件处理器属性');
});

test('转义只做一次', () => {
  // esc() 已经把整段转义过，捕获组再 esc() 一次会让 & 显示成 &amp;
  assert.ok(render('#读书&写作').includes('#读书&amp;写作'));
  assert.ok(!render('#读书&写作').includes('&amp;amp;'));
});

test('只放行 http 与 https', () => {
  assert.ok(render('[文档](https://example.com)').includes('<a href="https://example.com"'));
  assert.ok(render('[文档](http://example.com)').includes('<a href='));

  // 这几个交给系统打开等于本地代码执行的入口
  for (const bad of [
    '[点我](javascript:alert(1))',
    '[点我](file:///C:/Windows/System32/cmd.exe)',
    '[点我](ms-msdt:/id PCWDiagnostic)',
    '[点我](vbscript:msgbox)',
  ]) {
    const out = render(bad);
    assert.ok(!out.includes('<a '), `${bad} → ${out}`);
    assert.ok(out.includes('点我'), '不安全的链接应降级成纯文本而不是消失');
  }
});

// ── 行内 ────────────────────────────────────────────────────────

test('行内样式', () => {
  assert.ok(inline('**粗**').includes('<strong>粗</strong>'));
  assert.ok(inline('*斜*').includes('<em>斜</em>'));
  assert.ok(inline('~~删~~').includes('<del>删</del>'));
  assert.ok(inline('==亮==').includes('<mark>亮</mark>'));
  assert.ok(inline('`码`').includes('<code>码</code>'));
});

test('行内码里的标记不被解析', () => {
  const out = inline('`**不该加粗** #不是标签`');
  assert.ok(!out.includes('<strong>'), out);
  assert.ok(!out.includes('itag'), out);
});

test('正文里的数字不会被当成行内码占位符', () => {
  // 占位符若不用哨兵字符，`共 3 条` 里的 3 会被替换成一段代码
  const out = inline('共 3 条，其中 12 条有标签');
  assert.ok(!out.includes('<code>'), out);
  assert.ok(out.includes('3') && out.includes('12'), out);
});

test('标签成为可点击按钮', () => {
  const out = inline('读了点东西 #读书/认知 挺好');
  assert.ok(out.includes('data-tag="读书/认知"'), out);
});

test('引用渲染成摘要，目标缺失时降级', () => {
  assert.ok(inline('见 [[那条^a1]]', lookup).includes('data-ref="a1"'));

  // 目标被删：格式合法但查不到，渲染成 .dead 而不是消失——
  // 引用悄悄消失比留一个失效引用更糟
  const dead = inline('见 [[那条^b2c3d4e5-0000-4000-8000-000000000000]]', lookup);
  assert.ok(dead.includes('iref dead'), dead);
  assert.ok(!dead.includes('data-ref'), '失效引用不该可点击');
});

test('id 格式不合法的不当作引用', () => {
  const out = inline('这不是引用 [[随手写的^中文]]', lookup);
  assert.ok(!out.includes('iref'), out);
});

// ── 块级 ────────────────────────────────────────────────────────

test('标题只认二到四级', () => {
  assert.ok(render('## 小节').includes('<h2>'));
  assert.ok(render('#### 小小节').includes('<h4>'));
  // 一级标题不支持——片语没有标题，单个 # 是标签
  assert.ok(!render('# 一级').includes('<h1>'));
});

test('列表与待办', () => {
  assert.ok(render('- 甲\n- 乙').includes('<ul><li>甲</li><li>乙</li></ul>'));
  assert.ok(render('1. 甲').includes('<ol>'));
  const todo = render('- [ ] 未做\n- [x] 已做');
  assert.ok(todo.includes('class="ul todo"') || todo.includes('class="todo"'), todo);
  assert.ok(todo.includes('data-todo="0"') && todo.includes('data-todo="1"'), todo);
  assert.ok(todo.includes('ck done'), '已勾选的要有 done');
});

test('待办优先于无序列表', () => {
  // `- [ ] x` 也匹配无序列表的模式，顺序反了会渲染成普通列表项
  assert.ok(render('- [ ] 甲').includes('data-todo='));
});

test('代码块保留原文且不解析内部标记', () => {
  const out = render('```\n# 注释 **不加粗**\n```');
  assert.ok(out.includes('<pre><code>'), out);
  assert.ok(out.includes('# 注释'), out);
  assert.ok(!out.includes('<strong>'), out);
});

test('引用块与分割线', () => {
  assert.ok(render('> 引用').includes('<blockquote>引用</blockquote>'));
  assert.ok(render('---').includes('<hr>'));
});

test('段落内换行成 br，空行分段', () => {
  const out = render('第一行\n第二行\n\n第二段');
  assert.equal((out.match(/<p>/g) || []).length, 2, out);
  assert.ok(out.includes('<br>'), out);
});

test('残缺输入不抛异常', () => {
  for (const src of ['**没配对', '==没收尾', '[[', '```rust', '- [ ', '###', '']) {
    assert.doesNotThrow(() => render(src), src);
  }
});

// ── 待办改写 ────────────────────────────────────────────────────

test('勾选改写源文本且只动目标项', () => {
  const src = '前言\n- [ ] 甲\n- [ ] 乙\n后记 #读书';
  const after = toggleTodo(src, 1);
  assert.ok(after.includes('- [ ] 甲'));
  assert.ok(after.includes('- [x] 乙'));
  assert.ok(after.includes('后记 #读书'), '不该动正文其余部分');
  assert.equal(after.length, src.length, '只该换掉一个字符');
});

test('再勾一次取消', () => {
  assert.equal(toggleTodo(toggleTodo('- [ ] 甲', 0), 0), '- [ ] 甲');
});

test('越界的序号不改任何东西', () => {
  const src = '- [ ] 甲';
  assert.equal(toggleTodo(src, 9), src);
});

test('统计未勾选待办', () => {
  assert.deepEqual(openTodos('- [ ] 要做\n- [x] 做完\n- [ ] 也要做'), ['要做', '也要做']);
  assert.deepEqual(openTodos('没有待办'), []);
});

// ── 回车续行 ────────────────────────────────────────────────────

test('续列表', () => {
  assert.equal(continueList('- 甲'), '- ');
  assert.equal(continueList('  * 缩进'), '  * ');
  assert.equal(continueList('1. 甲'), '2. ');
  assert.equal(continueList('9) 甲'), '10) ');
  assert.equal(continueList('> 引用'), '> ');
});

test('续待办时新项一律未勾选', () => {
  assert.equal(continueList('- [x] 已做'), '- [ ] ');
  assert.equal(continueList('- [ ] 未做'), '- [ ] ');
});

test('空项跳出列表', () => {
  for (const line of ['- ', '- [ ] ', '1. ', '> ']) {
    assert.equal(continueList(line), '', line);
  }
});

test('普通行不续', () => {
  assert.equal(continueList('普通一行'), null);
  assert.equal(continueList(''), null);
  assert.equal(continueList('## 标题'), null);
});
