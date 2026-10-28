//! PDF tools on top of macOS PDFKit, called in-process via objc2.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod imp {
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSString, NSURL};
    use objc2_pdf_kit::{PDFDocument, PDFPage};
    use std::path::Path;

    fn url(p: &Path) -> Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
    }

    pub fn open(p: &Path) -> Result<Retained<PDFDocument>, String> {
        unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url(p)) }
            .ok_or_else(|| format!("Couldn't read {} as a PDF", p.display()))
    }

    pub fn new_doc() -> Retained<PDFDocument> {
        unsafe { PDFDocument::new() }
    }

    pub fn page_count(doc: &PDFDocument) -> usize {
        unsafe { doc.pageCount() }
    }

    /// All pages of `doc`, collected up front: PDFKit may move a page out of
    /// its document when it is inserted elsewhere, which shifts indexes.
    pub fn pages(doc: &PDFDocument) -> Vec<Retained<PDFPage>> {
        (0..page_count(doc)).filter_map(|i| unsafe { doc.pageAtIndex(i) }).collect()
    }

    pub fn append(dst: &PDFDocument, page: &PDFPage) {
        unsafe { dst.insertPage_atIndex(page, dst.pageCount()) };
    }

    pub fn append_image(dst: &PDFDocument, image: &Path) -> Result<(), String> {
        let img = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(&image.to_string_lossy()))
            .ok_or_else(|| format!("Couldn't read image {}", image.display()))?;
        let page = unsafe { PDFPage::initWithImage(PDFPage::alloc(), &img) }
            .ok_or_else(|| format!("Couldn't add {} to the PDF", image.display()))?;
        unsafe { dst.insertPage_atIndex(&page, dst.pageCount()) };
        Ok(())
    }

    pub fn write(doc: &PDFDocument, out: &Path) -> Result<(), String> {
        if unsafe { doc.writeToURL(&url(out)) } {
            Ok(())
        } else {
            Err(format!("Couldn't write {}", out.display()))
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    compile_error!("Voxi v1 is macOS only");
}

pub fn page_count(path: &Path) -> Result<usize, String> {
    let doc = imp::open(path)?;
    Ok(imp::page_count(&doc))
}

pub fn merge(paths: &[PathBuf], output: &Path) -> Result<(), String> {
    let out = imp::new_doc();
    for p in paths {
        let doc = imp::open(p)?;
        for page in imp::pages(&doc) {
            imp::append(&out, &page);
        }
    }
    imp::write(&out, output)
}

/// Writes pages `from..=to` (1-based) of `path` to `output`.
pub fn extract(path: &Path, from: u32, to: u32, output: &Path) -> Result<(), String> {
    let doc = imp::open(path)?;
    let n = imp::page_count(&doc);
    if from == 0 || to as usize > n || from > to {
        return Err(format!("{} has {n} pages", path.display()));
    }
    let out = imp::new_doc();
    for page in &imp::pages(&doc)[(from - 1) as usize..to as usize] {
        imp::append(&out, page);
    }
    imp::write(&out, output)
}

/// Writes one PDF per page into `out_dir` (which must already exist).
/// Returns the files written.
pub fn split(path: &Path, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "page".into());
    let doc = imp::open(path)?;
    let pages = imp::pages(&doc);
    let width = pages.len().to_string().len().max(2);
    let mut written = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let out = imp::new_doc();
        imp::append(&out, page);
        let file = out_dir.join(format!("{stem} page {:0width$}.pdf", i + 1));
        imp::write(&out, &file)?;
        written.push(file);
    }
    Ok(written)
}

pub fn images_to_pdf(images: &[PathBuf], output: &Path) -> Result<(), String> {
    let out = imp::new_doc();
    for img in images {
        imp::append_image(&out, img)?;
    }
    imp::write(&out, output)
}
