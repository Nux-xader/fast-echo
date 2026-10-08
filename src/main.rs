use socket2::{Domain, Socket, Type};
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, UdpSocket};

/// Extract Content-Length from raw HTTP headers (case-insensitive, zero-alloc).
fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.split("\r\n") {
        let mut parts = line.splitn(2, ':');
        if let (Some(name), Some(val)) = (parts.next(), parts.next())
            && name.eq_ignore_ascii_case("content-length")
        {
            return val.trim().parse().ok();
        }
    }
    None
}

/// Stream `body_data` to writer 4 times using tokio's optimized copy (8KB internal buffer).
/// Zero extra allocation — writes come directly from the existing body slice.
async fn echo_body_4x<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    body_data: &[u8],
) -> std::io::Result<()> {
    for _ in 0..4 {
        let mut chunk: &[u8] = body_data;
        tokio::io::copy(&mut chunk, writer).await?;
    }
    Ok(())
}

/// Handle UDP requests: respond with client IP regardless of payload
async fn udp_echo_loop(socket: UdpSocket) {
    let mut buf = [0u8; 64];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((_n, peer_addr)) => {
                let ip = peer_addr.ip().to_string();
                let ip = ip.strip_prefix("::ffff:").unwrap_or(&ip);
                let _ = socket.send_to(ip.as_bytes(), peer_addr).await;
            }
            Err(_) => continue,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = env::args()
        .skip_while(|arg| arg != "--addr")
        .nth(1)
        .unwrap_or_else(|| "[::]:8080".to_string());

    // Bind TCP and UDP on the same port
    let addr_parsed: SocketAddr = addr.parse()?;
    let socket = Socket::new(Domain::for_address(addr_parsed), Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr_parsed.into())?;
    socket.listen(4096)?; // Massive backlog to absorb the 300 connection spike
    let std_listener: std::net::TcpListener = socket.into();
    let listener = TcpListener::from_std(std_listener)?;
    let udp_socket = UdpSocket::bind(&addr).await?;
    println!("🚀 Ultra-Fast Hybrid Echo Server running on {}", addr);

    // Spawn UDP handler in background
    tokio::spawn(udp_echo_loop(udp_socket));

    loop {
        let (socket, peer_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        tokio::spawn(async move {
            let (mut reader, writer) = socket.into_split();
            let mut writer = BufWriter::new(writer);

            // Per-connection hard timeout: kill task after 30s regardless
            let _ = tokio::time::timeout(Duration::from_secs(30), async {
                let mut buf: Vec<u8> = Vec::with_capacity(1024);
                let mut chunk = [0u8; 1024];

                // Read initial data with 10s timeout
                let n = match tokio::time::timeout(Duration::from_secs(10), reader.read(&mut chunk))
                    .await
                {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => return,
                };
                buf.extend_from_slice(&chunk[..n]);

                let is_http = buf.starts_with(b"GET")
                    || buf.starts_with(b"POST")
                    || buf.starts_with(b"PUT")
                    || buf.starts_with(b"PATCH")
                    || buf.starts_with(b"DELETE")
                    || buf.starts_with(b"HEAD")
                    || buf.starts_with(b"OPTIONS");

                // Extract peer IP, stripping IPv4-mapped IPv6 prefix
                let ip = peer_addr.ip().to_string();
                let ip = ip.strip_prefix("::ffff:").unwrap_or(&ip);

                if is_http {
                    // Find end of headers
                    let mut hdr_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                    while hdr_end.is_none() && buf.len() < 8 * 1024 {
                        let mut chunk = [0u8; 1024];
                        let n = match tokio::time::timeout(
                            Duration::from_secs(10),
                            reader.read(&mut chunk),
                        )
                        .await
                        {
                            Ok(Ok(n)) if n > 0 => n,
                            _ => return,
                        };
                        buf.extend_from_slice(&chunk[..n]);
                        hdr_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                    }

                    if let Some(hdr_end) = hdr_end {
                        let header_str = std::str::from_utf8(&buf[..hdr_end]).unwrap_or("");
                        let body_len = parse_content_length(header_str).unwrap_or(0);

                        if body_len == 0 {
                            // No body → return client IP (original behavior)
                            let response = format!("HTTP/1.0 200 OK\r\n\r\n{}", ip);
                            let _ = tokio::time::timeout(
                                Duration::from_secs(10),
                                writer.write_all(response.as_bytes()),
                            )
                            .await;
                        } else {
                            let body_already_read = buf.len().saturating_sub(hdr_end).min(body_len);

                            // Accumulate full body: part already in buffer + remainder from socket
                            let mut body = Vec::with_capacity(body_len);
                            body.extend_from_slice(&buf[hdr_end..hdr_end + body_already_read]);

                            let remaining = body_len - body_already_read;
                            if remaining > 0 {
                                body.resize(body_len, 0);
                                let _ = tokio::time::timeout(
                                    Duration::from_secs(10),
                                    reader.read_exact(&mut body[body_already_read..]),
                                )
                                .await;
                            }

                            // Write response: headers once, body 4x
                            let _ = tokio::time::timeout(
                                Duration::from_secs(10),
                                writer.write_all(b"HTTP/1.0 200 OK\r\n\r\n"),
                            )
                            .await;
                            let _ = tokio::time::timeout(
                                Duration::from_secs(10),
                                echo_body_4x(&mut writer, &body),
                            )
                            .await;
                        }
                    } else {
                        // Headers exceed buffer — just return IP (fallback)
                        let response = format!("HTTP/1.0 200 OK\r\n\r\n{}", ip);
                        let _ = tokio::time::timeout(
                            Duration::from_secs(10),
                            writer.write_all(response.as_bytes()),
                        )
                        .await;
                    }
                } else {
                    // Pure RAW TCP Mode: return IP with zero HTTP overhead
                    let _ = tokio::time::timeout(
                        Duration::from_secs(10),
                        writer.write_all(ip.as_bytes()),
                    )
                    .await;
                }

                let _ = writer.shutdown().await;
            })
            .await;
        });
    }
}
