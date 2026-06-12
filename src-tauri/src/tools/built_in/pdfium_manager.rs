use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;

/// Locate the bundled pdfium library in the app's resource directory.
/// Uses pdfium-render's built-in platform detection for the correct filename.
pub fn find_library(resource_dir: &Path) -> Option<PathBuf> {
    let lib_name = Pdfium::pdfium_platform_library_name_at_path(
        resource_dir.to_str().unwrap_or(".")
    );
    let path = PathBuf::from(&lib_name);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn extract_text_with_pdfium(
    lib_path: &Path,
    pdf_bytes: &[u8],
) -> Result<(String, u32), String> {
    let bindings = Pdfium::bind_to_library(lib_path)
        .map_err(|e| format!("failed to load pdfium: {e}"))?;
    let pdfium = Pdfium::new(bindings);

    let doc = pdfium
        .load_pdf_from_byte_slice(pdf_bytes, None)
        .map_err(|e| format!("failed to open PDF: {e}"))?;

    let page_count = doc.pages().len() as u32;
    let mut text = String::new();

    for (i, page) in doc.pages().iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        match page.text() {
            Ok(page_text) => text.push_str(&page_text.all()),
            Err(e) => text.push_str(&format!("[Page {} error: {e}]", i + 1)),
        }
    }

    Ok((text, page_count))
}
