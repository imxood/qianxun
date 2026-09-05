//! 识别 `dsh web` 开始监听时打印的就绪行。
//!
//! DSH 在 stdout 打出 `dsh web: http://127.0.0.1:17300/?token=…`
//! （0.1.2 起 URL 携带进程启动 token）。这个字符串
//! 决定千寻把 WebView 指向哪里，所以必须校验而不是照单全收：
//! 被托管的子进程不能靠打印一行输出就把外壳引向任意地址。
//! scheme/回环/显式端口的校验不放宽；query 里的 token 原样保留，
//! 供桌面 WebView 与远程网关做登录兑换。

/// 就绪行的前缀标记。
const READY_PREFIX: &str = "dsh web: ";

/// 检查一行输出的结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Ready {
    /// 这一行宣布了一个可用的回环地址（完整 URL，可能带 `?token=`）。
    At(String),
    /// 这一行宣布了千寻拒绝加载的东西。
    Rejected(String),
}

/// 从一行 stdout 提取回环 URL；普通日志行返回 None。
pub fn parse(line: &str) -> Option<Ready> {
    let announced = line.trim_end().strip_prefix(READY_PREFIX)?;
    // 就绪行是一个裸 URL；空白之后的内容是注释。
    let candidate = announced.split_whitespace().next().unwrap_or_default();

    let Ok(url) = url::Url::parse(candidate) else {
        return Some(Ready::Rejected(format!(
            "DSH 报告了无法解析的地址：{candidate}"
        )));
    };

    let is_loopback = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"));
    if url.scheme() != "http" || !is_loopback {
        return Some(Ready::Rejected(format!(
            "DSH 报告了非回环地址：{candidate}"
        )));
    }
    if url.port().is_none() {
        return Some(Ready::Rejected(format!(
            "DSH 报告的地址缺少显式端口：{candidate}"
        )));
    }

    // 完整 URL 原样上交（含 query）；展示用 origin、鉴权用 token，
    // 由 supervisor 经 origin_of/token_of 拆分。
    Some(Ready::At(url.to_string()))
}

/// 从就绪 URL 里取 origin（scheme://host:port，无路径无 query）。
/// 状态机对外只暴露 origin，token 单独走 [`token_of`]，避免凭据
/// 混进展示与遥测字段。
pub fn origin_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .map(|parsed| parsed.origin().ascii_serialization())
}

/// 从就绪 URL 里取 `?token=` 的值（旧版 DSH 没有 token 时返回 None）。
pub fn token_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(name, _)| name == "token")
        .map(|(_, value)| value.into_owned())
}

/// 从就绪 origin 里取端口号，用于固定端口一致性校验（ADR-002）。
pub fn port_of(origin: &str) -> Option<u16> {
    url::Url::parse(origin).ok()?.port()
}

#[cfg(test)]
mod tests {
    use super::{origin_of, parse, port_of, token_of, Ready};

    #[test]
    fn 接受dsh实际打印的格式() {
        assert_eq!(
            parse("dsh web: http://127.0.0.1:17300"),
            Some(Ready::At("http://127.0.0.1:17300/".into()))
        );
    }

    #[test]
    fn 接受携带启动token的格式且完整保留() {
        let url = "http://127.0.0.1:17300/?token=rDZi5RscXXBARf98sSqQf5ultgjYQ2M14eGXq436gEw";
        assert_eq!(parse(&format!("dsh web: {url}")), Some(Ready::At(url.into())));
        assert_eq!(token_of(url).as_deref(), Some("rDZi5RscXXBARf98sSqQf5ultgjYQ2M14eGXq436gEw"));
        assert_eq!(
            origin_of(url).as_deref(),
            Some("http://127.0.0.1:17300")
        );
    }

    #[test]
    fn 旧版无token时token_of为空() {
        assert_eq!(token_of("http://127.0.0.1:17300/"), None);
    }

    #[test]
    fn origin_of剥掉路径与query() {
        assert_eq!(
            origin_of("http://localhost:3080/sub/page?token=x&keep=1").as_deref(),
            Some("http://localhost:3080")
        );
        assert_eq!(origin_of("not-a-url"), None);
    }

    #[test]
    fn 容忍结尾空白与回车() {
        assert_eq!(
            parse("dsh web: http://localhost:3080/\r\n"),
            Some(Ready::At("http://localhost:3080/".into()))
        );
    }

    #[test]
    fn 普通日志行被忽略() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("loading plugin dsh-tool-bash"), None);
        assert_eq!(parse("  dsh web: http://127.0.0.1:1"), None);
    }

    #[test]
    fn 拒绝被引向非回环地址() {
        assert!(matches!(
            parse("dsh web: http://example.com:80"),
            Some(Ready::Rejected(_))
        ));
        assert!(matches!(
            parse("dsh web: https://127.0.0.1:443"),
            Some(Ready::Rejected(_))
        ));
        assert!(matches!(
            parse("dsh web: file:///etc/passwd"),
            Some(Ready::Rejected(_))
        ));
    }

    #[test]
    fn 必须带显式端口() {
        assert!(matches!(
            parse("dsh web: http://127.0.0.1"),
            Some(Ready::Rejected(_))
        ));
    }

    #[test]
    fn 垃圾内容被拒绝() {
        assert!(matches!(
            parse("dsh web: not-a-url"),
            Some(Ready::Rejected(_))
        ));
    }

    #[test]
    fn 从origin提取端口() {
        assert_eq!(port_of("http://127.0.0.1:17300"), Some(17300));
        assert_eq!(port_of("http://localhost:9"), Some(9));
        assert_eq!(port_of("not-a-url"), None);
    }
}
