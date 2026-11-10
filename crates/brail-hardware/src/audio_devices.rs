use windows::core::PWSTR;
use windows::Win32::Media::Audio::{
    eCapture, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, StructuredStorage::PropVariantClear,
    CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;

/// Enumerates active WASAPI render (speaker/headset — used for desktop
/// audio loopback capture) and capture (microphone) endpoints by their real
/// friendly names, exactly as they'd appear in Windows Sound settings. The
/// device actually opened for loopback/mic capture at recording time is
/// resolved again in `brail-audio::wasapi` by endpoint ID, not by name
/// (names aren't guaranteed unique) — this list is for UI display and
/// selection only.
pub fn enumerate_audio_devices() -> anyhow::Result<(Vec<String>, Vec<String>)> {
    unsafe {
        // COINIT_MULTITHREADED matches the apartment model the rest of the
        // capture/audio pipeline runs under; CoInitializeEx returning
        // S_FALSE (already initialized on this thread) is not an error.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let result = enumerate_inner();

        CoUninitialize();
        result
    }
}

unsafe fn enumerate_inner() -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

    let capture_devices = list_endpoints(&enumerator, eCapture)?;
    let render_devices = list_endpoints(&enumerator, eRender)?;

    Ok((capture_devices, render_devices))
}

unsafe fn list_endpoints(
    enumerator: &IMMDeviceEnumerator,
    data_flow: windows::Win32::Media::Audio::EDataFlow,
) -> anyhow::Result<Vec<String>> {
    let collection = enumerator.EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)?;
    let count = collection.GetCount()?;

    let mut names = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device = collection.Item(i)?;
        let store: IPropertyStore = device.OpenPropertyStore(windows::Win32::System::Com::STGM_READ)?;
        let mut prop = store.GetValue(&PKEY_Device_FriendlyName)?;

        // PROPVARIANT -> string: the friendly name property is always
        // VT_LPWSTR for this key on real hardware; anything else means a
        // malformed driver, which we skip rather than crash on.
        let raw_pwstr = PWSTR(prop.Anonymous.Anonymous.Anonymous.pwszVal.0);
        if let Ok(name) = raw_pwstr.to_string() {
            names.push(name);
        }
        let _ = PropVariantClear(&mut prop);
    }

    Ok(names)
}
