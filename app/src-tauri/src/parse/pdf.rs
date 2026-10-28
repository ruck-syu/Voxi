//! PDF merge, split, page extraction, and images to PDF.

use super::{Pattern, cap, pat, phrase::clean_name, typed_src};
use crate::model::{FileType, Intent};

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "extract_pages",
            r"^(?:extract|pull\s+out|take\s+out|copy)\s+pages?\s+(\d+)\s*(?:to|through|thru|-|–|until)\s*(\d+)\s+(?:from|of|out\s+of|in)\s+(.+?)(?:\s+(?:into|as|to)\s+(.+))?$",
            |c| {
                let (from, to): (u32, u32) = (cap(c, 1)?.parse().ok()?, cap(c, 2)?.parse().ok()?);
                Some(Intent::ExtractPages {
                    src: typed_src(c, 3, FileType::Pdfs)?,
                    from,
                    to,
                    output: cap(c, 4).map(|s| clean_name(&s)),
                })
            },
        ),
        pat(
            "extract_page",
            r"^(?:extract|pull\s+out|take\s+out)\s+page\s+(\d+)\s+(?:from|of|out\s+of|in)\s+(.+?)(?:\s+(?:into|as|to)\s+(.+))?$",
            |c| {
                let n: u32 = cap(c, 1)?.parse().ok()?;
                Some(Intent::ExtractPages { src: typed_src(c, 2, FileType::Pdfs)?, from: n, to: n, output: cap(c, 3).map(|s| clean_name(&s)) })
            },
        ),
        pat("split_pdf", r"^split\s+(.+?)(?:\s+(?:into|to|by)\s+(?:separate\s+|individual\s+|single\s+)?pages?)?$", |c| {
            Some(Intent::SplitPdf { src: typed_src(c, 1, FileType::Pdfs)? })
        }),
        pat(
            "images_to_pdf",
            r"^(?:make|create|build|generate)\s+(?:a\s+)?(?:single\s+)?pdf\s+(?:from|of|out\s+of|with)\s+(.+?)(?:\s+(?:into|as|called)\s+(.+\.pdf))?$",
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
                output: cap(c, 2).map(|s| clean_name(&s)).unwrap_or_else(|| "merged.pdf".into()),
            })
        }),
    ]
}
