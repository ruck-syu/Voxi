//! The menu bar face "talks" while someone types or dictates into Voxi's box:
//! the ripple on its smile travels along, then settles once the typing stops.
//! Only Voxi's own text box drives it; Voxi never watches typing elsewhere.
//!
//! The frames are drawn by docs/icon/appicon.swift next to icons/tray.png.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::{AppHandle, Runtime};

pub const TRAY_ID: &str = "voxi";

const REST: &[u8] = include_bytes!("../icons/tray.png");
const FRAMES: [&[u8]; 12] = [
    include_bytes!("../icons/tray-talk-0.png"),
    include_bytes!("../icons/tray-talk-1.png"),
    include_bytes!("../icons/tray-talk-2.png"),
    include_bytes!("../icons/tray-talk-3.png"),
    include_bytes!("../icons/tray-talk-4.png"),
    include_bytes!("../icons/tray-talk-5.png"),
    include_bytes!("../icons/tray-talk-6.png"),
    include_bytes!("../icons/tray-talk-7.png"),
    include_bytes!("../icons/tray-talk-8.png"),
    include_bytes!("../icons/tray-talk-9.png"),
    include_bytes!("../icons/tray-talk-10.png"),
    include_bytes!("../icons/tray-talk-11.png"),
];

/// Time between frames: twelve frames make one 600ms sweep of the ripple.
const FRAME: Duration = Duration::from_millis(50);
/// How long the face keeps talking after the last keystroke.
const LINGER: Duration = Duration::from_millis(450);

static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_INPUT: Mutex<Option<Instant>> = Mutex::new(None);

/// The resting face and the talking frames, decoded once.
fn images() -> &'static (Image<'static>, Vec<Image<'static>>) {
    static IMAGES: OnceLock<(Image<'static>, Vec<Image<'static>>)> = OnceLock::new();
    IMAGES.get_or_init(|| {
        let decode = |bytes| Image::from_bytes(bytes).expect("tray frames are valid PNGs");
        (decode(REST), FRAMES.into_iter().map(decode).collect())
    })
}

pub fn rest_icon() -> tauri::Result<Image<'static>> {
    Image::from_bytes(REST)
}

fn show<R: Runtime>(app: &AppHandle<R>, image: &Image<'static>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    // One call that swaps the image and keeps it a template. Setting the icon
    // and then the template flag separately paints an untinted frame in
    // between, which shows as a flicker.
    let _ = tray.set_icon_with_as_template(Some(image.clone()), true);
}

/// Called on each change to the command box. Starts the animation if it
/// isn't running, or keeps it going a little longer if it is.
pub fn nudge<R: Runtime>(app: &AppHandle<R>) {
    *LAST_INPUT.lock().unwrap() = Some(Instant::now());
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let (rest, frames) = images();
        let mut i = 0;
        loop {
            let idle = LAST_INPUT.lock().unwrap().map_or(true, |t| t.elapsed() >= LINGER);
            // Finish the sweep so the ripple comes to rest where the icon's does.
            if idle && i % frames.len() == 0 {
                break;
            }
            show(&app, &frames[i % frames.len()]);
            i += 1;
            std::thread::sleep(FRAME);
        }
        show(&app, rest);
        RUNNING.store(false, Ordering::SeqCst);
        // A keystroke that landed between the idle check and the flag reset
        // would otherwise be missed; start again for it.
        if LAST_INPUT.lock().unwrap().is_some_and(|t| t.elapsed() < LINGER) {
            nudge(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_decodes_at_the_rest_icons_size() {
        let (rest, frames) = images();
        assert_eq!(frames.len(), 12);
        for frame in frames {
            assert_eq!((frame.width(), frame.height()), (rest.width(), rest.height()));
        }
    }
}
