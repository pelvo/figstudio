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

/// Publishes `items` with `try_list`, which answers `Ok(None)` when Windows refused a list
/// (it holds a file the person removed that was not recognised) and `Ok(Some(appended))`
/// when it took it (`appended`: the list held files). A refused list is tried again without
/// one item at a time, but never as an empty list. `Some(left_out)` when one was taken: the
/// item left out is named only when the list taken held files; `None` when none was (the
/// list shown stays). An error stops at once.
fn publish<T: Copy, E>(
    items: &[T],
    mut try_list: impl FnMut(&[T]) -> Result<Option<bool>, E>,
) -> Result<Option<Option<T>>, E> {
    if try_list(items)?.is_some() {
        return Ok(Some(None));
    }
    for skip in 0..items.len() {
        let rest: Vec<T> = items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, item)| *item)
            .collect();
        // An empty list taken would clear what Windows holds against the removed file
        // without showing which it was, so it could come back: not tried.
        if rest.is_empty() {
            continue;
        }
        if let Some(appended) = try_list(&rest)? {
            return Ok(Some(appended.then_some(items[skip])));
        }
    }
    Ok(None)
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
        // One session (after `BeginList`) with the items `chosen`: `None` when Windows refused
        // the list, `Some(appended)` when it was committed. A refused one is aborted, so the
        // list shown stays and the next try starts clean (a commit after a refusal would not
        // clear what Windows holds against the removed item).
        let mut fill = |chosen: &[usize],
                        slots: u32,
                        removed: &IObjectArray|
         -> windows::core::Result<Option<bool>> {
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
                    return Ok(None);
                }
            }
            list.CommitList()?;
            Ok(Some(count > 0))
        };
        publish(&order, |chosen| {
            let mut slots = 0u32;
            let removed: IObjectArray = list.BeginList(&mut slots)?;
            // Any failure inside ends the session too, so the list shown stays.
            fill(chosen, slots, &removed).inspect_err(|_| {
                let _ = list.AbortList();
            })
        })?
    };
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

    /// What Windows does with a list in these tests: refuse it while it holds `blocked`.
    fn windows(blocked: &'static str) -> impl FnMut(&[&'static str]) -> Result<Option<bool>, ()> {
        move |list| {
            Ok(if list.contains(&blocked) {
                None
            } else {
                Some(!list.is_empty())
            })
        }
    }

    #[test]
    fn a_refused_list_is_tried_again_without_one_file_at_a_time() {
        let files = ["a", "b", "c"];
        let mut tried = Vec::new();
        let mut take = windows("b");
        let published = publish(&files, |list| {
            tried.push(list.to_vec());
            take(list)
        });
        assert_eq!(
            published,
            Ok(Some(Some("b"))),
            "published without b, which was removed"
        );
        assert_eq!(tried, [vec!["a", "b", "c"], vec!["b", "c"], vec!["a", "c"]]);
        // Accepted at once: nothing left out.
        assert_eq!(publish(&files, |_| Ok::<_, ()>(Some(true))), Ok(Some(None)));
        // Refused whatever is left out: nothing is published (the old list stays).
        assert_eq!(publish(&files, |_| Ok::<_, ()>(None)), Ok(None));
    }

    #[test]
    fn a_retry_never_publishes_an_empty_list() {
        // One file, refused: an empty list would be taken and clear what Windows holds
        // against the removed file, which could then come back. It is not tried; the list
        // shown stays and no file is blamed.
        let mut tried = Vec::new();
        let mut take = windows("a");
        let published = publish(&["a"], |list| {
            tried.push(list.to_vec());
            take(list)
        });
        assert_eq!(published, Ok(None));
        assert_eq!(tried, [vec!["a"]]);
        // With no files at all the empty list is the list, and is published.
        assert_eq!(publish(&[] as &[&str], windows("a")), Ok(Some(None)));
    }

    #[test]
    fn an_error_stops_the_tries_at_once() {
        let mut tried = 0;
        let published = publish(&["a", "b"], |list| {
            tried += 1;
            if list.len() == 2 {
                Ok(None)
            } else {
                Err("broken")
            }
        });
        assert_eq!(published, Err("broken"));
        assert_eq!(tried, 2, "no list is tried after the error");
    }

    #[test]
    fn an_items_argument_reads_back_as_its_file() {
        for path in ["C:\\Design files\\a b.fig", "/home/someone/x & y.fig"] {
            let path = Path::new(path);
            assert_eq!(path_of_argument(&argument(path)), path);
        }
    }
}
