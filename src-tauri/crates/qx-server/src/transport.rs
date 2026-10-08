//! 载体归一（docs/19 §12.2）：把三种承载（axum WS / tungstenite 客户端 / 测试对）
//! 泵成同一组「控制通道 + 字节流」。
//!
//! 架构教训（2026-10-09 排障记录）：自研「消息→字节流」投影（BytePipe + 封包 codec）
//! 在 ≥33 条并发流下出现提前 FIN/事务中断——**字节流语义必须交给 tokio duplex**
//! （yamux 直接 `.compat()`，40 并发流实测无恙），WS 侧只做忠实的消息↔字节泵 + ctrl 分流。

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::protocol::Frame;

/// 单个 WS Binary 消息承载的字节上限（docs/19 §4.2 帧尺寸纪律）。
pub(crate) const MAX_CHUNK: usize = 256 * 1024;

/// 一条隧道的进程内投影：控制帧走 mpsc，yamux 字节走 duplex。
#[derive(Debug)]
pub struct TunnelChannels {
    /// 对端 → 本端：控制帧。
    pub ctrl_rx: mpsc::Receiver<Frame>,
    /// 本端 → 对端：控制帧。
    pub ctrl_tx: mpsc::Sender<Frame>,
    /// yamux 的字节流载体（tokio io；`Connection::new(io.compat(), …)`）。
    pub io: tokio::io::DuplexStream,
}

/// axum WS（服务端 /qx-ws 入口）→ 隧道频道。
///
/// **单任务泵**：socket、duplex、ctrl 同属一个任务——任一方向断即整线死亡
/// （socket 与 duplex 一起释放），杜绝「半开泵持有 socket 等对端」的循环等待
/// （2026-10-09 抢占排障教训）。
pub fn from_axum_ws(socket: axum::extract::ws::WebSocket) -> TunnelChannels {
    let (ctrl_to_wire, mut wire_ctrl_rx) = mpsc::channel::<Frame>(32);
    let (in_ctrl_tx, ctrl_rx) = mpsc::channel::<Frame>(32);
    let (io, peer_io) = tokio::io::duplex(64 * 1024);
    let (mut io_read, mut io_write) = tokio::io::split(io);
    let (mut sink, mut source) = socket.split();

    tokio::spawn(async move {
        let in_ctrl_tx = in_ctrl_tx;
        let mut buf = vec![0u8; MAX_CHUNK];
        loop {
            tokio::select! {
                incoming = source.next() => {
                    let Some(Ok(message)) = incoming else { break };
                    match message {
                        axum::extract::ws::Message::Text(text) => match Frame::decode(text.as_str()) {
                            Ok(frame) => {
                                if in_ctrl_tx.send(frame).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break, // 线上垃圾：协议违规断线
                        },
                        axum::extract::ws::Message::Binary(bytes) => {
                            match io_write.write_all(&bytes).await {
                                Ok(()) => {}
                                Err(_) => break,
                            }
                        }
                        axum::extract::ws::Message::Ping(payload) => {
                            if sink.send(axum::extract::ws::Message::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                frame = wire_ctrl_rx.recv() => {
                    let Some(frame) = frame else { break };
                    if sink.send(axum::extract::ws::Message::Text(frame.encode().into())).await.is_err() {
                        break;
                    }
                }
                readable = io_read.read(&mut buf) => {
                    match readable {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sink
                                .send(axum::extract::ws::Message::Binary(buf[..n].to_vec().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    TunnelChannels {
        ctrl_rx,
        ctrl_tx: ctrl_to_wire,
        io: peer_io,
    }
}

/// tungstenite 客户端（千寻出站连接）→ 隧道频道。
pub async fn connect(url: &str) -> Result<TunnelChannels, String> {
    let (stream, _response) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|cause| format!("连接 {url} 失败：{cause}"))?;
    Ok(from_tungstenite(stream))
}

/// tungstenite 流 → 隧道频道（同样是单任务泵，生命周期同 from_axum_ws）。
pub fn from_tungstenite(
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> TunnelChannels {
    use tokio_tungstenite::tungstenite::Message as Ws;
    let (ctrl_to_wire, mut wire_ctrl_rx) = mpsc::channel::<Frame>(32);
    let (in_ctrl_tx, ctrl_rx) = mpsc::channel::<Frame>(32);
    let (io, peer_io) = tokio::io::duplex(64 * 1024);
    let (mut io_read, mut io_write) = tokio::io::split(io);
    let (mut sink, mut source) = stream.split();

    tokio::spawn(async move {
        let in_ctrl_tx = in_ctrl_tx;
        let mut buf = vec![0u8; MAX_CHUNK];
        loop {
            tokio::select! {
                incoming = source.next() => {
                    let Some(Ok(message)) = incoming else { break };
                    match message {
                        Ws::Text(text) => match Frame::decode(&text) {
                            Ok(frame) => {
                                if in_ctrl_tx.send(frame).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        },
                        Ws::Binary(bytes) => match io_write.write_all(&bytes).await {
                            Ok(()) => {}
                            Err(_) => break,
                        },
                        _ => {} // Ping 由 tungstenite 自动应答
                    }
                }
                frame = wire_ctrl_rx.recv() => {
                    let Some(frame) = frame else { break };
                    if sink.send(Ws::Text(frame.encode().into())).await.is_err() {
                        break;
                    }
                }
                readable = io_read.read(&mut buf) => {
                    match readable {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sink.send(Ws::Binary(buf[..n].to_vec().into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    TunnelChannels {
        ctrl_rx,
        ctrl_tx: ctrl_to_wire,
        io: peer_io,
    }
}

/// Duplex 测试对：数据面直连；ctrl 走独立的封包小通道（不起端口跑全量协议）。
pub fn duplex_pair() -> (TunnelChannels, TunnelChannels) {
    let (a_io, b_io) = tokio::io::duplex(64 * 1024);
    let (a_ctrl_tx, a_ctrl_rx, b_ctrl_tx, b_ctrl_rx) = ctrl_pair();
    (
        TunnelChannels {
            ctrl_rx: a_ctrl_rx,
            ctrl_tx: a_ctrl_tx,
            io: a_io,
        },
        TunnelChannels {
            ctrl_rx: b_ctrl_rx,
            ctrl_tx: b_ctrl_tx,
            io: b_io,
        },
    )
}

/// ctrl 互通：两个方向各一条 mpsc（测试载体的简化——不走封包）。
fn ctrl_pair() -> (
    mpsc::Sender<Frame>,
    mpsc::Receiver<Frame>,
    mpsc::Sender<Frame>,
    mpsc::Receiver<Frame>,
) {
    let (a_to_b, b_rx) = mpsc::channel::<Frame>(32);
    let (b_to_a, a_rx) = mpsc::channel::<Frame>(32);
    (a_to_b.clone(), a_rx, b_to_a, b_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn duplex对控制帧互通() {
        let (mut a, mut b) = duplex_pair();
        a.ctrl_tx.send(Frame::Ping).await.expect("发送");
        assert_eq!(b.ctrl_rx.recv().await.expect("接收"), Frame::Ping);
        b.ctrl_tx.send(Frame::Pong).await.expect("发送");
        assert_eq!(a.ctrl_rx.recv().await.expect("接收"), Frame::Pong);
    }

    #[tokio::test]
    async fn duplex对字节流互通() {
        let (a, b) = duplex_pair();
        let (mut a_io, mut b_io) = (a.io, b.io);
        let payload = vec![7u8; 300 * 1024];
        let expected = payload.clone();
        let writer = tokio::spawn(async move {
            a_io.write_all(&payload).await.expect("写");
        });
        let mut got = vec![0u8; expected.len()];
        b_io.read_exact(&mut got).await.expect("读");
        writer.await.expect("任务");
        assert_eq!(got, expected);
    }
}
