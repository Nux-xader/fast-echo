use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::env;

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
        let (mut socket, peer_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        tokio::spawn(async move {
            let mut buf = [0; 64]; // Small buffer to peek at the incoming request
            
            // Read the first bytes to detect if this is an HTTP client (curl/browser) or Raw TCP
            let is_http = match socket.read(&mut buf).await {
                Ok(n) if n > 0 => buf.starts_with(b"GET") || buf.starts_with(b"POST"),
                _ => false,
            };

            let mut ip = peer_addr.ip().to_string();
            
            // Strip IPv4-mapped IPv6 prefix if present
            if ip.starts_with("::ffff:") {
                ip = ip[7..].to_string();
            }

            if is_http {
                // Ultra-minimal HTTP response: No Date, No Content-Type, No Server headers.
                // HTTP/1.0 tells the client to close the connection immediately after the response.
                // Total overhead is only 19 bytes!
                let response = format!("HTTP/1.0 200 OK\r\n\r\n{}", ip);
                let _ = socket.write_all(response.as_bytes()).await;
            } else {
                // Pure RAW TCP Mode for internal proxy micro-calibration (0 bytes overhead)
                let _ = socket.write_all(ip.as_bytes()).await;
            }

            let _ = socket.shutdown().await;
        });
    }
}
