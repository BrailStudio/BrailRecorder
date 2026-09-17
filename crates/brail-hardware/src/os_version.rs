use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    REG_SZ, REG_VALUE_TYPE,
};

const KEY_PATH: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";

/// Reads the real build number + a human-readable version name from the
/// registry. `GetVersionEx`/`RdtscGetVersion` are deliberately avoided:
/// both are subject to the application-compatibility version-lie shim,
/// which would make brail-hardware misreport the OS on some
/// enterprise-managed machines. The registry keys read here are the same
/// ones Windows Setup itself writes and are not shimmed.
pub fn detect_windows_version() -> anyhow::Result<(u32, String)> {
    let build_str = read_registry_string("CurrentBuildNumber").unwrap_or_else(|| "0".to_string());
    let build: u32 = build_str.parse().unwrap_or(0);

    let product_name =
        read_registry_string("ProductName").unwrap_or_else(|| "Windows (unknown edition)".to_string());
    let display_version = read_registry_string("DisplayVersion");

    let version_name = match display_version {
        Some(v) => format!("{product_name} {v} (build {build})"),
        None => format!("{product_name} (build {build})"),
    };

    Ok((build, version_name))
}

fn read_registry_string(value_name: &str) -> Option<String> {
    unsafe {
        let mut hkey = HKEY::default();
        let subkey: Vec<u16> = KEY_PATH.encode_utf16().chain(std::iter::once(0)).collect();

        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }

        let value: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = [0u16; 256];
        let mut buf_len = (buf.len() * 2) as u32;
        let mut value_type = REG_VALUE_TYPE::default();

        let status = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            Some(&mut value_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_len),
        );

        let _ = RegCloseKey(hkey);

        if status.is_err() || value_type != REG_SZ {
            return None;
        }

        let len_u16 = (buf_len as usize / 2).saturating_sub(1); // drop trailing NUL
        Some(String::from_utf16_lossy(&buf[..len_u16]))
    }
}
