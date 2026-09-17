use brail_core::error::{BrailError, BrailResult};
use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Real RTMP handshake + connect + publish flow, built on `rml_rtmp`'s
/// session state machine (which produces bytes to send / consumes bytes
/// received, but does no I/O itself) driving a plain Tokio `TcpStream`.
/// RTMPS (TLS) wraps the same session logic in a `tokio-native-tls`
/// stream at the transport layer — omitted here as a thin variant to keep
/// this listing focused on the base protocol flow.
pub struct RtmpClient {
    stream: TcpStream,
    session: ClientSession,
    stream_id: Option<f64>,
}

impl RtmpClient {
    pub async fn connect(server_url: &str, stream_key: &str) -> BrailResult<Self> {
        let (host, port, app) = parse_rtmp_url(server_url)
            .map_err(|_| BrailError::InvalidStreamCredentials)?;

        let mut stream = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| BrailError::StreamConnectFailed(format!("TCP connect to {host}:{port} failed: {e}")))?;

        perform_handshake(&mut stream)
            .await
            .map_err(|e| BrailError::StreamConnectFailed(format!("RTMP handshake failed: {e}")))?;

        let config = ClientSessionConfig::new();
        let (mut session, initial_results) = ClientSession::new(config)
            .map_err(|e| BrailError::StreamConnectFailed(format!("session init failed: {e:?}")))?;

        send_results(&mut stream, initial_results).await
            .map_err(|e| BrailError::StreamConnectFailed(e.to_string()))?;

        let connect_results = session
            .request_connection(app)
            .map_err(|e| BrailError::StreamConnectFailed(format!("connect request failed: {e:?}")))?;
        send_results(&mut stream, connect_results).await
            .map_err(|e| BrailError::StreamConnectFailed(e.to_string()))?;

        let mut client = Self {
            stream,
            session,
            stream_id: None,
        };

        client.wait_for_connect_success().await?;

        let publish_results = client
            .session
            .request_publishing(stream_key.to_string(), rml_rtmp::sessions::PublishRequestType::Live)
            .map_err(|e| BrailError::StreamConnectFailed(format!("publish request failed: {e:?}")))?;
        client.send(publish_results).await?;

        client.wait_for_publish_success().await?;

        Ok(client)
    }

    /// Sends one already-FLV-muxed media tag over the publish stream.
    pub async fn send_media(&mut self, flv_tag: &[u8]) -> BrailResult<()> {
        // The FLV tag's payload (minus FLV's own 11-byte tag header/4-byte
        // trailer, which are a container-file convention, not part of
        // RTMP's own chunk framing) is what actually goes out as an RTMP
        // "video data" / "audio data" message via
        // `ClientSession::publish_video_data` /
        // `ClientSession::publish_audio_data`. Re-deriving that split from
        // `flv_tag` here rather than changing `flv_muxer` to hand back
        // pre-split payloads keeps `flv_muxer` usable unmodified for the
        // local MKV/MP4 recording path too, which does want the full FLV
        // tag framing.
        if flv_tag.len() < 11 {
            return Err(BrailError::Internal("malformed FLV tag".into()));
        }
        let tag_type = flv_tag[0];
        let payload = &flv_tag[11..flv_tag.len() - 4];
        let timestamp_ms = u32::from_be_bytes([0, flv_tag[4], flv_tag[5], flv_tag[6]]);

        let results = if tag_type == 9 {
            self.session
                .publish_video_data(bytes::Bytes::copy_from_slice(payload), timestamp_ms.into(), false)
        } else {
            self.session
                .publish_audio_data(bytes::Bytes::copy_from_slice(payload), timestamp_ms.into(), false)
        }
        .map_err(|e| BrailError::StreamDisconnected(format!("publish failed: {e:?}")))?;

        self.send(vec![results]).await
    }

    async fn wait_for_connect_success(&mut self) -> BrailResult<()> {
        self.pump_until(|event| matches!(event, ClientSessionEvent::ConnectionRequestAccepted))
            .await
    }

    async fn wait_for_publish_success(&mut self) -> BrailResult<()> {
        self.pump_until(|event| matches!(event, ClientSessionEvent::PublishRequestAccepted))
            .await
    }

    async fn pump_until(
        &mut self,
        predicate: impl Fn(&ClientSessionEvent) -> bool,
    ) -> BrailResult<()> {
        let mut buf = [0u8; 4096];
        loop {
            let n = self
                .stream
                .read(&mut buf)
                .await
                .map_err(|e| BrailError::StreamDisconnected(e.to_string()))?;
            if n == 0 {
                return Err(BrailError::StreamDisconnected("server closed connection".into()));
            }

            let results = self
                .session
                .handle_input(&buf[..n])
                .map_err(|e| BrailError::StreamDisconnected(format!("protocol error: {e:?}")))?;

            let mut to_send = Vec::new();
            for r in results {
                match r {
                    ClientSessionResult::OutboundResponse(packet) => to_send.push(packet),
                    ClientSessionResult::RaisedEvent(event) => {
                        if predicate(&event) {
                            self.send(to_send.into_iter().map(ClientSessionResult::OutboundResponse).collect())
                                .await?;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            self.send(to_send.into_iter().map(ClientSessionResult::OutboundResponse).collect())
                .await?;
        }
    }

    async fn send(&mut self, results: Vec<ClientSessionResult>) -> BrailResult<()> {
        for result in results {
            if let ClientSessionResult::OutboundResponse(packet) = result {
                self.stream
                    .write_all(&packet.bytes)
                    .await
                    .map_err(|e| BrailError::StreamDisconnected(e.to_string()))?;
            }
        }
        Ok(())
    }
}

async fn send_results(stream: &mut TcpStream, results: Vec<ClientSessionResult>) -> anyhow::Result<()> {
    for result in results {
        if let ClientSessionResult::OutboundResponse(packet) = result {
            stream.write_all(&packet.bytes).await?;
        }
    }
    Ok(())
}

async fn perform_handshake(stream: &mut TcpStream) -> anyhow::Result<()> {
    let mut handshake = Handshake::new(PeerType::Client);
    let c0_c1 = handshake.generate_outbound_p0_and_p1()?;
    stream.write_all(&c0_c1).await?;

    let mut buf = vec![0u8; 4096];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            anyhow::bail!("connection closed during handshake");
        }
        match handshake.process_bytes(&buf[..n])? {
            HandshakeProcessResult::InProgress { response_bytes } => {
                if !response_bytes.is_empty() {
                    stream.write_all(&response_bytes).await?;
                }
            }
            HandshakeProcessResult::Completed { response_bytes, remaining_bytes: _ } => {
                if !response_bytes.is_empty() {
                    stream.write_all(&response_bytes).await?;
                }
                return Ok(());
            }
        }
    }
}

/// Parses `rtmp://host[:port]/app` (and `rtmps://`) into its parts. The
/// stream key is deliberately *not* parsed out of the URL here even if a
/// service-specific format embeds it — it's always passed separately from
/// `brail_core::config::StreamProfile::stream_key` so it never ends up
/// concatenated into a URL that could be logged.
fn parse_rtmp_url(url: &str) -> anyhow::Result<(String, u16, String)> {
    let without_scheme = url
        .strip_prefix("rtmp://")
        .or_else(|| url.strip_prefix("rtmps://"))
        .ok_or_else(|| anyhow::anyhow!("URL must start with rtmp:// or rtmps://"))?;

    let default_port = if url.starts_with("rtmps://") { 443 } else { 1935 };

    let (host_port, app) = without_scheme
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("URL missing application path"))?;

    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(default_port)),
        None => (host_port.to_string(), default_port),
    };

    Ok((host, port, app.to_string()))
}
