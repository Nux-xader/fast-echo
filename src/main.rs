use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;
use std::env;

/// Extract Content-Length from raw HTTP headers (case-insensitive, zero-alloc).
fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.split("\r\n") {
        let mut parts = line.splitn(2, ':');
        if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
            if name.eq_ignore_ascii_case("content-length") {
                return val.trim().parse().ok();
            }
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut addr = "[::]:8080".to_string();

    // Manual argument parsing to avoid heavy dependencies like `clap`.
    // This keeps the binary size extremely small (under 1 MB).
    let args: Vec<String> = env::args().collect();
    for i in 1..args.len() {
        if args[i] == "--addr" && i + 1 < args.len() {
            addr = args[i + 1].clone();
            break;
        }
    }

    let listener = TcpListener::bind(&addr).await?;
    println!("🚀 Ultra-Fast Hybrid Echo Server running on {}", addr);

    loop {
        let (socket, peer_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        tokio::spawn(async move {
            let (mut reader, writer) = socket.into_split();
            let mut writer = BufWriter::new(writer);
            let mut buf = [0u8; 1024]; // Stack buffer — no heap allocation

            let n = match reader.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => {
                    let _ = writer.shutdown().await;
                    return;
                }
            };

            let request = &buf[..n];

            // Detect HTTP methods that may carry a body
            let is_http = request.starts_with(b"GET")
                || request.starts_with(b"POST")
                || request.starts_with(b"PUT")
                || request.starts_with(b"PATCH")
                || request.starts_with(b"DELETE")
                || request.starts_with(b"HEAD")
                || request.starts_with(b"OPTIONS");

            // Extract peer IP, stripping IPv4-mapped IPv6 prefix
            let ip = peer_addr.ip().to_string();
            let ip = ip.strip_prefix("::ffff:").unwrap_or(&ip);

            if is_http {
                // Find end of headers
                let hdr_end = request
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4);

                if let Some(hdr_end) = hdr_end {
                    let header_str = std::str::from_utf8(&request[..hdr_end]).unwrap_or("");
                    let body_len = parse_content_length(header_str).unwrap_or(0);

                    if body_len == 0 {
                        // No body → return client IP (original behavior)
                        let response = format!("HTTP/1.0 200 OK\r\n\r\n{}", ip);
                        let _ = writer.write_all(response.as_bytes()).await;
                    } else {
                        let body_already_read = (n - hdr_end).min(body_len);

                        // Accumulate full body: part already in buffer + remainder from socket
                        let mut body = Vec::with_capacity(body_len);
                        body.extend_from_slice(&request[hdr_end..hdr_end + body_already_read]);

                        let remaining = body_len - body_already_read;
                        if remaining > 0 {
                            body.resize(body_len, 0);
                            reader
                                .read_exact(&mut body[body_already_read..])
                                .await
                                .ok();
                        }

                        // Write response: headers once, body 4x
                        let _ = writer.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await;
                        let _ = echo_body_4x(&mut writer, &body).await;
                    }
                } else {
                    // Headers exceed buffer — just return IP (fallback)
                    let response = format!("HTTP/1.0 200 OK\r\n\r\n{}", ip);
                    let _ = writer.write_all(response.as_bytes()).await;
                }
            } else {
                // Pure RAW TCP Mode: return IP with zero HTTP overhead
                let _ = writer.write_all(ip.as_bytes()).await;
            }

            let _ = writer.shutdown().await;
        });
    }
}
