use ffmpeg_next::ffi;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

/// Wraps the raw D3D11 device pointer in an FFmpeg `AVHWDeviceContext` so
/// hardware encoders (nvenc/amf/qsv, all of which accept D3D11VA frames
/// directly) can encode a texture WGC handed us without ever copying pixel
/// data to the CPU. This is the mechanism the spec's "GPU zero-copy
/// pipeline" and "<5% additional CPU overhead for encoding" requirements
/// both depend on — a CPU round-trip for every captured frame would blow
/// both budgets at 1080p60+.
///
/// This module is intentionally the only place in the codebase that drops
/// to raw FFmpeg FFI (`ffmpeg-next`'s safe wrapper doesn't expose hwframe
/// context creation), and every unsafe block here is scoped to a single
/// FFI call with the safety argument in a comment above it.
pub struct HwDeviceContext {
    raw: *mut ffi::AVBufferRef,
}

// SAFETY: AVBufferRef for a hw device context is reference-counted by
// FFmpeg internally and safe to move across threads as long as it isn't
// mutated concurrently without synchronization — which this codebase
// never does; the encoder pipeline owns it exclusively per-session.
unsafe impl Send for HwDeviceContext {}

impl HwDeviceContext {
    /// Builds an `AV_HWDEVICE_TYPE_D3D11VA` context that wraps the *same*
    /// `ID3D11Device` the capture engine created (see
    /// `brail-capture::d3d::D3dContext`), which is what makes this
    /// zero-copy: FFmpeg encodes textures that already live on that
    /// device's adapter without an inter-device copy.
    pub fn from_existing_device(device: &ID3D11Device) -> anyhow::Result<Self> {
        unsafe {
            let mut raw: *mut ffi::AVBufferRef = std::ptr::null_mut();

            let ret = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
            if ret.is_null() {
                anyhow::bail!("av_hwdevice_ctx_alloc returned null");
            }
            raw = ret;

            let ctx = &mut *((*raw).data as *mut ffi::AVHWDeviceContext);
            let d3d11_ctx = &mut *(ctx.hwctx as *mut ffi::AVD3D11VADeviceContext);

            // Hand FFmpeg our existing device and AddRef it so FFmpeg's
            // eventual Release on context teardown doesn't outlive our own
            // ID3D11Device COM reference.
            device.cast_to_raw_and_addref(&mut d3d11_ctx.device);

            let init_ret = ffi::av_hwdevice_ctx_init(raw);
            if init_ret < 0 {
                ffi::av_buffer_unref(&mut raw);
                anyhow::bail!("av_hwdevice_ctx_init failed: {init_ret}");
            }

            Ok(Self { raw })
        }
    }

    pub fn as_raw(&self) -> *mut ffi::AVBufferRef {
        self.raw
    }
}

impl Drop for HwDeviceContext {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.raw) };
    }
}

/// Small helper trait so `from_existing_device` above reads cleanly; the
/// actual implementation is a raw `AddRef` + pointer cast, which is exactly
/// what FFmpeg's own `d3d11va_device_create` does internally when it
/// creates its *own* device instead of reusing ours.
trait CastToRawAddRef {
    unsafe fn cast_to_raw_and_addref(&self, out: &mut *mut std::ffi::c_void);
}

impl CastToRawAddRef for ID3D11Device {
    unsafe fn cast_to_raw_and_addref(&self, out: &mut *mut std::ffi::c_void) {
        use windows::core::Interface;
        let ptr = self.as_raw();
        // AddRef because AVD3D11VADeviceContext takes ownership and will
        // Release it exactly once when the hw device context is freed.
        let unknown: windows::core::IUnknown = std::mem::transmute_copy(self);
        std::mem::forget(unknown.clone()); // clone() does the AddRef
        *out = ptr as *mut std::ffi::c_void;
    }
}
