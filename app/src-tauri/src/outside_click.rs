//! AppKit mouse events sent to other apps also dismiss a visible window that
//! never became key. Webview blur alone cannot detect that case.
use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSWindow};
use std::{cell::RefCell, ptr::NonNull};
use tauri::{AppHandle, Emitter, Manager};

/// Both inputs use AppKit's global screen points (not Tauri's top-left
/// coordinates or physical pixels). Negative origins on secondary displays
/// need no conversion. Treat the entire border as inside too.
fn point_in_frame(point: (f64, f64), origin: (f64, f64), size: (f64, f64)) -> bool {
    point.0 >= origin.0 && point.0 <= origin.0 + size.0 && point.1 >= origin.1 && point.1 <= origin.1 + size.1
}

// AppKit installation, callbacks and removal all run on the main thread.
thread_local! {
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

pub fn install(app: &AppHandle) -> Result<(), &'static str> {
    let handle = app.clone();
    let handler = RcBlock::new(move |_: NonNull<NSEvent>| {
        let Some(window) = handle.get_webview_window("main") else { return };
        // Global monitors can observe the activating click on our own window.
        // AppKit delivers this callback on the main thread, so read the live
        // frame here, before emitting any dismissal into the webview.
        let Ok(ptr) = window.ns_window() else { return };
        // SAFETY: Tauri owns this live NSWindow; this callback is on the main thread.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        let frame = ns.frame();
        let point = NSEvent::mouseLocation();
        if point_in_frame((point.x, point.y), (frame.origin.x, frame.origin.y), (frame.size.width, frame.size.height)) {
            return;
        }
        let state = handle.state::<crate::AppState>();
        let Ok(presentation) = state.presentation.lock() else {
            eprintln!("Presentation unavailable; restart Voxi");
            return;
        };
        let request_id = presentation.outside_click_request(window.is_visible().unwrap_or(false));
        if let Some(request_id) = request_id {
            // Only a request ID crosses into the page. It applies its existing
            // busy guard before starting the normal animated dismissal.
            let _ = window.emit("voxi://outside-click", request_id);
        }
    });
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
        &handler,
    )
    .ok_or("Couldn't install outside-click dismissal")?;
    MONITOR.with(|slot| {
        *slot.borrow_mut() = Some(monitor);
    });
    Ok(())
}

pub fn uninstall() {
    MONITOR.with(|slot| {
        if let Some(monitor) = slot.borrow_mut().take() {
            // SAFETY: this exact token came from AppKit's monitor installer.
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::point_in_frame;

    #[test]
    fn inside_and_all_edges_are_not_outside_clicks() {
        for point in [
            (15.0, 30.0),
            (10.0, 20.0),
            (110.0, 20.0),
            (10.0, 70.0),
            (110.0, 70.0),
            (10.0, 30.0),
            (110.0, 30.0),
            (15.0, 20.0),
            (15.0, 70.0),
        ] {
            assert!(point_in_frame(point, (10.0, 20.0), (100.0, 50.0)), "{point:?}");
        }
        for point in [(9.9, 30.0), (110.1, 30.0), (15.0, 19.9), (15.0, 70.1)] {
            assert!(!point_in_frame(point, (10.0, 20.0), (100.0, 50.0)), "{point:?}");
        }
    }

    #[test]
    fn secondary_displays_can_have_negative_and_fractional_coordinates() {
        let origin = (-1440.5, -900.5);
        let size = (680.0, 132.0);
        assert!(point_in_frame((-1400.25, -850.25), origin, size));
        assert!(point_in_frame(origin, origin, size));
        assert!(point_in_frame((-760.5, -768.5), origin, size));
        assert!(!point_in_frame((-760.4, -850.0), origin, size));
        assert!(!point_in_frame((100.0, 100.0), origin, size));
        // A window crossing the primary display's origin uses the same space.
        assert!(point_in_frame((10.0, 10.0), (-100.0, -100.0), (680.0, 132.0)));
    }
}
