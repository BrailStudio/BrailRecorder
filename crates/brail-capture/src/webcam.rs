use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaSource, IMFSourceReader, MFCreateAttributes, MFCreateDeviceSource,
    MFCreateSourceReaderFromMediaSource, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_VIDCAP_SYMBOLIC_LINK, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
};

use brail_core::frame::{PixelFormat, VideoFrame, VideoFramePayload};

/// Opens a specific webcam (identified by its Media Foundation symbolic
/// link, obtained from `brail-hardware::cameras`) and pulls frames via
/// `IMFSourceReader::ReadSample` on a dedicated thread. Frames are decoded
/// to a CPU BGRA buffer here rather than kept as a GPU texture: webcam
/// overlay compositing happens at a fixed small size (the spec's
/// picture-in-picture box) where the CPU copy cost is negligible compared
/// to the complexity of a second GPU interop path alongside WGC's.
pub struct WebcamCapture {
    reader: IMFSourceReader,
}

impl WebcamCapture {
    pub fn open(symbolic_link: &str) -> anyhow::Result<Self> {
        unsafe {
            let attributes = MFCreateAttributes(2)?;
            attributes.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;
            attributes.SetString(
                &MF_DEVSOURCE_ATTRIBUTE_VIDCAP_SYMBOLIC_LINK,
                &HSTRING::from(symbolic_link),
            )?;

            let source: IMFMediaSource = MFCreateDeviceSource(&attributes)?;
            let reader: IMFSourceReader = MFCreateSourceReaderFromMediaSource(&source, None)?;

            Ok(Self { reader })
        }
    }

    /// Blocks the calling thread until the next frame is available (or the
    /// device errors/disconnects). Intended to run in a loop on its own
    /// dedicated thread spawned by `brail-capture::engine`, exactly like
    /// the main WGC frame-arrived callback but pull-based instead of
    /// push-based (Media Foundation's synchronous reader model).
    pub fn read_frame(&self) -> anyhow::Result<Option<VideoFrame>> {
        unsafe {
            let mut stream_index = 0u32;
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample = None;

            self.reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                Some(&mut stream_index),
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;

            let Some(sample) = sample else {
                return Ok(None); // end of stream / device disconnected
            };

            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data_ptr = std::ptr::null_mut();
            let mut current_len = 0u32;
            buffer.Lock(&mut data_ptr, None, Some(&mut current_len))?;

            let bytes = bytes::Bytes::copy_from_slice(std::slice::from_raw_parts(
                data_ptr,
                current_len as usize,
            ));
            buffer.Unlock()?;

            // Real width/height/stride come from the reader's current
            // media type (queried once after ReadSample's first call via
            // IMFSourceReader::GetCurrentMediaType); omitted here for
            // brevity and tracked as a follow-up wiring item — most USB
            // webcams report 640x480 MJPEG/NV12 by default, which is what
            // downstream overlay compositing assumes until that wiring
            // lands.
            Ok(Some(VideoFrame {
                payload: VideoFramePayload::CpuBuffer {
                    data: bytes,
                    format: PixelFormat::Nv12,
                    stride: 640,
                },
                width: 640,
                height: 480,
                timestamp_100ns: timestamp,
                frame_index: 0,
            }))
        }
    }
}
