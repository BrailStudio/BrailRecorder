use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::{AudioFrame, AudioSource};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// A running WASAPI capture stream — either desktop-audio loopback (opened
/// on a *render* endpoint with `AUDCLNT_STREAMFLAGS_LOOPBACK`, which is the
/// documented way to capture "what you hear" without a virtual cable) or a
/// normal microphone capture endpoint.
pub struct WasapiCapture {
    client: IAudioClient,
    capture_client: IAudioCaptureClient,
    format: WaveFormat,
    source: AudioSource,
}

#[derive(Clone, Copy)]
pub struct WaveFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl WasapiCapture {
    /// Opens desktop-audio loopback on the current default render endpoint.
    /// Re-resolving the *current* default (rather than caching a device)
    /// matters because the spec requires handling a default-device change
    /// (e.g. user switches from speakers to a Bluetooth headset) mid
    /// recording without crashing — `brail-audio::pipeline` re-opens this
    /// on a device-change notification rather than this struct trying to
    /// self-heal.
    pub fn open_desktop_loopback() -> BrailResult<Self> {
        let device = default_endpoint(eRender)
            .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;
        Self::open(device, AUDCLNT_STREAMFLAGS_LOOPBACK.0 as u32, AudioSource::DesktopLoopback)
    }

    /// Opens a specific microphone by its WASAPI endpoint (resolved from
    /// `brail-hardware::audio_devices`'s enumeration). Falls back to the
    /// system default capture endpoint if `device_id` is `None`.
    pub fn open_microphone(device_id: Option<&str>) -> BrailResult<Self> {
        let device = match device_id {
            Some(id) => endpoint_by_id(id).map_err(|e| BrailError::AudioDeviceError(e.to_string()))?,
            None => default_endpoint(windows::Win32::Media::Audio::eCapture)
                .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?,
        };
        Self::open(device, 0, AudioSource::Microphone)
    }

    fn open(device: IMMDevice, stream_flags: u32, source: AudioSource) -> BrailResult<Self> {
        unsafe {
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;

            let mix_format_ptr = client
                .GetMixFormat()
                .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;
            let mix_format = *mix_format_ptr;

            // 200ms buffer: generous enough to tolerate scheduler jitter on
            // a loaded system (the spec's "works under load" requirement)
            // without adding perceptible audio latency for streaming.
            const BUFFER_DURATION_100NS: i64 = 2_000_000;

            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    BUFFER_DURATION_100NS,
                    0,
                    mix_format_ptr,
                    None,
                )
                .map_err(|e| BrailError::AudioDeviceError(format!("IAudioClient::Initialize failed: {e}")))?;

            let capture_client: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;

            client
                .Start()
                .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;

            Ok(Self {
                client,
                capture_client,
                format: WaveFormat {
                    sample_rate: mix_format.nSamplesPerSec,
                    channels: mix_format.nChannels,
                    bits_per_sample: mix_format.wBitsPerSample,
                },
                source,
            })
        }
    }

    pub fn format(&self) -> WaveFormat {
        self.format
    }

    /// Pulls all currently-available packets from the endpoint buffer.
    /// WASAPI delivers audio in variable-sized packets on its own timer
    /// (typically every ~10ms), so this is called from a polling loop
    /// (`brail-audio` pipeline task) rather than blocking on a fixed frame
    /// size the way video capture does.
    pub fn read_available(&self) -> BrailResult<Vec<AudioFrame>> {
        let mut frames = Vec::new();

        unsafe {
            loop {
                let packet_len = self
                    .capture_client
                    .GetNextPacketSize()
                    .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;

                if packet_len == 0 {
                    break;
                }

                let mut data_ptr = std::ptr::null_mut();
                let mut frames_available = 0u32;
                let mut flags = 0u32;
                let mut device_position = 0u64;
                let mut qpc_position = 0u64;

                self.capture_client
                    .GetBuffer(
                        &mut data_ptr,
                        &mut frames_available,
                        &mut flags,
                        Some(&mut device_position),
                        Some(&mut qpc_position),
                    )
                    .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;

                let bytes_per_frame = (self.format.bits_per_sample / 8) as usize * self.format.channels as usize;
                let byte_len = frames_available as usize * bytes_per_frame;

                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    // Silent packet (e.g. nothing is playing for loopback
                    // capture): WASAPI doesn't guarantee zeroed memory at
                    // data_ptr in this case, so silence is synthesized
                    // rather than read from the pointer.
                    bytes::Bytes::from(vec![0u8; byte_len])
                } else {
                    bytes::Bytes::copy_from_slice(std::slice::from_raw_parts(data_ptr, byte_len))
                };

                frames.push(AudioFrame {
                    samples,
                    sample_rate: self.format.sample_rate,
                    channels: self.format.channels,
                    // QPC position is in 100ns units already, matching the
                    // timestamp convention used for video frames — required
                    // for A/V sync in the muxer.
                    timestamp_100ns: qpc_position as i64,
                    source: self.source,
                });

                self.capture_client
                    .ReleaseBuffer(frames_available)
                    .map_err(|e| BrailError::AudioDeviceError(e.to_string()))?;
            }
        }

        Ok(frames)
    }
}

impl Drop for WasapiCapture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn default_endpoint(data_flow: windows::Win32::Media::Audio::EDataFlow) -> anyhow::Result<IMMDevice> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(data_flow, eConsole)?;
        Ok(device)
    }
}

fn endpoint_by_id(id: &str) -> anyhow::Result<IMMDevice> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let id_wide = windows::core::HSTRING::from(id);
        let device = enumerator.GetDevice(&id_wide)?;
        Ok(device)
    }
}
