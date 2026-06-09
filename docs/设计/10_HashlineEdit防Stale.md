# 缺口 10: Hashline Edit 防 Stale

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[oh-my-opencode](E:\git\ai\oh-my-opencode) `hashline-edit`: Read 输出加 `LINE#ID` content hash, Edit 提交时校验 hash 拒绝 stale。

## 问题

千寻 builtin `write_file` / `edit_file` 工具**无 stale 检测**:

```rust
// LLM 流程
1. read_file("config.rs") → 拿到 v1 内容
2. 用户另一处改了 config.rs → 变 v2
3. LLM 用 v1 上下文调 edit_file("config.rs", 基于 v1 改)
4. 默默写入 → 基于 v1 改 + v2 的内容, 结果错乱
```

## 方案

### 10.1 Read 返回带 hash

```rust
// qianxun-core/src/tools/builtin/hashline_read.rs (新)

pub fn read_with_hashline(path: &Path) -> Result<HashlinedContent, ToolError>;

pub struct HashlinedContent {
    pub lines: Vec<HashlineLine>,
    pub file_hash: String,  // 整个文件的 hash
}

pub struct HashlineLine {
    pub num: u32,             // 42
    pub content: String,      // "fn main() {"
    pub hash: String,         // 8 字符 (ZPMQVRWSNKTXJBYH 字符表)
    pub char_count: u32,
}
```

输出格式:
```
42#ZPMQ fn main() {
43#VRWS     println!("hello");
44#NKTX }
```

### 10.2 Edit 必须带 hash

```rust
pub fn hashline_edit(path: &Path, edits: Vec<HashlineEdit>) -> Result<(), ToolError>;

pub struct HashlineEdit {
    pub line_num: u32,        // 42
    pub line_hash: String,    // "ZPMQ"
    pub new_content: String,  // "fn main() -> Result<()> {"
}
```

校验逻辑:
```rust
fn apply_edit(file: &mut File, edit: HashlineEdit) -> Result<()> {
    let current = read_line(file, edit.line_num)?;
    let current_hash = hash(&current);
    if current_hash != edit.line_hash {
        return Err(ToolError::StaleEdit {
            line: edit.line_num,
            expected_hash: edit.line_hash,
            current_hash,
            current_content: current,
        });
    }
    write_line(file, edit.line_num, &edit.new_content)?;
    Ok(())
}
```

### 10.3 StaleEdit 错误返回

LLM 收到 `StaleEdit` 后, 应该:
1. 重新 read_file 拿最新 hash
2. 用新 hash 重提 edit

主 session 收到 stale 后, 可以 reflect retry (跟缺口 11 联动)。

### 10.4 配置

```rust
pub struct HashlineConfig {
    pub enabled: bool,                // 默认 true
    pub hash_length: usize,           // 默认 8 字符
    pub char_table: String,           // 默认 "ZPMQVRWSNKTXJBYH" (omo 同源)
}
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/tools/builtin/hashline.rs` (新) | read + edit | +150 |
| `qianxun-core/src/tools/builtin/mod.rs` | 注册 | +10 |
| builtin/read_file.rs, write_file.rs | 改造 | +50 |
| 测试 | stale 检测 + 正常 edit | +60 |

**总计 ~270 行**

## 不做什么

- 不做 binary 文件 hashline (只 text)
- 不做多文件原子 hashline edit
- 不做 hash 冲突检测 (8 字符 = 16^8 = 42 亿种, 冲突概率极低)

## 验收

- [ ] read_file 返带 hashline 的内容
- [ ] edit_file 用过期 hash → StaleEdit 错误
- [ ] edit_file 用正确 hash → 写入成功
- [ ] LLM 收到 StaleEdit 自动重试 → 第二次成功
- [ ] hash 字符表正确 (16 字符)
