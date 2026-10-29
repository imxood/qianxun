//! ONNX Runtime 会话管理(ort 2.0.0-rc.13 实测 API)。
//!
//! 模式沿用 egui_hikvision_app 的 `OnnxRunner`:懒加载 + Mutex 保护 + 预设构造器。
//! 引擎选 onnxruntime(`ort` crate)而非 OpenCV DNN:Laya 是 ModernBERT transformer
//! (int64/bool 多输入、动态序列长度),不是 CNN 图像 blob;onnxruntime 与
//! @receptron/laya 同引擎,数值语义可直接对齐,且可插 DirectML/CUDA EP。
//!
//! rc.13 API 要点(与 rc.9 教程示例不同):
//! - EP 类型是 `ort::ep::CPU` / `ort::ep::DirectML`(经 `ort::execution_providers` 亦可);
//! - builder 各方法返回 typed error(`ort::Error<SessionBuilder>`),需逐点 map_err;
//! - `Session::run` 需要 `&mut self`;
//! - `outputs["name"].try_extract_tensor::<T>()` 返回 `(&Shape, &[T])`。

use crate::error::{LayaError, Result};
use ort::ep::CPU;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn ort_err<E: std::fmt::Display>(e: E) -> LayaError {
    LayaError::Ort(e.to_string())
}

/// load-dynamic 模式下,首次使用 ORT 前必须定位 onnxruntime.dll。
/// 依次探测:环境变量 ORT_DYLIB_PATH → exe 目录 → 从 exe 与 CWD 向上逐级找
/// `vendor\onnxruntime\onnxruntime.dll`。找到后写回 ORT_DYLIB_PATH(进程级,只影响首次加载)。
pub fn ensure_ort_dylib() -> Result<()> {
    if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|v| !v.is_empty()) {
        return Ok(());
    }
    const REL: [&str; 3] = ["vendor", "onnxruntime", "onnxruntime.dll"];

    fn with_dll_name(dir: &Path) -> Option<PathBuf> {
        let direct = dir.join("onnxruntime.dll");
        if direct.is_file() {
            return Some(direct);
        }
        let vendored = dir.join(REL.iter().collect::<PathBuf>());
        if vendored.is_file() {
            return Some(vendored);
        }
        None
    }

    fn walk_up(start: &Path) -> Option<PathBuf> {
        let mut dir = Some(start.to_path_buf());
        for _ in 0..10 {
            let Some(d) = dir else { break };
            if let Some(found) = with_dll_name(&d) {
                return Some(found);
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        None
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    let found = candidates
        .iter()
        .find_map(|dir| walk_up(dir))
        .ok_or_else(|| {
            LayaError::Ort(
                "onnxruntime.dll not found; set ORT_DYLIB_PATH or place it at vendor\\onnxruntime\\onnxruntime.dll"
                    .into(),
            )
        })?;
    std::env::set_var("ORT_DYLIB_PATH", found);
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Ep {
    Cpu,
    #[cfg(feature = "directml")]
    DirectML {
        device_id: i32,
    },
    #[cfg(feature = "cuda")]
    Cuda {
        device_id: i32,
    },
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub ep: Ep,
    /// 线程数;0 = 交给 ORT 取默认(= 物理核数)。
    /// 桌面端 sidecar 没有别的活可干,给满比留一半快。
    pub intra_threads: usize,
    /// ONNX 图文件名,receptron/laya-onnx 布局为 `laya.onnx`(+ `.data`)。
    pub model_file: String,
    /// 线程池在两次推理之间是否自旋等待。
    /// **默认 false,且这是最大的一项 CPU 优化**:ORT 的 intra-op 线程池
    /// 默认在空闲时自旋吃满 CPU,sidecar 每 250ms 才来一次请求,绝大部分
    /// 时间全花在空转上,还会和 UI 线程抢核。关掉后本机实测(见 docs/16 §4)
    /// 延迟不升反降、CPU 占用大降。
    pub intra_op_spinning: bool,
    /// 把非规格化数(denormal)当 0 处理。attention 的 softmax 会产出大量
    /// 极小值,非规格化数在 x86 上走微码路径,能慢一个数量级。
    pub flush_to_zero: bool,
    /// 用 tanh 近似 GELU。ORT 官方文档:约 10% 更快且无数值质量损失。
    pub approximate_gelu: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            ep: Ep::Cpu,
            intra_threads: 0,
            model_file: "laya.onnx".into(),
            intra_op_spinning: false,
            flush_to_zero: true,
            approximate_gelu: true,
        }
    }
}

/// 量化候选文件名(按优先级)。
///
/// 背景(docs/16 §4):fp32 的 322M 模型在本机实测 ~0.8ms/token,游戏态
/// ~200 token 就是 180ms,刚好卡在 System 1 的时间预算上。int8 把内存
/// 带宽压力降到 1/4,配合 VNNI(本机 14900HX 有)是最大的单点提速。
///
/// **per-tensor,不 per-channel**:Laya v0.3.23 实测 per-channel 量化把
/// 输出一致性压到 32%,官方已回退成 per-tensor。上线前必须跑 parity 比对。
pub const QUANTIZED_CANDIDATES: [&str; 3] = ["laya.int8.onnx", "laya.q8.onnx", "laya.quant.onnx"];

/// 默认 fp32 图名。它是**兜底**而不是显式指定,所以量化版要压过它。
pub const DEFAULT_FP32_NAME: &str = "laya.onnx";

/// 在目录里挑一个可用的 ONNX 图。
///
/// 优先级:
/// 1. 调用方**显式**指定的名字(≠ `DEFAULT_FP32_NAME`)存在 → 用它;
/// 2. 量化候选按 `QUANTIZED_CANDIDATES` 顺序第一个存在的;
/// 3. `DEFAULT_FP32_NAME`;
/// 4. 都没有 → 报错并指出期望的名字。
///
/// 纯函数,不碰 ORT —— 便于单测锁定优先级顺序。
pub fn resolve_model_file(dir: &Path, preferred: &str) -> Result<String> {
    if preferred != DEFAULT_FP32_NAME && dir.join(preferred).is_file() {
        return Ok(preferred.to_owned());
    }
    for c in QUANTIZED_CANDIDATES {
        if dir.join(c).is_file() {
            return Ok(c.to_owned());
        }
    }
    if dir.join(DEFAULT_FP32_NAME).is_file() {
        return Ok(DEFAULT_FP32_NAME.to_owned());
    }
    Err(LayaError::IncompleteBundle {
        dir: dir.display().to_string(),
        missing: preferred.to_owned(),
    })
}

pub struct OrtEngine {
    model_path: PathBuf,
    options: EngineOptions,
    session: Mutex<Option<Session>>,
}

/// 一次 forward 的输入(已 collate 成 batch)。
pub struct BatchInput {
    pub input_ids: Vec<i64>,
    pub attention_mask: Vec<i64>,
    pub marker_pos: Vec<i64>,
    pub marker_mask: Vec<bool>,
    pub qtype: Vec<i64>,
    pub batch: usize,
    pub seq_len: usize,
    pub max_markers: usize,
}

/// 一次 forward 的输出。
pub struct BatchOutput {
    /// [B, K] 未校准 logits(masked 槽位 = -1e4)。
    pub logits: Vec<f32>,
    /// [B, 2] 动作头概率。
    pub act_probs: Vec<f32>,
}

impl OrtEngine {
    pub fn new(model_dir: &Path, options: EngineOptions) -> Result<Self> {
        ensure_ort_dylib()?;
        // 优先量化版(若存在),否则用调用方指定的名字
        let model_file = match resolve_model_file(model_dir, &options.model_file) {
            Ok(f) => f,
            Err(_) => options.model_file.clone(),
        };
        let model_path = model_dir.join(&model_file);
        if !model_path.exists() {
            return Err(LayaError::IncompleteBundle {
                dir: model_dir.display().to_string(),
                missing: options.model_file.clone(),
            });
        }
        let data_path = model_dir.join(format!("{}.data", model_file));
        // receptron 官方 bundle 权重外置(.data);自导出(export_onnx external_data=False)
        // 为单文件内嵌权重——图文件足够大即视为已含权重,不再硬性要求 .data。
        if !data_path.exists() {
            let graph_len = model_path.metadata().map(|m| m.len()).unwrap_or(0);
            if graph_len < 100_000_000 {
                return Err(LayaError::IncompleteBundle {
                    dir: model_dir.display().to_string(),
                    missing: format!("{}.data", model_file),
                });
            }
        }
        let options = EngineOptions {
            model_file,
            ..options
        };
        Ok(Self {
            model_path,
            options,
            session: Mutex::new(None),
        })
    }

    /// 实际使用的图文件名(fp32 还是量化版)—— 供 /health 与日志展示。
    pub fn model_file(&self) -> &str {
        &self.options.model_file
    }

    /// 已生效的线程数(0 = ORT 取物理核数)。
    pub fn intra_threads(&self) -> usize {
        self.options.intra_threads
    }

    fn ensure_loaded(&self) -> Result<std::sync::MutexGuard<'_, Option<Session>>> {
        let mut guard = self.session.lock().expect("session mutex poisoned");
        if guard.is_some() {
            return Ok(guard);
        }
        let builder = Session::builder()
            .map_err(ort_err)?
            // 注意:rc.13 的 Level3 映射 ORT_ENABLE_LAYOUT,官方 1.22.x DLL 不认;
            // All(ORT_ENABLE_ALL)全版本有效。
            .with_optimization_level(GraphOptimizationLevel::All)
            .map_err(ort_err)?
            .with_intra_threads(self.options.intra_threads)
            .map_err(ort_err)?
            // 下面三项是 CPU 侧真正的提速开关(docs/16 §4),缺任何一个都会
            // 浪费掉 20~50% 的算力:线程池空转、denormal 走微码、GELU 精算。
            .with_intra_op_spinning(self.options.intra_op_spinning)
            .map_err(ort_err)?;
        // rc.13 里这两个是无参开关(只能开不能关),要关就整段不调用
        let builder = if self.options.flush_to_zero {
            builder.with_flush_to_zero().map_err(ort_err)?
        } else {
            builder
        };
        let builder = if self.options.approximate_gelu {
            builder.with_approximate_gelu().map_err(ort_err)?
        } else {
            builder
        };
        let mut builder = match &self.options.ep {
            Ep::Cpu => builder
                .with_execution_providers([CPU::default().build()])
                .map_err(ort_err)?,
            #[cfg(feature = "directml")]
            Ep::DirectML { device_id } => {
                use ort::ep::DirectML;
                builder
                    .with_execution_providers([DirectML::default()
                        .with_device_id(*device_id)
                        .build()])
                    .map_err(ort_err)?
            }
            #[cfg(feature = "cuda")]
            Ep::Cuda { device_id } => {
                use ort::ep::CUDA;
                builder
                    .with_execution_providers([CUDA::default().with_device_id(*device_id).build()])
                    .map_err(ort_err)?
            }
        };
        let session = builder
            .commit_from_file(&self.model_path)
            .map_err(ort_err)?;
        *guard = Some(session);
        Ok(guard)
    }

    pub fn run(&self, input: &BatchInput) -> Result<BatchOutput> {
        let mut guard = self.ensure_loaded()?;
        let session = guard.as_mut().expect("session loaded");

        let outputs = session
            .run(ort::inputs![
                "input_ids" => Tensor::from_array((
                    [input.batch as i64, input.seq_len as i64],
                    input.input_ids.clone()
                ))
                .map_err(ort_err)?,
                "attention_mask" => Tensor::from_array((
                    [input.batch as i64, input.seq_len as i64],
                    input.attention_mask.clone()
                ))
                .map_err(ort_err)?,
                "marker_pos" => Tensor::from_array((
                    [input.batch as i64, input.max_markers as i64],
                    input.marker_pos.clone()
                ))
                .map_err(ort_err)?,
                "marker_mask" => Tensor::from_array((
                    [input.batch as i64, input.max_markers as i64],
                    input.marker_mask.clone()
                ))
                .map_err(ort_err)?,
                "qtype" => Tensor::from_array(([input.batch as i64], input.qtype.clone()))
                    .map_err(ort_err)?
            ])
            .map_err(ort_err)?;

        let (_, logits) = outputs["logits"]
            .try_extract_tensor::<f32>()
            .map_err(ort_err)?;
        let (_, act) = outputs["act_probs"]
            .try_extract_tensor::<f32>()
            .map_err(ort_err)?;

        let logits: Vec<f32> = logits.to_vec();
        let act: Vec<f32> = act.to_vec();

        if logits.iter().any(|v| !v.is_finite()) || act.iter().any(|v| !v.is_finite()) {
            return Err(LayaError::NonFiniteOutput);
        }

        Ok(BatchOutput {
            logits,
            act_probs: act,
        })
    }
}
