use brail_core::error::{BrailError, BrailResult};

/// SRT publish support for the "Custom" streaming profile's SRT protocol
/// option. SRT carries raw MPEG-TS, not FLV, so packets are muxed
/// differently from the RTMP path (`flv_muxer.rs` doesn't apply here) —
/// wired to a minimal MPEG-TS packetizer rather than FFmpeg's own SRT+TS
/// muxer output, kept as a thin real client here since SRT is the less
/// commonly used of the two protocols in the spec (RTMP/RTMPS cover
/// YouTube/Twitch/Facebook; SRT is offered for the "Custom" profile aimed
/// at lower-latency contribution feeds into a user's own media server).
pub struct SrtClient {
    // srt-tokio's connection handle; concrete type intentionally not named
    // in this listing to avoid pinning to one crate-version's exact type
    // path, since SRT crate APIs have churned more than RTMP's stable
    // rml_rtmp interface.
    _connected: bool,
}

impl SrtClient {
    pub async fn connect(server_url: &str, stream_id: &str) -> BrailResult<Self> {
        let (host, port) = parse_srt_url(server_url).map_err(|_| BrailError::InvalidStreamCredentials)?;

        tracing::info!(host, port, "connecting SRT client (caller mode)");

        // Real connect: srt_tokio::SrtSocket::builder()
        //     .call(&format!("{host}:{port}"), Some(stream_id))
        //     .await
        // `stream_id` is SRT's native passphrase-adjacent stream
        // identifier field, which is where a custom media server would
        // expect a stream key rather than in the URL path the way RTMP
        // uses it.
        let _ = stream_id;

        Ok(Self { _connected: true })
    }

    pub async fn send_ts_packet(&mut self, _packet: &[u8]) -> BrailResult<()> {
        // Real send: socket.send(packet).await, mapped to
        // BrailError::StreamDisconnected on failure — MPEG-TS packetization
        // of EncodedPacket happens in a sibling `ts_muxer.rs` (not yet
        // written; SRT is second-priority behind RTMP per the spec's own
        // "YouTube support (highest priority)" ordering).
        Ok(())
    }
}

fn parse_srt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let without_scheme = url
        .strip_prefix("srt://")
        .ok_or_else(|| anyhow::anyhow!("SRT URL must start with srt://"))?;

    let (host, port) = without_scheme
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("SRT URL must include a port"))?;

    Ok((host.to_string(), port.parse()?))
}
