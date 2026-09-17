use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFCreateAttributes, MFEnumDeviceSources, MFShutdown, MFStartup,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_VERSION,
};

/// Enumerates real webcams via Media Foundation's video capture device
/// source enumeration — the same API OBS and the stock Windows Camera app
/// use, so anything listed here is genuinely openable by
/// `brail-capture::webcam` (which uses `IMFActivate::ActivateObject` on the
/// same device to actually start frames flowing).
pub fn enumerate_cameras() -> anyhow::Result<Vec<String>> {
    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL_FLAG)?;
        let result = enumerate_inner();
        let _ = MFShutdown();
        result
    }
}

// MFSTARTUP_FULL constant isn't re-exported with a friendly name in every
// windows-rs version; 0 is MFSTARTUP_FULL's real underlying value.
const MFSTARTUP_FULL_FLAG: u32 = 0;

unsafe fn enumerate_inner() -> anyhow::Result<Vec<String>> {
    let attributes = MFCreateAttributes(1)?;
    attributes.SetGUID(
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    )?;

    let devices: Vec<Option<IMFActivate>> = MFEnumDeviceSources(&attributes)?;

    let mut names = Vec::new();
    for device in devices.into_iter().flatten() {
        if let Ok(name) = device.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME) {
            names.push(name.0.to_string_lossy());
        }
    }

    Ok(names)
}
