# 缺口 04: Skill 生命周期自动化

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[opencrust](E:\git\ai\opencrust) self-learning skill lifecycle: 5+ 次蒸馏 + confidence gate + 30 天归档 + 3-layer quality。

## 问题

千寻 v2 skill 是**静态加载**: 启动时 `SkillManager` 读 `~/.qianxun/skills/`, 中途不增删改, 无版本, 无淘汰。

后果:
- 用户用 100 次的 workflow 没沉淀成 skill
- 1 年前加载的废弃 skill 仍占 token
- 同名 skill 覆盖, 旧版丢失
- skill 失败率高无 audit

## 方案

### 4.1 4 状态 lifecycle

```rust
// qianxun-core/src/skills/lifecycle.rs (新)

pub enum SkillStatus {
    Active,     // 当前可用
    Candidate,  // 5+ 次使用, 等待 promote
    Archived,   // 30+ 天未用
    Quarantined, // confidence < 0.4, 暂停使用
}
```

### 4.2 触发条件

| 条件 | 动作 | 借鉴 |
|---|---|---|
| 同一 workflow (hash(task)) 重复 5+ 次 | → Candidate | opencrust 5+ rule |
| Candidate 评估 confidence ≥ 0.7 | → Active (正式 save) | opencrust confidence gate |
| Active 30+ 天未用 | → Archived (移到 `.archived/`) | opencrust 30-day auto-archive |
| 复用 Active 时成功率 < 0.4 (近 10 次) | → Quarantined | opencrust self-assess |

### 4.3 SkillLifecycle 模块

```rust
// qianxun-core/src/skills/lifecycle.rs

pub struct SkillLifecycle {
    skill_manager: Arc<SkillManager>,
    memory: Arc<MemoryCore>,
    usage_log: Arc<RwLock<HashMap<String, UsageRecord>>>,  // skill_name → {last_used, count, success_rate}
}

impl SkillLifecycle {
    /// 每个 session 结束调用
    pub async fn record_usage(&self, skill_name: &str, success: bool);

    /// 启动时跑一次 + 每天 0 点跑一次
    pub async fn tick(&self) -> LifecycleReport {
        // 1. 扫描 5+ 次且没 promote 的 → Candidate
        // 2. Candidate 评估 → Active / 留 Candidate
        // 3. Active 30+ 天未用 → Archived
        // 4. Active 失败率高 → Quarantined
    }

    /// LLM 复用 skill 时, silently 评估
    pub async fn on_skill_invoke(&self, skill_name: &str) {
        // 记录本次使用
        // 异步跑 confidence 重算
    }
}
```

### 4.4 持久化

- 新增表 `skill_lifecycle` (skill_name, status, last_used_at, use_count, success_count, confidence)
- 新增表 `skill_changelog` (skill_name, version, changed_at, reason, snapshot_diff)

### 4.5 Skill 格式升级

skill 文件新增字段 (向后兼容, 老 skill 自动 Active):
```markdown
---
name: my-skill
version: 2  # 每次 patch 自增
status: active  # active / candidate / archived / quarantined
created: 2026-01-01
last_used: 2026-06-09
use_count: 42
success_rate: 0.85
---
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/skills/lifecycle.rs` (新) | SkillLifecycle 主逻辑 | +200 |
| `qianxun-runtime/src/persistence.rs` | +2 张表 | +40 |
| `qianxun-core/src/skills/mod.rs` | 加 lifecycle 引用 | +10 |
| 测试 | 5+ / 30 天 / confidence / changelog | +80 |

**总计 ~330 行**

## 不做什么

- 不做 skill 自动生成内容 (LLM 蒸馏) — 简单为先, 5+ 次后只打标
- 不做 skill 跨用户共享 — 个人 AI, 单用户
- 不做 skill 评分 leaderboard — 内部用, 不外露

## 验收

- [ ] 同一 workflow 用 5 次 → status=Candidate
- [ ] Candidate 评估 confidence 0.8 → status=Active + 写 changelog
- [ ] Active 31 天未用 → status=Archived + 移目录
- [ ] 复用 skill 失败率 5/10 → status=Quarantined
- [ ] 启动时跑 tick, 不阻塞 boot (async 后台)
