//! Zip and unzip (run with macOS `ditto`).

use super::{Pattern, cap, pat, phrase::clean_name, src};
use crate::model::Intent;
use regex::Regex;
use std::sync::LazyLock;

/// "into a file called out.zip", "as an archive named out.zip" -> "out.zip"
static OUTPUT_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:an?\s+)?(?:(?:zip\s+)?(?:file|archive)\s+)?(?:called|named)\s+").unwrap());

/// "into a zip", "as one archive": no name given, so Voxi picks one.
static NO_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(?:an?|one|1|a\s+single|single)\s+)?(?:zip|archive|zip\s+file|zipped\s+file|compressed\s+file)$").unwrap());
/// "into one out.zip" -> "out.zip"
static COUNT_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:an?\s+single|single|one|1)\s+(\S+\.[a-z0-9]{1,5})$").unwrap());

fn output(raw: &str) -> Option<String> {
    let s = OUTPUT_LABEL.replace(raw.trim(), "");
    if NO_NAME.is_match(&s) { return None; }
    Some(clean_name(&COUNT_WORD.replace(&s, "$1")))
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "unzip",
            r"^(?:unzip|decompress|expand|extract|unpack)\s+(.+?\.zip)(?:\s+(?:into|to|in)\s+(.+))?$",
            |c| Some(Intent::Unzip { src: src(c, 1)?, destination: cap(c, 2).and_then(|s| output(&s)) }),
        ),
        pat("zip", r"^(?:zip(?:\s+up)?|compress|archive)\s+(.+?)(?:\s+(?:into|to|as)\s+(.+))?$", |c| {
            Some(Intent::Zip { src: src(c, 1)?, output: cap(c, 2).and_then(|s| output(&s)) })
        }),
    ]
}

#[cfg(test)]
mod tests {
    use crate::{model::{FileType, Filter, Intent, Source}, parse::parse};

    #[test]
    fn explicit_zip_outputs_do_not_merge_pdfs_or_move_items() {
        let expected = Intent::Zip {
            src: Source::set(None, Filter { file_type: Some(FileType::Pdfs), name_contains: Some("scan".into()), ..Default::default() }),
            output: Some("out.zip".into()),
        };
        for command in [
            "Put the \"scan\" PDFs together into a zip called out.zip",
            "put PDFs named scan together as out.zip",
        ] {
            assert_eq!(parse(command).unwrap(), expected, "{command}");
        }
        assert_eq!(parse("put Project into a zip file named Project.zip").unwrap(),
            Intent::Zip { src: Source::item("Project"), output: Some("Project.zip".into()) });
        assert!(matches!(parse("put PDFs together into combined.pdf"), Ok(Intent::MergePdf { .. })));
        assert!(matches!(parse("move report.pdf to \"Archive.zip\""), Ok(Intent::Move { .. })));
    }

    #[test]
    fn unsupported_compression_and_hidden_selection_preserve_literal_names() {
        for command in ["compress all the photos in Pictures to make them smaller",
            "compress photo.png without losing quality", "remove hidden files from Downloads",
            "list hidden folders in Documents"] {
            assert!(parse(command).is_err(), "{command}");
        }
        assert!(matches!(parse("compress IMG_4421.heic"), Ok(Intent::Zip { .. })));
        assert_eq!(parse("zip \"to make them smaller\"").unwrap(),
            Intent::Zip { src: Source::item("to make them smaller"), output: None });
        assert!(parse("move \"hidden files\" to Documents").is_ok());
        assert!(parse("list files named hidden in Downloads").is_ok());
    }
}
