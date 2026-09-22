//! `laya_config.json`:上下文预算与校准温度。
//!
//! 对齐 laya-mlx `common.py`:检查点自带的拟合温度可能低于 1(锐化 logits),
//! `choice:11+` 桶曾出现 0.1006(≈10 倍锐化),会把接近随机的答案报告成近乎确定。
//! 因此与 laya-mlx 相同:保留原始值供检查,应用前一律钳制到 `[0.5, 5.0]`。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const TEMP_MIN: f64 = 0.5;
pub const TEMP_MAX: f64 = 5.0;

#[derive(Debug, Clone, Deserialize)]
struct RawLayaConfig {
    #[serde(default = "default_max_len")]
    max_len: usize,
    #[serde(default = "default_head_max_len")]
    head_max_len: usize,
    #[serde(default)]
    temperature: Option<[f64; 3]>,
    #[serde(default)]
    temperature_by_options: Option<BTreeMap<String, f64>>,
}

fn default_max_len() -> usize {
    512
}
fn default_head_max_len() -> usize {
    192
}

#[derive(Debug, Clone)]
pub struct LayaConfig {
    pub max_len: usize,
    pub head_max_len: usize,
    /// [choice, score, noul],已钳制。
    pub temperature: [f64; 3],
    /// 形如 "choice:11+" 的按选项数分桶温度,已钳制。
    pub temperature_by_options: BTreeMap<String, f64>,
    /// 加载时被钳制掉的原始值(调试/审计用)。
    pub clamped: Vec<String>,
}

impl LayaConfig {
    pub fn load(dir: &Path) -> crate::Result<Self> {
        let path = dir.join("laya_config.json");
        let raw: RawLayaConfig = if path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&path)?)?
        } else {
            serde_json::from_str("{}")?
        };
        Self::from_raw(raw)
    }

    fn from_raw(raw: RawLayaConfig) -> crate::Result<Self> {
        let mut clamped = Vec::new();
        let mut temperature = raw.temperature.unwrap_or([1.0; 3]);
        for (i, t) in temperature.iter_mut().enumerate() {
            let c = clamp_temperature(*t);
            if c != *t {
                clamped.push(format!("temperature[{i}]={t:.4} -> {c:.4}"));
            }
            *t = c;
        }
        let mut temperature_by_options = BTreeMap::new();
        for (k, v) in raw.temperature_by_options.unwrap_or_default() {
            let c = clamp_temperature(v);
            if c != v {
                clamped.push(format!("{k}={v:.4} -> {c:.4}"));
            }
            temperature_by_options.insert(k, c);
        }
        Ok(Self {
            max_len: raw.max_len,
            head_max_len: raw.head_max_len,
            temperature,
            temperature_by_options,
            clamped,
        })
    }

    /// 查某题型、k 个选项时应使用的温度(laya-mlx `temp_bucket` + 兜底)。
    pub fn temperature_for(&self, kind: QuestionKindAsInt, options: usize) -> f64 {
        let bucket = temp_bucket(kind, options);
        self.temperature_by_options
            .get(&bucket)
            .copied()
            .unwrap_or(self.temperature[kind.0 as usize])
    }
}

/// 与 Python 侧 `QTYPES` 对齐的 0/1/2 编号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestionKindAsInt(pub u8);

pub fn clamp_temperature(t: f64) -> f64 {
    if !t.is_finite() {
        return 1.0;
    }
    t.clamp(TEMP_MIN, TEMP_MAX)
}

/// laya-mlx `common.temp_bucket`:"choice:3-5" 等。
pub fn temp_bucket(kind: QuestionKindAsInt, k: usize) -> String {
    let name = match kind.0 {
        0 => "choice",
        1 => "score",
        _ => "noul",
    };
    let size = if k <= 2 {
        "2"
    } else if k <= 5 {
        "3-5"
    } else if k <= 10 {
        "6-10"
    } else {
        "11+"
    };
    format!("{name}:{size}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_names_match_python() {
        assert_eq!(temp_bucket(QuestionKindAsInt(0), 2), "choice:2");
        assert_eq!(temp_bucket(QuestionKindAsInt(0), 3), "choice:3-5");
        assert_eq!(temp_bucket(QuestionKindAsInt(1), 5), "score:3-5");
        assert_eq!(temp_bucket(QuestionKindAsInt(1), 10), "score:6-10");
        assert_eq!(temp_bucket(QuestionKindAsInt(2), 11), "noul:11+");
    }

    #[test]
    fn clamp_sharpens_off() {
        assert_eq!(clamp_temperature(0.1006), 0.5);
        assert_eq!(clamp_temperature(99.0), 5.0);
        assert_eq!(clamp_temperature(f64::NAN), 1.0);
        assert_eq!(clamp_temperature(1.0), 1.0);
    }

    #[test]
    fn parse_minimal_config() {
        let cfg = LayaConfig::from_raw(serde_json::from_str("{}").unwrap()).unwrap();
        assert_eq!(cfg.max_len, 512);
        assert_eq!(cfg.head_max_len, 192);
        assert_eq!(cfg.temperature, [1.0, 1.0, 1.0]);
    }
}
