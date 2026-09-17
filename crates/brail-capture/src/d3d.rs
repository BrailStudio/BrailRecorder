use windows::core::Interface;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;

/// Owns the single D3D11 device the whole capture->encode pipeline shares.
/// Sharing one device (rather than one per subsystem) is what makes
/// zero-copy GPU frame handoff possible: a texture created on this device
/// can be opened directly by the encoder's D3D11VA/NVENC/AMF/QSV session
/// without a CPU round-trip, as long as both sides keep the same device (or
/// use a keyed mutex / shared handle across devices on multi-GPU systems —
/// see `open_shared_texture` below for that path).
pub struct D3dContext {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// The WinRT-facing wrapper around `device`, required by
    /// Direct3D11CaptureFramePool::CreateFreeThreaded, which takes a WinRT
    /// `IDirect3DDevice`, not the raw Win32 `ID3D11Device`.
    pub winrt_device: IDirect3DDevice,
}

impl D3dContext {
    /// Creates a hardware D3D11 device with BGRA support (required by
    /// Windows Graphics Capture, which always hands back BGRA8 surfaces)
    /// and the WinRT interop wrapper WGC's frame pool needs.
    pub fn new() -> anyhow::Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_UNKNOWN,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }

        let device = device.ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice returned no device"))?;
        let context = context.ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice returned no context"))?;

        let dxgi_device: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?.cast()? };

        Ok(Self {
            device,
            context,
            winrt_device,
        })
    }

    /// Opens a texture that was shared from a *different* D3D11 device
    /// (relevant on hybrid iGPU/dGPU laptops where the capture adapter and
    /// the encode-capable adapter differ) via its NT shared handle. Used by
    /// `brail-encoder::gpu_surface` when `is_capture_adapter` in the
    /// hardware profile doesn't match the adapter the chosen encoder
    /// backend needs to run on.
    pub fn open_shared_texture(&self, shared_handle: HANDLE) -> anyhow::Result<ID3D11Texture2D> {
        unsafe {
            let texture: ID3D11Texture2D = self.device.OpenSharedResource(shared_handle)?;
            Ok(texture)
        }
    }
}

/// Extracts the raw `ID3D11Texture2D` backing a WinRT `IDirect3DSurface`
/// (what `Direct3D11CaptureFrame::Surface` returns) via the
/// `IDirect3DDxgiInterfaceAccess` bridge interface — the standard,
/// documented way to cross from a WinRT surface back into Win32 D3D11.
pub fn texture_from_surface(
    surface: &windows::Graphics::DirectX::Direct3D11::IDirect3DSurface,
) -> anyhow::Result<ID3D11Texture2D> {
    use windows::Win32::Graphics::Direct3D11::IDirect3DDxgiInterfaceAccess;
    unsafe {
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let texture: ID3D11Texture2D = access.GetInterface()?;
        Ok(texture)
    }
}
