# 09 · Laya 决策引擎与 DSH 集成设计

> 状态:Phase 0 已完成(English 基线)/ Phase 1-2 进行中(multilingual 中文 / dsh 插件)
> 代码:`src-tauri/crates/laya-engine`、`src-tauri/crates/laya-cli`
> 模型资产:`models/laya-onnx`(English 421M)、`models/laya-multilingual-onnx`(转换中)
> 上游设计对齐:《DSH 架构设计》01-07(Cloud Brain + Local Execution + Laya Decision Plane)

---

## 1. 背景与定位

Laya 是 Convai Innovations 的开源 **System-1 非自回归决策模型**:输入
`state + typed questions`,**一次 forward** 输出全部答案(choice / score / noul,
带校准概率),**零生成 token**。在千寻架构中它不是 Main Agent、不是 Qwen 替代品,
而是旁路的 **Decision Plane**:

```
是否下沉本地;任务分类;是否升级云端;工具风险分级;
Verification 是否满足结束条件;Retry / Escalation / Approval 的结构化判断
```

**不负责**:代码生成、长文本推理、复杂规划、系统权限、最终用户回答。

硬边界(红线):**模型不是权限系统**。Laya 只产出结构化 Decision + policy_hint,
真正执行由 Policy 决定;Laya 不允许直接调用 Tauri command / native capability。

```
Laya Decision → Decision Contract → Policy → Action → (如需 native)qianxun-bridge → Rust
```

## 2. 总体架构

```
Qianxun (Tauri/Rust, Native Control Plane)
 ├── src-tauri/crates/laya-engine      ← Laya 推理引擎(本设计核心)
 ├── src-tauri/crates/laya-cli         ← 探针 / 压测 CLI
 ├── crates/laya-server(规划中)       ← localhost HTTP sidecar(127.0.0.1 only)
 ├── vendor/onnxruntime/onnxruntime.dll ← 自备 ORT 运行时(gitignored)
 ├── models/laya-*.onnx/               ← 模型 bundle(gitignored)
 │
 └── embedded npm / dsh
      └── decision-laya(Host bundle 插件,规划中)
           ├── Tool `laya_decide`      ← Agent 可调用
           ├── Cordis Service          ← policy/verification 插件注入消费
           └── Config(endpoint/阈值)  ← 插件管理器可改
```

调用链(Zero-Fork,上游 dsh 不可变):

```
DSH Agent → laya_decide tool → HTTP 127.0.0.1:10230/predict → laya-engine → Decision JSON
                                              ↑
                       laya-server 常驻(未来由 Qianxun Rust supervisor 拉起)
```

## 3. 模块设计

### 3.1 laya-engine(推理引擎 crate)

| 模块 | 职责 | 对齐参考 |
|---|---|---|
| `sequence.rs` | 提示序列构造:`[CLS] <type> question: <ins> [SEP] [MASK] opt… [SEP] state [SEP]`,选项 48 token 截断、head 192 预算、marker 定位 | laya-mlx `common.py` 逐行移植 |
| `config.rs` | `laya_config.json` 解析 + **温度钳制 [0.5, 5.0]**(checkpoint 自带 `choice:11+ = 0.1006` ≈10 倍锐化,会把近似随机报成确定,必须拒绝) | laya-mlx `clamp_temperature` |
| `questions.rs` | typed questions(choice/score/noul)与选项渲染;state 为 dict/list 时按 Python `json.dumps` 默认分隔符序列化 | 上游 `RLAgent` |
| `tokenizer.rs` | 原生加载 `tokenizer.json`(Rust `tokenizers` 库,即 Python 侧的底层),cls/sep/pad/mask 特殊 token 解析 | laya-mlx `tokenizer.py` |
| `engine.rs` | ORT 会话:懒加载 + Mutex(仿 egui_hikvision_app `OnnxRunner` 模式)+ `ensure_ort_dylib()` DLL 自动发现 | — |
| `agent.rs` | `system_one` 编排:collate → forward → 温度校准 → 熵置信度 → 答案形状;`{model, answers, usage}` | laya-mlx `agent.py` |

ONNX 输入输出契约(receptron/laya-onnx 定义,与 PyTorch 参考实现 logit diff ≈1e-5):

```
inputs : input_ids [B,L] i64 · attention_mask [B,L] i64 · marker_pos [B,K] i64 ·
         marker_mask [B,K] bool · qtype [B] i64
outputs: logits [B,K] f32(未校准,masked=-1e4) · act_probs [B,2] f32
```

公开 API:

```rust
let agent = LayaAgent::load(model_dir, batch_size, EngineOptions { ep: Ep::Cpu, .. })?;
let result /* serde_json::Value */ = agent.predict(&state, &questions)?;
```

### 3.2 laya-cli(探针 / 压测)

```powershell
# 单次预测 + 延迟
laya-cli --model-dir models\laya-onnx --questions examples\questions.json `
         --state-file examples\state.json --repeat 20
```

### 3.3 laya-server(规划中,Phase 2)

`tiny_http` 实现的 localhost sidecar,**127.0.0.1 only、无鉴权依赖、不公网绑定**
(对齐 Runtime 文档 06 §8:Local Model Service 与 native shell 解耦):

```
POST /predict   {state, questions} → system_one JSON + 耗时
GET  /health    → {ok, model, checkpoint}
```

未来由 Qianxun Rust supervisor 拉起(spawn/health-check/restart),插件不阻塞 UI 首屏。

### 3.4 decision-laya(Host bundle 插件,规划中,Phase 2)

零依赖 npm 包(`package.json` 声明 `dsh.bundle.patch`):

- **Tool `laya_decide`**:`inject: ['tools']` + `ctx.tools.register`,入参
  `{state, questions}`,出参为 Laya answers + `policy_hint`(由 Config 阈值计算:
  `proceed | escalate | reject`)。插件只产出 Decision,不执行任何动作。
- **Cordis Service**:service class 形式导出,供 policy/verification/memory 插件注入。
- **Config**(JSON Schema):`endpoint / timeoutMs / 阈值`,插件管理器可视化修改。
- 模型不可用时优雅降级(tool 返回结构化错误,不炸 session)。

## 4. 关键技术决策

| 决策 | 理由 |
|---|---|
| **Rust + `ort`(onnxruntime)而非 OpenCV DNN** | egui_hikvision_app 的 OpenCV DNN 适合 PP-OCR 这类 CNN 图像 blob;Laya 是 ModernBERT transformer(int64/bool 多输入、动态序列长),DNN 导入器覆盖不了。且 onnxruntime 与官方 Node 实现(@receptron/laya)同引擎,数值语义可直接对齐,未来可插 DirectML/CUDA EP 用 RTX 4080。egui 项目的**架构模式**(懒加载+Mutex+预设构造器)原样沿用 |
| **`load-dynamic` + 自备 DLL** | ort-sys 预编译静态库是 `/MT` 编译,与 Rust `/MD` 冲突(LNK2005/LNK1169);官方 release 的 `onnxruntime.dll` 动态加载彻底绕开 CRT 问题,且符合"库与模型都是本机静态文件"的部署模型 |
| **DLL 自动发现** | `ensure_ort_dylib()`:`ORT_DYLIB_PATH` 环境变量优先 → exe 目录 → 从 exe/cwd 向上逐级找 `vendor\onnxruntime\onnxruntime.dll`(与 cargo test 深路径、exe 部署、qianxun 打包布局均兼容) |
| **优化等级用 `All` 不用 `Level3`** | ort rc.13 的 `Level3` 映射 `ORT_ENABLE_LAYOUT`(新版 ORT 才有),官方 1.22.x DLL 报 `graph_optimization_level is not valid`;`All(ORT_ENABLE_ALL)` 全版本有效 |
| **Session 泄漏式退出** | ORT 在进程收尾销毁全局线程池时偶发 fail-fast(0xC0000409),与 pyke ort "环境常驻" 同源问题。CLI 显式 `exit(0)`;测试 `mem::forget(agent)`;长期进程(server/qianxun)Session 常驻天然不受影响 |
| **fp32 导出不量化** | "不能降低性能(准确性)" 硬约束:receptron 导出与 PyTorch 参考实现四位小数/1e-5 logit 对齐;量化留待自建 Decision Benchmark 验收后再议 |

## 5. 模型资产管线

```
convaiinnovations/laya-multilingual (mmBERT-base 322M, 1024 ctx, fp16 643MB)
   │  + rl_common.py(从 convaiinnovations/laya 主仓补拉,build_model 需要)
   ▼  receptron/laya export/export_onnx.py(torch 2.14 CPU + transformers 5.17)
onnx bundle: laya.onnx(fp32 ~1.3GB) + laya_config.json + tokenizer/
   │  导出脚本自带 parity 校验:max |dlogits| ≈ 1e-5
   ▼
models/laya-multilingual-onnx/
```

**不降性能论证**:① 数值——fp32 全精度导出,无量化损失;② 延迟——multilingual
322M < english 421M,FLOPs 更少,基线只会更快;③ 上下文——1024 > 512。
中文/非英语必须走 multilingual(english checkpoint 在非英语上崩溃且高置信,ECE 0.855)。

**已知限制**:HF `receptron/laya-onnx` 目前只有 English 现成 bundle;multilingual 必须
自行导出(本文档写作时正在执行)。`laya_config.json` 的 `max_len/head_max_len/温度`
由导出脚本自动从 `rl_agent_config.json` 提取,engine 侧零改动兼容。

## 6. 测试与验证体系

| 层 | 内容 | 状态 |
|---|---|---|
| 单元测试(10) | 温度分桶/钳制、熵置信度、softmax、选项渲染、JSON 序列化对齐 | ✅ 绿 |
| tokenizer smoke | 真实 tokenizer 加载 + 序列结构(cls=101/sep=102/pad=0/mask=103、marker 对位、预算截断) | ✅ 绿 |
| model parity | 真实权重加载 + 推理:概率归一、score 值域、noul 语义(退款邮件 P(true)>0.5) | ✅ 绿 |
| **stress** | `laya-cli stress`:混合 8 状态(中/英/trace)循环 200 次 × 4 并发,确定性逐字节一致,0 错误,P50 680ms(并发争抢)/串行 P50 122.7ms | ✅ PASS |
| **CDP e2e** | `e2e/probe-laya-decision.ts`:spawn debug 千寻 → `connectOverCDP(10222)` → DSH 新会话 → 10 条中文工单驱动 agent 调 `laya_decide` → 断言 `部门=<choice>` 回复 | ✅ **10/10 通过** |

English 基线实测(Windows 11 · CPU EP · 3 题一次 forward · 254 tokens):

```
department = billing  P=0.9554    ← 重复扣款邮件,判断正确
urgency    = 1.45/2                ← "refund today or we cancel"
refund     = noul 0.8592           ← 确实在要退款
usage.output_tokens = 0            ← 纯 System-1

P50 ≈ 525ms | P95 ≈ 630ms | min ≈ 474ms(20 轮 × 3 连跑,exit=0 稳定)
```

multilingual 322M 实测(2026-07-31,同机 CPU EP):

```
中文工单「发票被重复扣款了,请立刻退款,否则投诉」:
department = billing  P=0.6284    ✓
refund     = noul 0.998           ✓(English checkpoint 反转为 0.097,证明必须用 multilingual)
urgent     = 1.0041/2             ✓(尽快处理)

ONNX↔PyTorch parity:max|dlogits| = 6.2e-06,max|dact| = 0.0(external_data=False 单文件导出)
串行 P50 = 122.7ms | min = 122.2ms(比 English 421M 快 4 倍+)
压测:200 iter × 4 并发,deterministic PASS,0 errors,5.9 qps
```

### 6.1 实施结果(2026-07-31 全量落地)

- **仓库整合**:laya-engine / laya-cli / laya-server 三个 crate 并入 `src-tauri/crates/`,
  插件 `plugins/decision-laya`(Host bundle)随仓库走,共用一个 git。
- **模型资产**:`models/laya-multilingual-onnx/`(laya.onnx 1.29GB 单文件 + tokenizer/ +
  laya_config.json),`models/laya-onnx/`(English 基线);均被 `.gitignore` 忽略,导出脚本
  见 `.tmp/laya-export/`(receptron/laya `export/export_onnx.py`,可复现)。
- **sidecar**:`laya-server.exe --model-dir models/laya-multilingual-onnx --port 10230`,
  `GET /health` + `POST /predict`(serde_json 构造响应,Windows 路径反斜杠已正确转义)。
- **插件**:`@qianxun/decision-laya` 注册 `laya_decide` 工具(policy_version
  `local-v1-min-confidence`,min<0.6 → escalate hint),dev/release 两套 dsh-home profile
  均以 Junction + cordis.patch.yml install 条目方式接入。
- **e2e 链路**:`npx tsx e2e/probe-laya-decision.ts --times 10`——探针自带 vite(dev:web,
  5190)、spawn debug 二进制(CDP 10222)、`harness_start`、经 UI 点 DSH → 新会话
  (exact-match,避开侧栏同名会话)、composer 定位(placeholder 含「调用指令」)、
  发送校验(fill 读值 + Enter 后清空检查 + 发送按钮兜底)、断言 agent 回复
  `部门=<choice>,置信度=…`。默认模型若为 MiniMax(账号 429)自动经二级菜单切 GLM-5.3-Flash。
- **实测结论**:10/10 通过;agent 每轮 2 步(工具调用 + 回复),单轮 ~8s(LLM 主导),
  laya 决策本身 P50 123ms,决策平面零延迟瓶颈。

## 7. Windows 踩坑实录(全部已绕开)

1. **ort-sys 静态库 CRT 冲突**:预编译 `onnxruntime.lib` 是 `/MT`,Rust 默认 `/MD`,
   链接期 `LNK2005/LNK1169 multiply defined symbols` → `load-dynamic` 路线根治。
2. **`GraphOptimizationLevel::Level3` 在官方 1.22.x 无效**(映射 ORT_ENABLE_LAYOUT)
   → 用 `All`。
3. **进程退出 fail-fast(0xC0000409)**:ORT 全局线程池销毁竞态,且**非确定性**
   (同一二进制时崩时不崩)→ 输出后显式退出 / 泄漏 Session。
4. **ort-sys `copy_dylibs` 目录假设**:全新 workspace 缺 `target\debug\build\{examples,deps}`
   时 `fs::copy` 直接 panic(符号链接需要 Windows 开发者模式)→ load-dynamic 不触发。
5. **HF 大文件下载易断**(schannel 56):`curl -C -` 循环续传到字节数精确匹配。
6. **PS 5.1 不识别 LF 行尾 here-string**、中文注释在 ANSI 解码下破坏语法 →
   构建脚本纯 ASCII / 逻辑放独立 .py。

## 8. 与既有架构文档的映射

| 领域文档(docs 01-07) | 本设计的落点 |
|---|---|
| 02 Laya Decision Plane | §1 定位与红线;Tool 只产 Decision + policy_hint |
| 04 Zero-Fork 插件 | §3.4 bundle 形态;不改 packages/core / Agent Loop |
| 05 Runtime & Deployment | §2 端口与 127.0.0.1 边界;`Laya: local only, no network` |
| 06 Native Capability Bridge | 未来 laya-server 由 Rust supervisor 拉起;插件→HTTP→Rust 不越级 |
| 07 Decision Contract | `{decision_id, task_id, kind, choice, confidence, policy_version, model, timestamp}` 由插件层补齐包装 |
| 07 Phase 2 | "先 standalone benchmark,再 plugin"——本文 §6 顺序即此 |

## 9. 路线图

- [x] **Phase 0** Rust 引擎 + English bundle 本地跑通 + 基线延迟
- [ ] **Phase 1** multilingual 导出 + 中文验证 + 压测报告 ← 进行中
- [ ] **Phase 2** laya-server sidecar + decision-laya dsh 插件
- [ ] **Phase 3** CDP e2e:debug 千寻 + Playwright 10 次直访 DSH 验证
- [ ] **Phase 4** 自建 Decision Benchmark(local/cloud、retry/stop、escalation 验收)
- [ ] **Phase 5** Qianxun supervisor 托管 laya-server 生命周期 + 打包分发
