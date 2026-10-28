//! Image conversion and resizing (run with macOS `sips`).

use super::{Pattern, cap, pat, typed_src};
use crate::model::{FileType, ImageFormat, Intent};

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "convert_image",
            r"^(?:convert|change|turn|save)\s+(.+?)\s+(?:to|into|as)\s+(?:a\s+)?(png|jpe?g|webp|heic)(?:s|\s+files?|\s+format|\s+images?)?$",
            |c| {
                let format = match cap(c, 2)?.to_ascii_lowercase().as_str() {
                    "png" => ImageFormat::Png,
                    "jpg" | "jpeg" => ImageFormat::Jpg,
                    "webp" => ImageFormat::Webp,
                    _ => ImageFormat::Heic,
                };
                Some(Intent::ConvertImage { src: typed_src(c, 1, FileType::Images)?, format })
            },
        ),
        pat(
            "resize_image",
            r"^(?:resize|scale(?:\s+down)?|shrink)\s+(.+?)\s+(?:down\s+)?to\s+(\d+)\s*(?:px|pixels?)?(?:\s+(?:wide|width|across|in\s+width))?$",
            |c| {
                let width: u32 = cap(c, 2)?.parse().ok().filter(|w| *w > 0)?;
                Some(Intent::ResizeImage { src: typed_src(c, 1, FileType::Images)?, width })
            },
        ),
    ]
}
