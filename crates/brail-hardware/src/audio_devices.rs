
use windows::Win32::Media::Audio::{
    eCapture, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

pub fn enumerate_audio_devices() -> anyhow::Result<(Vec<String>, Vec<String>)> {
    unsafe {
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
        let id = device.GetId()?;

        if let Ok(id_string) = id.to_string() {
            names.push(id_string);
        }
    }

    Ok(names)
}
