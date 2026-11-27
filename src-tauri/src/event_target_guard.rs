//! Desktop exit hang (L4 2026-10-18, downloader enhancement D1c).
//!
//! tao keeps one hidden "Tao Thread Event Target" window per event loop. It is
//! a visible (`WS_VISIBLE | WS_POPUP`, layered and transparent), untitled and
//! unowned top-level window, and its window procedure leaves `WM_CLOSE` to
//! `DefWindowProc`, which destroys it. An external close aimed at the
//! process's "main window" can reach it: `Process.CloseMainWindow` takes the
//! first visible unowned top-level window, which was this one while the real
//! window was minimized (L4 close receipt: untitled window 111877066 instead
//! of the "EOStage" window), and `taskkill` without `/F` or the Restart
//! Manager post `WM_CLOSE` to every top-level window. Without its event target
//! the event loop loses the messages it posts to itself: the real window's
//! close was not handled while it stayed minimized, and after it was finally
//! destroyed the event loop never exited (minidump: the main thread idle in
//! tao's `GetMessageW`, no application, scheduler or worker thread left, the
//! profile lock held). The guard keeps the event target window alive by
//! ignoring `WM_CLOSE` sent to it; every other message reaches tao unchanged.
//!
//! Downloader enhancement D1d: ignoring that close kept the event loop alive,
//! but `Process.CloseMainWindow` then no longer closed the application. The
//! guard now forwards the close to the real main window (posted, never
//! handled inside this window procedure), so it takes the application's
//! normal close flow: the main window's close request, where the desktop host
//! shuts its services down, then the window closes and the application exits.
//! The event target window stays alive throughout.

#[cfg(windows)]
mod windows_impl {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClassNameW, IsWindow, PostMessageW, WM_CLOSE,
    };

    /// The window class of tao's event target window (tao 0.35.3,
    /// `platform_impl/windows/event_loop.rs`).
    pub(super) const EVENT_TARGET_CLASS: &str = "Tao Thread Event Target";
    /// The identifier of this subclass on the event target window.
    const GUARD_SUBCLASS_ID: usize = 0x0D1C_0001;

    /// Keeps `WM_CLOSE` from destroying the event target window and posts it to
    /// the main window (`close_target`, zero for none) instead; forwards every
    /// other message unchanged to tao.
    unsafe extern "system" fn forward_close(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        close_target: usize,
    ) -> LRESULT {
        if message == WM_CLOSE {
            let main_window = close_target as HWND;
            // SAFETY: IsWindow accepts any handle value; posting returns at
            // once, so the main window's close is handled later by tao, outside
            // this window procedure.
            unsafe {
                if !main_window.is_null() && IsWindow(main_window) != 0 {
                    PostMessageW(main_window, WM_CLOSE, 0, 0);
                }
            }
            return 0;
        }
        // SAFETY: forwards the message, unchanged, to the next procedure of
        // this window's subclass chain.
        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }

    unsafe extern "system" fn collect_event_targets(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the `Vec` passed by `event_target_windows_of_this_thread`,
        // which outlives the enumeration.
        let found = unsafe { &mut *(found as *mut Vec<HWND>) };
        let mut class = [0_u16; 64];
        // SAFETY: the buffer outlives the call and its length is passed with it.
        let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
        if length > 0 && String::from_utf16_lossy(&class[..length as usize]) == EVENT_TARGET_CLASS {
            found.push(window);
        }
        1
    }

    /// The tao event target windows owned by the calling thread.
    pub(super) fn event_target_windows_of_this_thread() -> Vec<HWND> {
        let mut found: Vec<HWND> = Vec::new();
        // SAFETY: the callback uses `found` only during this call.
        unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                Some(collect_event_targets),
                &mut found as *mut Vec<HWND> as LPARAM,
            );
        }
        found
    }

    pub(super) fn guard_this_thread(close_target: Option<isize>) -> usize {
        let reference = close_target.unwrap_or(0) as usize;
        event_target_windows_of_this_thread()
            .into_iter()
            .filter(|window| {
                // SAFETY: `window` is a live window of the calling thread, the
                // only thread allowed to subclass it. Installing the same
                // subclass again replaces it (and its close target), so
                // repeated calls are harmless.
                unsafe {
                    SetWindowSubclass(*window, Some(forward_close), GUARD_SUBCLASS_ID, reference)
                        != 0
                }
            })
            .count()
    }
}

/// Keeps the tao event target windows of the calling thread, which must be
/// the event loop thread, from being destroyed by `WM_CLOSE`, and forwards that
/// close to `close_target` (the main window's handle) when given. Returns how
/// many windows are guarded; zero on Windows means tao's internals changed
/// and this guard needs review.
pub(crate) fn guard_event_target_windows(close_target: Option<isize>) -> usize {
    #[cfg(windows)]
    {
        windows_impl::guard_this_thread(close_target)
    }
    #[cfg(not(windows))]
    {
        let _ = close_target;
        0
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::platform::windows::{EventLoopBuilderExtWindows, WindowExtWindows};
    use tao::window::WindowBuilder;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_CLOSE};

    use super::{guard_event_target_windows, windows_impl};

    /// A real tao event loop on its own thread receives `WM_CLOSE` at its event
    /// target window, as `Process.CloseMainWindow` sent it in L4, and is then
    /// asked to exit through its proxy (as tauri asks it). The loop exits and
    /// its event target window survives.
    #[test]
    fn a_wm_close_at_the_event_target_window_does_not_keep_the_event_loop_from_exiting() {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut event_loop = EventLoopBuilder::<()>::with_user_event()
                .with_any_thread(true)
                .build();
            let guarded = guard_event_target_windows(None);
            let targets = windows_impl::event_target_windows_of_this_thread();
            for &target in &targets {
                // SAFETY: posts a message to a window of this thread.
                unsafe {
                    PostMessageW(target, WM_CLOSE, 0, 0);
                }
            }
            let proxy = event_loop.create_proxy();
            let mut survived = false;
            event_loop.run_return(|event, _, control_flow| {
                *control_flow = ControlFlow::Wait;
                match event {
                    Event::NewEvents(StartCause::Init) => {
                        let _ = proxy.send_event(());
                    }
                    Event::UserEvent(()) => {
                        // SAFETY: IsWindow accepts any handle value.
                        survived = targets
                            .iter()
                            .all(|target| unsafe { IsWindow(*target) } != 0);
                        *control_flow = ControlFlow::Exit;
                    }
                    _ => {}
                }
            });
            let _ = sender.send((targets.len(), guarded, survived));
        });
        let outcome = receiver.recv_timeout(Duration::from_secs(10));
        let Ok((targets, guarded, survived)) = outcome else {
            panic!("the event loop did not exit within 10 s after WM_CLOSE reached its event target window");
        };
        assert_eq!(targets, 1, "tao has one event target window per event loop");
        assert_eq!(guarded, 1, "the event target window is guarded");
        assert!(survived, "the event target window survived WM_CLOSE");
    }

    /// Downloader enhancement D1d: `Process.CloseMainWindow` reached the event
    /// target window, and swallowing that `WM_CLOSE` kept the window alive but no
    /// longer closed the application. The close must take the application's
    /// normal close flow: the real main window (hidden here, nothing is shown)
    /// receives its close request, where the desktop host shuts its services
    /// down, while the event target window is still alive.
    #[test]
    fn a_wm_close_at_the_event_target_window_requests_the_main_window_close() {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut event_loop = EventLoopBuilder::<()>::with_user_event()
                .with_any_thread(true)
                .build();
            let window = WindowBuilder::new()
                .with_visible(false)
                .build(&event_loop)
                .expect("hidden main window");
            let guarded = guard_event_target_windows(Some(window.hwnd()));
            let targets = windows_impl::event_target_windows_of_this_thread();
            for &target in &targets {
                // SAFETY: posts a message to a window of this thread.
                unsafe {
                    PostMessageW(target, WM_CLOSE, 0, 0);
                }
            }
            let mut target_alive_at_close = None;
            event_loop.run_return(|event, _, control_flow| {
                *control_flow = ControlFlow::Wait;
                if let Event::WindowEvent {
                    window_id,
                    event: WindowEvent::CloseRequested,
                    ..
                } = event
                {
                    if window_id == window.id() {
                        // SAFETY: IsWindow accepts any handle value.
                        target_alive_at_close = Some(
                            targets
                                .iter()
                                .all(|target| unsafe { IsWindow(*target) } != 0),
                        );
                        *control_flow = ControlFlow::Exit;
                    }
                }
            });
            let _ = sender.send((guarded, target_alive_at_close));
        });
        let Ok((guarded, target_alive_at_close)) = receiver.recv_timeout(Duration::from_secs(10))
        else {
            panic!(
                "the main window received no close request within 10 s after WM_CLOSE reached the event target window"
            );
        };
        assert_eq!(guarded, 1, "the event target window is guarded");
        assert_eq!(
            target_alive_at_close,
            Some(true),
            "the main window's close was requested while the event target window was alive"
        );
    }
}
