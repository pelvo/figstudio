//! The jump list (Windows): a right-click on the taskbar icon lists recent files. Each item
//! starts figstudio with its file; a file gone by then fails to open with the usual toast
//! and is marked missing, so the next list leaves it out.

use std::path::{Path, PathBuf};

use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
use windows::core::{HSTRING, Interface as _};

/// The jump list's heading over the files.
const CATEGORY: &str = "Recent files";

/// The command line argument that opens `path`.
fn argument(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Lists `files` (newest first) in the jump list. Files the person took off the list
/// themselves stay off (Windows refuses the whole list otherwise).
pub(crate) fn show(files: &[PathBuf]) -> anyhow::Result<()> {
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let items: Vec<(String, String, HSTRING)> = files
        .iter()
        .map(|path| {
            (
                argument(path),
                crate::recents::file_label(path),
                HSTRING::from(path.as_os_str()),
            )
        })
        .collect();
    #[expect(unsafe_code, reason = "the Windows jump list is a COM API")]
    // SAFETY: plain COM calls on this (the UI) thread, which winit initialised for OLE;
    // `CoInitializeEx` only adds a reference or reports the existing mode. Every interface
    // comes from `CoCreateInstance` or a call that returns it, and the `windows` wrappers
    // release them. The strings are `HSTRING`s that outlive the calls, and the argument
    // buffer is sized and read up to its first nul.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0u32;
        let removed: IObjectArray = list.BeginList(&mut slots)?;
        let mut gone = Vec::new();
        for i in 0..removed.GetCount()? {
            let Ok(link) = removed.GetAt::<IShellLinkW>(i) else {
                continue;
            };
            let mut text = [0u16; 1024];
            if link.GetArguments(&mut text).is_ok() {
                let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                gone.push(String::from_utf16_lossy(&text[..end]));
            }
        }
        let collection: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        let mut count = 0;
        for (argument, title, description) in &items {
            if gone.contains(argument) || count >= slots {
                continue;
            }
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&exe)?;
            link.SetArguments(&HSTRING::from(argument.as_str()))?;
            link.SetIconLocation(&exe, 0)?;
            link.SetDescription(description)?;
            let store: IPropertyStore = link.cast()?;
            store.SetValue(&PKEY_Title, &PROPVARIANT::from(title.as_str()))?;
            store.Commit()?;
            collection.AddObject(&link)?;
            count += 1;
        }
        if count > 0 {
            let array: IObjectArray = collection.cast()?;
            list.AppendCategory(&HSTRING::from(CATEGORY), &array)?;
        }
        list.CommitList()?;
    }
    Ok(())
}
