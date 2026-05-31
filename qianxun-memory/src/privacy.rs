use regex::Regex;

/// 对工具输出进行脱敏处理。
pub fn strip_private_data(text: &str) -> String {
    let mut result = text.to_string();

    // API Keys: sk-xxx, pk-xxx, ghp_xxx
    if let Ok(re) = Regex::new(r#"(?i)(sk-|pk-|ghp_|gho_|ghu_|ghs_|ghr_)[a-z0-9]{20,}"#) {
        result = re.replace_all(&result, "[REDACTED_API_KEY]").to_string();
    }

    // JWT: eyJxxx.yyy.zzz
    if let Ok(re) = Regex::new(r"eyJ[a-zA-Z0-9_-]+\.eyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+") {
        result = re.replace_all(&result, "[REDACTED_JWT]").to_string();
    }

    // 连接字符串密码: postgres://user:password@host
    if let Ok(re) = Regex::new(r#"(postgres|mysql|redis|mongodb)://[^:]+:([^@]+)@"#) {
        result = re.replace_all(&result, "${1}://[REDACTED]:[REDACTED]@").to_string();
    }

    // AWS 密钥: AKIAxxxxxxxx
    if let Ok(re) = Regex::new(r"AKIA[0-9A-Z]{16}") {
        result = re.replace_all(&result, "[REDACTED_AWS_KEY]").to_string();
    }

    // 私钥块
    if let Ok(re) = Regex::new(r"-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----") {
        result = re.replace_all(&result, "[REDACTED_PRIVATE_KEY]").to_string();
    }

    // 环境变量中的 API Key
    if let Ok(re) = Regex::new(r#"(?i)(api_key|api_secret|access_key|secret_key|token)\s*[:=]\s*['\"]?[a-z0-9_\-]{16,}['\"]?"#) {
        result = re.replace_all(&result, "${1}=[REDACTED]").to_string();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_api_key() {
        let input = "api key is sk-abc123def456ghi789jklmno";
        let result = strip_private_data(input);
        assert!(result.contains("[REDACTED_API_KEY]"));
        assert!(!result.contains("sk-abc123"));
    }

    #[test]
    fn test_strip_jwt() {
        let input = "token: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3j4g8g8y8y8y8y8y8y8y8y8y8y8";
        let result = strip_private_data(input);
        assert!(result.contains("[REDACTED_JWT]"));
    }

    #[test]
    fn test_strip_connection_string() {
        let input = "postgres://user:secret123@localhost:5432/db";
        let result = strip_private_data(input);
        assert!(!result.contains("secret123"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_strip_env_var() {
        let input = "export API_KEY=sk-abc123def456ghi789j";
        let result = strip_private_data(input);
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_clean_text_unchanged() {
        let input = "普通文本不包含敏感信息";
        let result = strip_private_data(input);
        assert_eq!(result, input);
    }
}
