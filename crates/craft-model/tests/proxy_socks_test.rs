//! socks 代理接线集成测试（D14）：
//! 走生产模型调用的真实出口（craft_model::net::build_http_client → apply_reqwest，
//! reqwest feature 组合 rustls-tls + socks 与 craft-model 一致），用本地 std TcpListener
//! 实现的最小 SOCKS5 mock 服务器断言握手真实走通：
//! greeting（无鉴权）→ CONNECT 请求（代理侧收到目标地址）→ 成功回包 → 应用层 HTTP 请求经隧道应答。
//!
//! 位置说明：不放在 craft-core——reqwest 的内置 socks 连接器要先经 HttpConnector 连代理，
//! 该 HttpConnector 仅在启用 TLS feature 的连接器构造路径放开非 http scheme（enforce_http
//! 关闭）；craft-core 单独测试时无 TLS feature，socks 必然失败。目标 host 用 RFC 5737
//! 文档网段（203.0.113.1，永不经真实网络）；socks5://（本地解析）配 IP 目标不触发 DNS。
//! 留给真实环境手工验收的部分：真实远端代理（凭据鉴权、socks5h 远端 DNS 解析）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_core::network::{normalize_proxy_url, ProxyConfig, ProxyMode};

const RESPONSE_BODY: &str = "craftwork-socks-ok";
/// 文档保留网段（RFC 5737 TEST-NET-3）：若 socks 链路失效而直连该地址，连接快速失败。
const TARGET_HOST: &str = "203.0.113.1";

/// custom 模式配置（地址按生产路径经 normalize 归一，覆盖 socks5:// 无协议头补齐场景）。
fn socks5_config(port: u16) -> ProxyConfig {
    ProxyConfig {
        mode: ProxyMode::Custom,
        effective_url: Some(normalize_proxy_url(&format!("socks5://127.0.0.1:{port}")).expect("socks5 地址应合法")),
    }
}

/// 最小 SOCKS5 mock 服务器：无鉴权握手 → 记录 CONNECT 目标地址 → 回成功 →
/// 读客户端 HTTP 请求头 → 回固定 200 响应。
struct Socks5Server {
    port: u16,
    targets: Arc<Mutex<Vec<String>>>,
}

impl Socks5Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        let port = listener.local_addr().expect("读取本地端口失败").port();
        let targets = Arc::new(Mutex::new(Vec::new()));
        let targets_clone = Arc::clone(&targets);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve_socks5(stream, &targets_clone);
            }
        });
        Self { port, targets }
    }

    fn recorded_targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }
}

fn serve_socks5(mut stream: TcpStream, targets: &Mutex<Vec<String>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    // 握手问候：VER(1) + NMETHODS(1) + METHODS(N)，无鉴权回 0x05 0x00
    let mut greeting = [0u8; 2];
    stream.read_exact(&mut greeting)?;
    let mut methods = vec![0u8; greeting[1] as usize];
    stream.read_exact(&mut methods)?;
    stream.write_all(&[0x05, 0x00])?;
    // CONNECT 请求：VER(1) CMD(1) RSV(1) ATYP(1) ADDR(N) PORT(2)
    let mut head = [0u8; 4];
    stream.read_exact(&mut head)?;
    let host = match head[3] {
        0x01 => {
            let mut addr = [0u8; 4];
            stream.read_exact(&mut addr)?;
            addr.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(".")
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len)?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain)?;
            String::from_utf8_lossy(&domain).to_string()
        }
        0x04 => {
            let mut addr = [0u8; 16];
            stream.read_exact(&mut addr)?;
            format!("{addr:?}")
        }
        other => format!("atyp-{other}"),
    };
    let mut port_bytes = [0u8; 2];
    stream.read_exact(&mut port_bytes)?;
    targets.lock().unwrap().push(format!("{host}:{}", u16::from_be_bytes(port_bytes)));
    // 回成功（BIND.ADDR 全零占位），随后客户端发送应用层请求
    stream.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])?;
    stream.flush()?;
    // 读客户端的 HTTP 请求头（读到 \r\n\r\n），再回固定响应。
    // 注意必须「先入缓冲再判终止」：若请求头一次读入后先判终止，会永远等下一次读，
    // 与等响应的客户端形成互等死锁（实测 5 秒超时挂死）。
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let body = RESPONSE_BODY;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// socks5 自定义代理：握手真实走通，代理侧记录到 CONNECT 的目标地址。
#[tokio::test]
async fn socks5_custom_proxy_completes_handshake_and_tunnels_request() {
    let socks = Socks5Server::start();
    let client = craft_model::net::build_http_client(Some(&socks5_config(socks.port)));
    let resp = client
        .get(format!("http://{TARGET_HOST}/tunneled"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("socks5 代理请求应握手并应答成功");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.expect("读响应体失败"), RESPONSE_BODY);
    assert_eq!(
        socks.recorded_targets(),
        vec![format!("{TARGET_HOST}:80")],
        "代理应收到 CONNECT 目标地址"
    );
}
