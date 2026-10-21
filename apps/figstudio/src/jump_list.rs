//! The jump list (Windows): a right-click on the taskbar icon lists recent files. Each item
//! starts figstudio with its file; a file gone by then fails to open with the usual toast
//! and is marked missing, so the next list leaves it out.

use std::path::{Path, PathBuf};

#[cfg(windows)]
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
#[cfg(windows)]
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
#[cfg(windows)]
use windows::core::{HSTRING, Interface as _};

/// The jump list's heading over the files.
#[cfg(windows)]
const CATEGORY: &str = "Recent files";

/// The command line argument that opens `path`.
fn argument(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Publishes `items` with `try_list`, which reports whether Windows took a list. A refused
/// list (it holds a file the person removed that was not recognised) is tried again without
/// one item at a time. `Some(left_out)` when one was taken, `None` when none was.
fn publish<T: Copy>(items: &[T], mut try_list: impl FnMut(&[T]) -> bool) -> Option<Option<T>> {
    if try_list(items) {
        return Some(None);
    }
    (0..items.len()).find_map(|skip| {
        let rest: Vec<T> = items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, item)| *item)
            .collect();
        try_list(&rest).then_some(Some(items[skip]))
    })
}

/// The file an item's argument opens (`argument` read back).
fn path_of_argument(argument: &str) -> PathBuf {
    let inner = argument.strip_prefix('"').unwrap_or(argument);
    PathBuf::from(inner.strip_suffix('"').unwrap_or(inner))
}

/// Lists `files` (newest first) in the jump list. Returns the files the person took off it
/// since the last list: the caller takes them off Recents, so they stay off (Windows asks
/// apps to, and refuses a list that puts one back).
#[cfg(windows)]
pub(crate) fn show(files: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let titles = crate::recents::shortlist_labels(files);
    let items: Vec<(String, String, HSTRING)> = files
        .iter()
        .zip(titles)
        .map(|(path, title)| (argument(path), title, HSTRING::from(path.as_os_str())))
        .collect();
    // Arguments of items the person removed, as Windows reports them.
    let mut gone: Vec<String> = Vec::new();
    let mut failed = None;
    let order: Vec<usize> = (0..items.len()).collect();
    #[expect(unsafe_code, reason = "the Windows jump list is a COM API")]
    // SAFETY: plain COM calls on this (the UI) thread, which winit initialised for OLE;
    // `CoInitializeEx` only adds a reference or reports the existing mode. Every interface
    // comes from `CoCreateInstance` or a call that returns it, and the `windows` wrappers
    // release them. The strings are `HSTRING`s that outlive the calls, and the argument
    // buffer is sized for the longest command line and read up to its first nul. Each try
    // is one list session, ended by `CommitList` or `AbortList`.
    let published = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        // One session with the items `chosen`: true when Windows took it. A refused one is
        // aborted, so the list shown stays, and the next try starts clean (a commit after a
        // refusal would not clear what Windows holds against the removed item).
        let mut session = |chosen: &[usize]| -> windows::core::Result<bool> {
            let mut slots = 0u32;
            let removed: IObjectArray = list.BeginList(&mut slots)?;
            for i in 0..removed.GetCount()? {
                let Ok(link) = removed.GetAt::<IShellLinkW>(i) else {
                    continue;
                };
                // The longest command line Windows takes, so no argument comes back cut.
                let mut text = vec![0u16; 32_768];
                if link.GetArguments(&mut text).is_ok() {
                    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                    let argument = String::from_utf16_lossy(&text[..end]);
                    if !gone.contains(&argument) {
                        gone.push(argument);
                    }
                }
            }
            let collection: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            let mut count = 0;
            for &i in chosen {
                let (argument, title, description) = &items[i];
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
                if list
                    .AppendCategory(&HSTRING::from(CATEGORY), &array)
                    .is_err()
                {
                    list.AbortList()?;
                    return Ok(false);
                }
            }
            list.CommitList()?;
            Ok(true)
        };
        publish(&order, |chosen| match session(chosen) {
            Ok(taken) => taken,
            Err(error) => {
                failed = Some(error);
                false
            }
        })
    };
    if let Some(error) = failed {
        return Err(error.into());
    }
    let mut taken_off: Vec<PathBuf> = gone.iter().map(|a| path_of_argument(a)).collect();
    match published {
        // Refused with it, taken without: the person removed it.
        Some(Some(i)) => taken_off.push(files[i].clone()),
        Some(None) => {}
        None => tracing::warn!("jump list refused; the one shown stays"),
    }
    Ok(taken_off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_list_is_tried_again_without_one_file_at_a_time() {
        let files = ["a", "b", "c"];
        // Windows refuses any list with "b" (removed, but not recognised as such).
        let mut tried = Vec::new();
        let published = publish(&files, |list| {
            tried.push(list.to_vec());
            !list.contains(&"b")
        });
        assert_eq!(
            published,
            Some(Some("b")),
            "published without b, which was removed"
        );
        assert_eq!(tried, [vec!["a", "b", "c"], vec!["b", "c"], vec!["a", "c"]]);
        // Accepted at once: nothing left out.
        assert_eq!(publish(&files, |_| true), Some(None));
        // Refused whatever is left out: nothing is published (the old list stays).
        assert_eq!(publish(&files, |_| false), None);
    }

    #[test]
    fn an_items_argument_reads_back_as_its_file() {
        for path in ["C:\\Design files\\a b.fig", "/home/someone/x & y.fig"] {
            let path = Path::new(path);
            assert_eq!(path_of_argument(&argument(path)), path);
        }
    }
}
