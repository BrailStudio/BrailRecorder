
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, MFCreateAttributes, MFEnumDeviceSources, MFShutdown, MFStartup,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_VERSION,
};
use windows::Win32::System::Com::CoTaskMemFree;

pub fn enumerate_cameras() -> anyhow::Result<Vec<String>> {
    unsafe {
        MFStartup(MF_VERSION, 0)?;
        let result = enumerate_inner();
        let _ = MFShutdown();
        result
    }
}

unsafe fn enumerate_inner() -> anyhow::Result<Vec<String>> {
    let mut attributes: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attributes, 1)?;

    let attributes = attributes
        .ok_or_else(|| anyhow::anyhow!("Could not create camera attributes"))?;

    attributes.SetGUID(
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    )?;

    let mut devices: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count: u32 = 0;

    MFEnumDeviceSources(&attributes, &mut devices, &mut count)?;

    let mut names = Vec::new();

    if !devices.is_null() {
        let device_list = std::slice::from_raw_parts(devices, count as usize);

        for device in device_list.iter().flatten() {
            if let Ok(name) =
                device.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
            {
                names.push(name.0.to_string_lossy());
            }
        }

        CoTaskMemFree(Some(devices as *const std::ffi::c_void));
    }

    Ok(names)
}
