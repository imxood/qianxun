//! 子进程输出 的有界逐行读取。
//!
//! DSH 与 npm 的输出都是不可信输入：一行没有换行符的输出若不设上限，
//! 就能让千寻内存无限增长。这里把单行截断在 32KB，超长部分丢弃但
//! 继续排水（不阻塞子进程退出），并保证下一行不受污染。

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

const MAX_LINE_BYTES: usize = 32 * 1024;
const TRUNCATED: &[u8] = b" ... [line truncated]";

/// 读一行；返回 false 表示流结束。超长行被截断并标注。
pub async fn next_line<R>(reader: &mut R, line: &mut Vec<u8>) -> std::io::Result<bool>
where
    R: AsyncBufRead + Unpin,
{
    line.clear();
    let content_limit = MAX_LINE_BYTES - TRUNCATED.len() - 1;
    let mut truncated = false;
    let mut saw_bytes = false;

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if truncated {
                line.extend_from_slice(TRUNCATED);
            }
            return Ok(saw_bytes);
        }

        saw_bytes = true;
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |at| at + 1);
        let room = content_limit.saturating_sub(line.len());
        let kept = end.min(room);
        line.extend_from_slice(&available[..kept]);
        truncated |= kept < end;
        let complete = available[..end].ends_with(b"\n");
        reader.consume(end);

        if complete {
            if truncated {
                // 截断行末尾去掉残缺的回车换行再补标注，保证标注可见。
                while line
                    .last()
                    .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
                {
                    line.pop();
                }
                line.extend_from_slice(TRUNCATED);
                line.push(b'\n');
            }
            return Ok(true);
        }
    }
}

/// 有上限地整体读取一段输出：保留前 `maximum` 字节，其余只排水。
///
/// 排水让子进程不至于卡在写满的管道上而无法退出，但多出的不可信
/// 字节不再进入内存。
/// （Node 二进制下载校验即将使用；先行落地保持与探测工具同族。）
#[allow(dead_code)]
pub async fn capture<R>(mut reader: R, maximum: usize) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut body = Vec::with_capacity(maximum.min(64 * 1024));
    let mut limited = tokio::io::AsyncReadExt::take(&mut reader, maximum.saturating_add(1) as u64);
    limited.read_to_end(&mut body).await?;
    if body.len() <= maximum {
        return Ok(body);
    }

    tokio::io::copy(&mut reader, &mut tokio::io::sink()).await?;
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("child output exceeds the {maximum} byte safety limit"),
    ))
}

/// `capture` 的同步版：给少数不能进入异步上下文的一次性探测用。
pub fn capture_sync<R>(mut reader: R, maximum: usize) -> std::io::Result<Vec<u8>>
where
    R: std::io::Read,
{
    let mut body = Vec::with_capacity(maximum.min(64 * 1024));
    let mut limited = std::io::Read::take(&mut reader, maximum.saturating_add(1) as u64);
    std::io::Read::read_to_end(&mut limited, &mut body)?;
    if body.len() <= maximum {
        return Ok(body);
    }

    std::io::copy(&mut reader, &mut std::io::sink())?;
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("child output exceeds the {maximum} byte safety limit"),
    ))
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    #[tokio::test]
    async fn 超长行被截断且不吞掉下一行() {
        let mut input = vec![b'x'; MAX_LINE_BYTES * 2];
        input.extend_from_slice(b"\nnext\n");
        let mut reader = BufReader::new(input.as_slice());
        let mut line = Vec::new();

        assert!(next_line(&mut reader, &mut line).await.expect("first line"));
        assert!(line.len() <= MAX_LINE_BYTES);
        assert!(line.ends_with(b"[line truncated]\n"));
        assert!(next_line(&mut reader, &mut line)
            .await
            .expect("second line"));
        assert_eq!(line, b"next\n");
        assert!(!next_line(&mut reader, &mut line).await.expect("end"));
    }

    #[tokio::test]
    async fn 无换行的末行与空输入各报告一次() {
        let mut reader = BufReader::new(b"last".as_slice());
        let mut line = Vec::new();
        assert!(next_line(&mut reader, &mut line).await.expect("last line"));
        assert_eq!(line, b"last");
        assert!(!next_line(&mut reader, &mut line).await.expect("end"));
    }

    #[tokio::test]
    async fn capture在上限内接受超限则报错并排水() {
        assert_eq!(capture(b"12345".as_slice(), 5).await.unwrap(), b"12345");
        let failure = capture(b"123456789".as_slice(), 5)
            .await
            .expect_err("oversized output");
        assert_eq!(failure.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn 同步capture同样执行上限() {
        assert_eq!(capture_sync(b"12345".as_slice(), 5).unwrap(), b"12345");
        assert_eq!(
            capture_sync(b"123456".as_slice(), 5).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
