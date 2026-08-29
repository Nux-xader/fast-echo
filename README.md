# fast-echo

`fast-echo` is an ultra-fast, zero-overhead hybrid TCP/HTTP echo server written in Rust using Tokio. It is purpose-built to serve as a lightweight public IP checker (similar to `api.ipify.org`), but optimized specifically for extreme high-concurrency NAT64/CGNAT micro-calibration.

## Features

- **Hybrid Protocol Detection**: Automatically detects whether the incoming connection is an HTTP client or a Raw TCP client.
- **Raw TCP Mode (0 Bytes Overhead)**: If a non-HTTP request is detected, it instantly returns the raw IP address and closes the socket. No HTTP headers are parsed or sent.
- **Ultra-Minimal HTTP Mode (19 Bytes Overhead)**: If a `curl` or browser request (`GET` / `POST`) is detected, it responds with a stripped-down `HTTP/1.0 200 OK` header containing only the IP.
- **4× Body Echo**: When an HTTP request includes a body (with `Content-Length`), the server echoes it back exactly 4 times its original size. Supports all HTTP methods (`POST`, `PUT`, `PATCH`, `DELETE`, etc.).
- **Dependency-Free Arguments**: Argument parsing is done manually via `std::env` to completely avoid heavy dependencies like `clap`.
- **Alpine / Docker Ready**: Fully static compilation target (`x86_64-unknown-linux-musl`) with LTO and binary stripping, resulting in a binary size of ~665 KB.
- **Portable**: Compiled without hardware-locked CPU flags, making it safe to deploy on any legacy or modern x86_64 VPS architecture.

## Installation & Build

Ensure you have the Rust `musl` toolchain installed for static compilation:

```bash
rustup target add x86_64-unknown-linux-musl
```

Compile the project with the release profile:

```bash
cargo build --release
```

The statically linked, highly-optimized binary will be available at:
`target/x86_64-unknown-linux-musl/release/fast-echo`

## Usage

Start the server. By default, it listens on port `8080` for both IPv4 and IPv6 (`[::]:8080`).

```bash
./target/x86_64-unknown-linux-musl/release/fast-echo
```

### Custom Port Binding
You can define a custom IP and port using the `--addr` argument:

```bash
./target/x86_64-unknown-linux-musl/release/fast-echo --addr "0.0.0.0:80"
```

## Testing

**Testing IP Echo (no body):**
```bash
curl http://127.0.0.1:8080
```
*Output will be your IPv4 address.*

**Testing 4× Body Echo:**
```bash
curl -X POST http://127.0.0.1:8080 -d "hello"
```
*Output: `hellohellohellohello` (5 bytes × 4 = 20 bytes)*

```bash
curl -X PUT http://127.0.0.1:8080 -d "AB"
```
*Output: `ABABABAB` (2 bytes × 4 = 8 bytes)*

**Testing Raw TCP Mode:**
```bash
echo "RAW" | nc 127.0.0.1 8080
```
*Output will be exactly your IP address with zero HTTP headers.*
