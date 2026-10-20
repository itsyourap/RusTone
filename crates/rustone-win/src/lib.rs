//! Windows shell integration that needs COM, kept out of the app crate
//! (which forbids unsafe code): the taskbar jump list.

/// A jump list entry: RusTone started with these arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub title: String,
    pub arguments: String,
}

/// Replaces the taskbar jump list: `category` with `recent` (e.g. numbers
/// to call) above the Tasks with `tasks`. Each entry starts `exe` with its
/// arguments. Call off the interface thread; it initialises COM there.
#[cfg(windows)]
pub fn set_jump_list(exe: &std::path::Path, tasks: &[Link], category: &str, recent: &[Link]) -> Result<(), String> {
    imp::set(exe, tasks, category, recent)
}

#[cfg(not(windows))]
pub fn set_jump_list(_exe: &std::path::Path, _tasks: &[Link], _category: &str, _recent: &[Link]) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoUninitialize};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, PWSTR, Result};

    use super::Link;

    /// COM for this thread, released on drop.
    struct Com;

    impl Com {
        fn init() -> Result<Self> {
            // SAFETY: paired with CoUninitialize in Drop on this thread.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
            Ok(Self)
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: initialised by `init` on this thread.
            unsafe { CoUninitialize() };
        }
    }

    pub fn set(exe: &Path, tasks: &[Link], category: &str, recent: &[Link]) -> std::result::Result<(), String> {
        let step = |what: &'static str| move |e: windows::core::Error| format!("{what}: {}", e.message());
        let _com = Com::init().map_err(step("starting COM"))?;
        // SAFETY: plain COM calls on interfaces this function owns; the
        // strings live until each call returns.
        unsafe {
            let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(step("creating the list"))?;
            let mut slots = 0u32;
            let _removed: IObjectArray = list.BeginList(&mut slots).map_err(step("beginning the list"))?;
            if !recent.is_empty() {
                let items = collection(exe, &recent[..recent.len().min(slots as usize)]).map_err(step("making the recent links"))?;
                let items = items.cast::<IObjectArray>().map_err(step("making the recent links"))?;
                // Access is denied when the user turned off recent items in
                // Start, Jump Lists and File Explorer: respect it, keep the tasks.
                match list.AppendCategory(&HSTRING::from(category), &items) {
                    Err(e) if e.code() == windows::Win32::Foundation::E_ACCESSDENIED => {}
                    other => other.map_err(step("adding the recent category"))?,
                }
            }
            if !tasks.is_empty() {
                let items = collection(exe, tasks).and_then(|c| c.cast::<IObjectArray>()).map_err(step("making the tasks"))?;
                list.AddUserTasks(&items).map_err(step("adding the tasks"))?;
            }
            list.CommitList().map_err(step("committing the list"))
        }
    }

    unsafe fn collection(exe: &Path, links: &[Link]) -> Result<IObjectCollection> {
        // SAFETY: as in `set`.
        unsafe {
            let items: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            for link in links {
                items.AddObject(&shell_link(exe, link)?)?;
            }
            Ok(items)
        }
    }

    unsafe fn shell_link(exe: &Path, link: &Link) -> Result<IShellLinkW> {
        // SAFETY: as in `set`; the title PROPVARIANT owns a CoTaskMem copy
        // that PropVariantClear frees.
        unsafe {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let path = HSTRING::from(exe.as_os_str());
            shell.SetPath(&path)?;
            shell.SetArguments(&HSTRING::from(link.arguments.as_str()))?;
            shell.SetIconLocation(&path, 0)?;
            shell.SetDescription(&HSTRING::from(link.title.as_str()))?;
            let store: IPropertyStore = shell.cast()?;
            let mut title = lpwstr_variant(&link.title);
            let result = store.SetValue(&PKEY_Title, &title).and_then(|()| store.Commit());
            let _ = PropVariantClear(&mut title);
            result?;
            Ok(shell)
        }
    }

    /// A VT_LPWSTR PROPVARIANT (what the shell wants for titles).
    unsafe fn lpwstr_variant(text: &str) -> PROPVARIANT {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a CoTaskMem buffer of the right size, filled before use;
        // the PROPVARIANT's owner frees it with PropVariantClear.
        unsafe {
            let buffer = CoTaskMemAlloc(wide.len() * 2).cast::<u16>();
            let mut variant = PROPVARIANT::default();
            if buffer.is_null() {
                return variant;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), buffer, wide.len());
            let inner = &mut variant.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = PWSTR(buffer);
            variant
        }
    }
}
