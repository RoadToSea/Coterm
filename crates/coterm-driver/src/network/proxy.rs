use super::config::{Config, Proxy};
use anyhow::{bail, Context, Result};
use std::net::IpAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Opens one TCP tunnel. The caller bounds DNS, connect and negotiation with one deadline.
pub(crate) async fn connect(config: &Config) -> Result<TcpStream> {
    let (host, port) = match &config.proxy {
        Proxy::Direct => (&config.host, config.port),
        Proxy::Socks5 { host, port } | Proxy::Http { host, port } => (host, *port),
    };
    let mut stream = TcpStream::connect((host.as_str(), port))
        .await
        .context("TCP connection failed")?;
    stream.set_nodelay(true)?;
    match config.proxy {
        Proxy::Direct => {}
        Proxy::Socks5 { .. } => socks5(&mut stream, &config.host, config.port).await?,
        Proxy::Http { .. } => http_connect(&mut stream, &config.host, config.port).await?,
    }
    Ok(stream)
}

async fn socks5(stream: &mut TcpStream, host: &str, port: u16) -> Result<()> {
    stream.write_all(&[5, 1, 0]).await?;
    let mut method = [0; 2];
    stream.read_exact(&mut method).await?;
    if method != [5, 0] {
        bail!("SOCKS5 proxy must accept authentication-free connections")
    }
    let mut request = vec![5, 1, 0];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend(ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend(ip.octets());
        }
        Err(_) => {
            if host.len() > 255 {
                bail!("SOCKS5 target hostname is too long")
            }
            request.extend([3, host.len() as u8]);
            request.extend(host.as_bytes());
        }
    }
    request.extend(port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut response = [0; 4];
    stream.read_exact(&mut response).await?;
    if response[..3] != [5, 0, 0] {
        bail!("SOCKS5 proxy rejected the tunnel")
    }
    let size = match response[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await? as usize,
        _ => bail!("Invalid SOCKS5 response address"),
    };
    let mut address = vec![0; size + 2];
    stream.read_exact(&mut address).await?;
    Ok(())
}

async fn http_connect(stream: &mut TcpStream, host: &str, port: u16) -> Result<()> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    stream
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await?;
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= 8192 {
            bail!("HTTP proxy response headers exceed 8 KiB")
        }
        response.push(stream.read_u8().await?);
    }
    let line = response.split(|b| *b == b'\n').next().unwrap_or_default();
    let mut fields = line.split(|b| *b == b' ');
    if !matches!(fields.next(), Some(b"HTTP/1.0" | b"HTTP/1.1")) || fields.next() != Some(b"200") {
        bail!("HTTP CONNECT proxy rejected the tunnel; proxy authentication is not supported")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{
        net::TcpListener,
        time::{timeout, Duration},
    };

    #[tokio::test]
    async fn http_fragmented_headers_preserve_the_first_tunneled_payload() -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let config = Config::parse(
            json!({"host":"::1","port":23,
            "proxy":{"type":"http","host":"127.0.0.1","port":listener.local_addr()?.port()}}),
            true,
        )?;
        let server = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await?;
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(peer.read_u8().await?);
            }
            assert_eq!(
                request,
                b"CONNECT [::1]:23 HTTP/1.1\r\nHost: [::1]:23\r\n\r\n"
            );
            peer.write_all(b"HTTP/1.1 200 OK\r\n").await?;
            peer.write_all(b"X-Test: yes\r\n\r\nfirst-payload").await?;
            Ok::<_, anyhow::Error>(())
        });
        let mut stream = timeout(Duration::from_secs(2), connect(&config)).await??;
        let mut data = [0; 13];
        stream.read_exact(&mut data).await?;
        assert_eq!(&data, b"first-payload");
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn socks5_handles_all_address_types_and_keeps_target_bytes() -> Result<()> {
        for (host, kind, length) in [("127.0.0.1", 1, 4), ("::1", 4, 16), ("board.local", 3, 11)] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let config = Config::parse(
                json!({"host":host,"port":8080,
                "proxy":{"type":"socks5","host":"127.0.0.1","port":listener.local_addr()?.port()}}),
                false,
            )?;
            let server = tokio::spawn(async move {
                let (mut peer, _) = listener.accept().await?;
                let mut greeting = [0; 3];
                peer.read_exact(&mut greeting).await?;
                assert_eq!(greeting, [5, 1, 0]);
                peer.write_all(&[5]).await?;
                peer.write_all(&[0]).await?;
                let mut header = [0; 4];
                peer.read_exact(&mut header).await?;
                assert_eq!(header, [5, 1, 0, kind]);
                if kind == 3 {
                    assert_eq!(peer.read_u8().await?, length as u8);
                }
                let mut address = vec![0; length];
                peer.read_exact(&mut address).await?;
                assert_eq!(peer.read_u16().await?, 8080);
                peer.write_all(&[5, 0, 0, 3, 3, b'o']).await?;
                peer.write_all(&[b'n', b'e', 0, 23, b'f', b'i', b'r', b's', b't'])
                    .await?;
                Ok::<_, anyhow::Error>(())
            });
            let mut stream = timeout(Duration::from_secs(2), connect(&config)).await??;
            let mut data = [0; 5];
            stream.read_exact(&mut data).await?;
            assert_eq!(&data, b"first");
            server.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn proxy_authentication_rejection_and_oversized_headers_fail_closed() -> Result<()> {
        for (proxy, response) in [
            ("socks5", vec![5, 2]),
            (
                "http",
                b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".to_vec(),
            ),
            ("http", vec![b'x'; 8193]),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let config = Config::parse(
                json!({"host":"board.local","proxy":{"type":proxy,"host":"127.0.0.1","port":listener.local_addr()?.port()}}),
                false,
            )?;
            let server = tokio::spawn(async move {
                let (mut peer, _) = listener.accept().await?;
                let mut request = [0; 1024];
                assert!(peer.read(&mut request).await? > 0);
                peer.write_all(&response).await?;
                Ok::<_, anyhow::Error>(())
            });
            assert!(timeout(Duration::from_secs(2), connect(&config))
                .await?
                .is_err());
            server.await??;
        }
        Ok(())
    }
}
