//! COM and TSF registration (`regsvr32 seamtype.dll`, run elevated by the installer): the COM
//! class, the input profile under Korean, and the categories that tell TSF what the text
//! service can do (keyboard, display attributes, Store apps, the taskbar button).

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, MAX_PATH};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr, ITfInputProcessorProfileMgr,
    GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIP_KEYBOARD,
};
use windows_core::{Result, GUID, PCWSTR};

use crate::{module, CLSID_SEAMTYPE, GUID_PROFILE, LANGID_KOREAN};

const CATEGORIES: [GUID; 4] = [
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn guid_string(guid: &GUID) -> String {
    format!("{{{guid:?}}}")
}

/// This DLL's path.
pub fn module_path() -> String {
    let mut buffer = vec![0u16; MAX_PATH as usize * 4];
    let n = unsafe { GetModuleFileNameW(Some(module()), &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..n])
}

fn set_string(key: HKEY, name: Option<&str>, value: &str) -> Result<()> {
    let name = name.map(wide);
    let data: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    unsafe {
        RegSetValueExW(key, name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())), None, REG_SZ, Some(&data)).ok()
    }
}

fn create_key(parent: HKEY, path: &str) -> Result<HKEY> {
    let path = wide(path);
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(parent, PCWSTR(path.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
            .ok()?;
    }
    Ok(key)
}

fn clsid_key() -> String {
    format!("SOFTWARE\\Classes\\CLSID\\{}", guid_string(&CLSID_SEAMTYPE))
}

fn initialize_com() {
    // regsvr32 has usually done this; a second call only returns S_FALSE or a mode error.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

pub fn register() -> Result<()> {
    initialize_com();
    let path = module_path();

    let class = create_key(HKEY_LOCAL_MACHINE, &clsid_key())?;
    set_string(class, None, "Seamtype")?;
    let server = create_key(class, "InprocServer32")?;
    set_string(server, None, &path)?;
    set_string(server, Some("ThreadingModel"), "Apartment")?;
    unsafe {
        let _ = windows::Win32::System::Registry::RegCloseKey(server);
        let _ = windows::Win32::System::Registry::RegCloseKey(class);
    }

    unsafe {
        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let description: Vec<u16> = "Seamtype".encode_utf16().collect();
        let icon: Vec<u16> = path.encode_utf16().collect();
        profiles.RegisterProfile(&CLSID_SEAMTYPE, LANGID_KOREAN, &GUID_PROFILE, &description, &icon, 0, HKL::default(), 0, true, 0)?;

        let categories: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for category in CATEGORIES {
            categories.RegisterCategory(&CLSID_SEAMTYPE, &category, &CLSID_SEAMTYPE)?;
        }
    }
    Ok(())
}

pub fn unregister() -> Result<()> {
    initialize_com();
    unsafe {
        if let Ok(categories) = CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER) {
            for category in CATEGORIES {
                let _ = categories.UnregisterCategory(&CLSID_SEAMTYPE, &category, &CLSID_SEAMTYPE);
            }
        }
        if let Ok(profiles) =
            CoCreateInstance::<_, ITfInputProcessorProfileMgr>(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)
        {
            let _ = profiles.UnregisterProfile(&CLSID_SEAMTYPE, LANGID_KOREAN, &GUID_PROFILE, 0);
        }
        let key = wide(&clsid_key());
        let status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()));
        if status.is_err() && status != ERROR_FILE_NOT_FOUND {
            return status.ok();
        }
    }
    Ok(())
}
