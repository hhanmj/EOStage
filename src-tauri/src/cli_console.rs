//! Console output of the command-line modes of a GUI-subsystem host.
//!
//! L5 recheck 2026-10-19: the release host was a console-subsystem program
//! (PE Subsystem 3). Started from Explorer it opened a console window, and
//! closing that window ended the application. A release build is now a Windows
//! GUI program (see `main.rs`), so it has no console of its own. Its
//! command-line modes (`--selftest`, `--smoke-test`, ...) still print one
//! payload and set the exit code: when their standard output or error is not
//! redirected (started from a terminal), they attach to the parent's console so
//! the output appears there. Redirected handles (the packaging smoke reads
//! pipes) are kept unchanged; the exit code is the process exit code either way.

/// Whether the parent's console is needed: some standard stream has no usable
/// handle.
#[cfg(any(windows, test))]
fn needs_parent_console(stdout_usable: bool, stderr_usable: bool) -> bool {
    !(stdout_usable && stderr_usable)
}

/// Attach the standard streams that have no usable handle to the parent
/// process's console, if it has one. Does nothing on other platforms and when
/// both streams are usable (a console build, or redirected handles).
pub fn attach_parent_console_for_cli() {
    #[cfg(windows)]
    windows_console::attach();
}

#[cfg(windows)]
mod windows_console {
    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_UNKNOWN,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_HANDLE, STD_OUTPUT_HANDLE,
    };

    fn usable(stream: STD_HANDLE) -> bool {
        // SAFETY: plain queries of this process's standard handle and its type.
        let handle: HANDLE = unsafe { GetStdHandle(stream) };
        !handle.is_null()
            && handle != INVALID_HANDLE_VALUE
            && unsafe { GetFileType(handle) } != FILE_TYPE_UNKNOWN
    }

    pub(super) fn attach() {
        let stdout_usable = usable(STD_OUTPUT_HANDLE);
        let stderr_usable = usable(STD_ERROR_HANDLE);
        if !super::needs_parent_console(stdout_usable, stderr_usable) {
            return;
        }
        // SAFETY: attaching to the parent's console has no memory effects;
        // failure (no parent console) leaves the process unchanged.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let console = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if console == INVALID_HANDLE_VALUE {
            return;
        }
        // SAFETY: `console` is a valid handle owned for the rest of the process.
        unsafe {
            if !stdout_usable {
                SetStdHandle(STD_OUTPUT_HANDLE, console);
            }
            if !stderr_usable {
                SetStdHandle(STD_ERROR_HANDLE, console);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::needs_parent_console;

    #[test]
    fn only_a_missing_stream_needs_the_parent_console() {
        assert!(!needs_parent_console(true, true));
        assert!(needs_parent_console(false, true));
        assert!(needs_parent_console(true, false));
        assert!(needs_parent_console(false, false));
    }

    /// Under the test harness both streams are captured; attaching must leave
    /// them alone.
    #[test]
    fn redirected_streams_are_kept() {
        super::attach_parent_console_for_cli();
        println!("still captured");
    }
}
