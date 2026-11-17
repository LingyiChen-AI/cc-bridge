//! Claude 模型透传必须走客户端配置的对外代理（RuntimeOverrides.outbound_proxy），
//! 不能只依赖进程环境变量：桌面客户端从开始菜单启动时没有 HTTPS_PROXY，直连会被 Anthropic 按地区拒绝（403）。
use std::time::Duration;

use claude_codex::overrides::{self, RuntimeOverrides};
use claude_codex::server::{ServerConfig, serve_with_shutdown};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn claude_passthrough_uses_configured_outbound_proxy() {
    for k in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        unsafe { std::env::remove_var(k) };
    }

    // 假的对外代理：记下收到的第一行（应为 CONNECT api.anthropic.com:443），然后回 502 结束。
    let fake = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fake_addr = fake.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let (mut sock, _) = fake.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let first = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
        let _ = sock.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
        let _ = tx.send(first);
    });

    overrides::set(RuntimeOverrides {
        outbound_proxy: Some(format!("http://{fake_addr}")),
        ..Default::default()
    });

    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(serve_with_shutdown(
        ServerConfig { bind_address: "127.0.0.1".into(), port, monitor: None },
        async move {
            let _ = stop_rx.await;
        },
    ));

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://127.0.0.1:{port}/v1/messages");
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "max_tokens": 8,
        "messages": [{"role": "user", "content": "hi"}]
    });
    for _ in 0..50 {
        let sent = client
            .post(&url)
            .header("authorization", "Bearer test-token")
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await;
        if sent.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let first = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .expect("Claude 透传没有经过配置的对外代理")
        .unwrap();
    assert!(first.starts_with("CONNECT api.anthropic.com:443"), "unexpected first line: {first}");
    let _ = stop_tx.send(());
}
