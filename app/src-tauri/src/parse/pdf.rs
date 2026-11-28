//! PDF merge, split, page extraction, and images to PDF.

use super::{Pattern, cap, pat, phrase::clean_name, typed_src};
use crate::model::{FileType, Intent};
use regex::Regex;
use std::sync::LazyLock;

static OUTPUT_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:(?:a|an|one|1)(?:\s+single)?|single)\s+)?(?:(?:file|pdf)\s*(?:,\s*|(?:called|named)\s+)|(?:called|named)\s+)")
        .unwrap()
});

/// "into one out.pdf" -> "out.pdf"
static COUNT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:an?\s+single|single|one|1)\s+(\S+\.[a-z0-9]{1,5})$").unwrap());

fn output_name(text: &str) -> String {
    clean_name(&COUNT_WORD.replace(&OUTPUT_LABEL.replace(text, ""), "$1"))
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("put_pdfs_together", r"^put\s+(.+?)\s+together\s+(?:as|into)\s+(.+)$", |c| {
            let output = output_name(&cap(c, 2)?);
            if output.to_ascii_lowercase().ends_with(".zip") {
                return Some(Intent::Zip { src: super::src(c, 1)?, output: Some(output) });
            }
            let src = typed_src(c, 1, FileType::Pdfs)?;
            if !matches!(src, crate::model::Source::Set { .. }) {
                return None;
            }
            Some(Intent::MergePdf { src, output })
        }),
        pat(
            "extract_pages",
            r"^(?:extract|pull\s+out|take\s+out|copy)\s+pages?\s+(\d+)\s*(?:to|through|thru|-|–|until)\s*(\d+)\s+(?:from|of|out\s+of|in)\s+(.+?)(?:\s+(?:into|as|to)\s+(.+))?$",
            |c| {
                let (from, to): (u32, u32) = (cap(c, 1)?.parse().ok()?, cap(c, 2)?.parse().ok()?);
                Some(Intent::ExtractPages { src: typed_src(c, 3, FileType::Pdfs)?, from, to, output: cap(c, 4).map(|s| clean_name(&s)) })
            },
        ),
        pat(
            "extract_page",
            r"^(?:extract|pull\s+out|take\s+out)\s+page\s+(\d+)\s+(?:from|of|out\s+of|in)\s+(.+?)(?:\s+(?:into|as|to)\s+(.+))?$",
            |c| {
                let n: u32 = cap(c, 1)?.parse().ok()?;
                Some(Intent::ExtractPages {
                    src: typed_src(c, 2, FileType::Pdfs)?,
                    from: n,
                    to: n,
                    output: cap(c, 3).map(|s| clean_name(&s)),
                })
            },
        ),
        pat(
            "split_pdf",
            r"^split\s+(?:up\s+)?(.+?)(?:\s+(?:into|to|by)\s+(?:(?:(?:its|their)\s+|(?:separate|individual|single)\s+|(?:one|1)\s+(?:file\s+)?per\s+)?pages?|(?:one|1)\s+page\s+each))?$",
            |c| Some(Intent::SplitPdf { src: typed_src(c, 1, FileType::Pdfs)? }),
        ),
        pat(
            "images_to_pdf",
            r"^(?:(?:make|create|build|generate)\s+(?:a\s+)?|create\s+1\s+)(?:single\s+)?pdf\s+(?:from|of|out\s+of|with|using)\s+(.+?)(?:\s+(?:into|as|called)\s+(.+\.pdf))?$",
            |c| Some(Intent::ImagesToPdf { src: typed_src(c, 1, FileType::Images)?, output: cap(c, 2).map(|s| clean_name(&s)) }),
        ),
        pat(
            "images_into_pdf",
            r"^(?:turn|convert|combine)\s+(.+?)\s+(?:into|to)\s+(?:a\s+)?(?:single\s+)?pdf(?:\s+(?:called|named)\s+(.+))?$",
            |c| Some(Intent::ImagesToPdf { src: typed_src(c, 1, FileType::Images)?, output: cap(c, 2).map(|s| clean_name(&s)) }),
        ),
        pat("merge_pdf", r"^(?:merge|combine|join)\s+(.+?)(?:\s+(?:into|as|to)\s+(.+))?$", |c| {
            Some(Intent::MergePdf {
                src: typed_src(c, 1, FileType::Pdfs)?,
                output: cap(c, 2).map(|s| output_name(&s)).unwrap_or_else(|| "merged.pdf".into()),
            })
        }),
    ]
}
