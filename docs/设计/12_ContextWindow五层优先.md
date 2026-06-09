# 缺口 12: Context Window 五层优先

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[moltis](E:\git\ai\moltis) `HANDOFF-reconfigurable-context-windows.md`: 5 层 precedence 链, 避免 model context window 硬编码猜错。

## 问题

千寻 v2 compaction 不知道**每个 model 真实 context window**:

```rust
// 当前: 硬编码
fn max_tokens_for_model(model: &str) -> u32 {
    if model.starts_with("claude-") { 200_000 }
    // 问题: deepseek-v4-flash 真实是 128k 还是 200k? minimax-M3?
}
```

后果:
- deepseek 实际 200k, 千寻按 128k 压, 浪费 36% 容量
- 实际 100k, 千寻按 200k 不压, 溢出

## 方案

### 12.1 5 层 precedence

```rust
// qianxun-core/src/provider/capabilities.rs (新)

pub fn context_window_for_model(
    model: &str,
    config: &ModelCapabilitiesConfig,
) -> u32 {
    // 1. provider-scope config (e.g. "deepseek.json" 写明 deepseek-v4-flash = 128000)
    if let Some(v) = config.provider_scope.get(model) {
        return *v;
    }

    // 2. global config ("~/.qianxun/config.json" model_capabilities)
    if let Some(v) = config.global.get(model) {
        return *v;
    }

    // 3. API metadata (启动时 GET /v1/models 拿, 缓存 24h)
    if let Some(v) = api_metadata_cache.get(model) {
        return *v;
    }

    // 4. heuristic (model name prefix 猜)
    if let Some(v) = heuristic_guess(model) {
        return *v;
    }

    // 5. trait default
    128_000
}
```

### 12.2 API metadata 抓取

```rust
// qianxun-core/src/provider/capabilities.rs

pub async fn fetch_api_metadata(provider: &str, api_key: &str) -> HashMap<String, u32> {
    // Anthropic: GET https://api.anthropic.com/v1/models → 解析 context_window
    // OpenAI:    GET https://api.openai.com/v1/models → 同
    // DeepSeek:  GET https://api.deepseek.com/models → 同
    // 失败 → 返回空 (走 heuristic)
}
```

启动时跑一次, 缓存 24h, 后台定时刷新。

### 12.3 配置

```json
// ~/.qianxun/config.json
{
  "model_capabilities": {
    "global": {
      "deepseek-v4-flash": 128000,
      "minimax-M3": 200000
    },
    "deepseek": {
      "deepseek-v4-flash": 200000  // 覆盖 global
    }
  }
}
```

### 12.4 接入 compaction

```rust
// qianxun-core/src/agent/context/compact.rs

pub fn should_compact(&self) -> bool {
    let max = context_window_for_model(self.model, &self.config.capabilities);
    let usage = self.estimate_tokens();
    usage >= max * self.config.hard_threshold_ratio
}
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/provider/capabilities.rs` (新) | 5 层 chain | +200 |
| `qianxun-core/src/config.rs` | + ModelCapabilitiesConfig | +30 |
| `qianxun-core/src/agent/context/compact.rs` | 接入 | +15 |
| 测试 | 5 层覆盖 + 启发式 | +60 |

**总计 ~305 行**

## 不做什么

- 不做 model capabilities 自动学习 (用户配置为主)
- 不做 per-session override (全局足够)
- 不做 model 评分 (留给缺口 08 Provider Failover)

## 验收

- [ ] provider-scope 配置 > global > API > heuristic > default
- [ ] API metadata 抓取成功 → 缓存 24h
- [ ] API metadata 抓取失败 → fallback heuristic
- [ ] 改 config.json 后下次启动生效 (不热更)
- [ ] compaction 阈值用真实 context window, 不再溢出
