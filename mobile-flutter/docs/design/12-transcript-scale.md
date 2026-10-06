# 转录规模实测 —— 工具卡截断阈值的依据

> 证据：`scripts/probe_transcript_scale.py` → `docs/probe/transcript-scale.json`
> 样本：`session-ef57cbfd-…`（本部署**步数最多**的一条，2485 步）

---

## 1. 实测数字

```
取回 1910 条记录（hasMore=True，说明还有更早的没取完）
总计 6.33 MB（6,642,122 字节）

每条事件字节
  全部         n=1910  p50=489      p90=9,592     p99=45,209   max=142,782
  工具/步骤类  n=1425  p50=94        p90=2,010     p99=13,242   max=35,229
  消息类       n=403   p50=6,905     p90=29,208    p99=82,451   max=142,782

超阈值的条数
  >  8 KB:  211 条 (11.0%)
  > 16 KB:  105 条 ( 5.5%)
  > 32 KB:   41 条 ( 2.1%)
  > 64 KB:    8 条 ( 0.4%)
  >128 KB:    1 条 ( 0.1%)
  >256 KB:    0 条 ( 0.0%)

事件类型分布
  assistant/message   360
  step/end            360
  step/start          359
  tool/result         355
  tool/call           351
  user/message         42
  turn/end              9
  workspace/changes     8
  turn/start            8
  compaction/prune      4
```

---

## 2. 三条被实测修正的判断

### 2.1 ⚠️ 要截断的是**消息**，不是工具行

我原本判断「工具输出 2MB，要截断工具卡」。实测反过来：

| | p50 | p90 | p99 | max |
|---|---|---|---|---|
| 工具/步骤类 | **94 B** | 2.0 KB | 13.2 KB | 35.2 KB |
| 消息类 | **6.9 KB** | 29.2 KB | 82.5 KB | **142.8 KB** |

**工具事件本身极小**（p50 才 94 字节，就是一个 id + 状态）。
大的是 `assistant/message` 的 markdown 正文（p50 就 6.9KB，是工具事件的 73 倍）。

所以 `custom.css` 里给 `[data-disclosure-row]`（工具行）设 48dp 触控目标
就够了，**不需要给它做截断**。要截断的是消息正文。

### 2.2 「一次 2MB 输出」这个说法是**单条上限**，不是典型

我此前记的「一次 rg 输出 2MB」——实测单条最大 **142KB**，没有 2MB。
2MB 是**整条会话累计**（6.33MB）的量级。

这个区别影响判断：
- 若按「单条可能 2MB」设计截断，会给 99% 的内容加一个用不上的「全文」按钮
- 实测 11% 超过 8KB，所以**折叠 12KB、超出才给「全文」**，命中面合理

### 2.3 分页是必须的，且要留「更早」入口

`hasMore=True` —— 1910 条**不是全部**。而这一条会话已经有 6.33MB。

⚠️ **不能一次性拉全量**。正确做法：
1. 首屏只取最近 N 条（实测 p90=9.4KB，取 50 条约 470KB，可接受）
2. 向上滚动时用 `beforeSeq` 继续分页
3. 每页**只渲染可见窗口**（虚拟滚动），不能一次塞进 DOM

---

## 3. 工具卡两层披露的定稿规格

```
折叠态（默认）
┌──────────────────────────────────────┐
│ > Bash · 成功 · 1.2s            ›    │  ← data-disclosure-row，48dp
│   142 个文件                          │  ← 一句话结论（若有）
└──────────────────────────────────────┘
        ↓ 点开
展开态
┌──────────────────────────────────────┐
│ $ rg --files lib/ | wc -l             │  ← 等宽，不换行，横向滚
│ 142                                  │
│ …还有 38 行            [看全文]      │  ← 超 12KB 才出现
└──────────────────────────────────────┘
```

| 参数 | 值 | 依据 |
|---|---|---|
| 折叠行高 | 48dp | Apple HIG / Material 下限 |
| 展开后行数 | 20 行 | 20 行 × ~40 字符 ≈ 一屏多一点 |
| 折叠字节阈值 | **12 KB** | 略高于 p90（9.4KB）；实测 11% 超 8KB，命中面合理 |
| 「看全文」触发 | 单条 > 12 KB | 同上 |
| 全文呈现 | 底部 sheet，可滚动 | 139KB 塞进主视图会挤掉一切 |

⚠️ **全文必须惰性取**：不能预渲染。取的时候用该条的原始内容，
渲染进 sheet 再说。

---

## 4. 顺带修正 API 文档的一个实质错误

实测 `session/page` 的参数形状：

```
平铺 {address, throughSeq, maxMessages}
  → gateway/arguments-invalid
    args fields do not match the descriptor:
    missing "request"; unexpected "address", "throughSeq", "maxMessages"

正确 {request: {address, throughSeq, maxMessages}}
```

`docs/api/dsh-remote-api.md` 原先把 `session/page` / `session/follow`
记成**平铺**的。已更正，并写明规律：

> 除 `session/list`（参数键恰好叫 `_request`）外，
> **其余方法都要包一层 `request`**。

早前那句「`payload.args` 是唯一合法键」只说了信封层，
**没说清 args 内部还有一层 `request`** —— 那是个会误导所有照它写的人的坑。

另：`throughSeq` 是**游标**，不是任意上限。填 `2**31-1` 会得到
`session page through seq 2147483647 is past cursor 13487`。
真实值取自 `projections.asOfSeq`。
