//! The WebAssembly interface: plain exports rather than wasm-bindgen.
//!
//! Exported functions take and return offsets into this module's own memory,
//! so the build needs nothing but cargo and a wasm32 target -- no bindgen CLI
//! whose version has to match the crate's -- and the loader on the other side
//! is a few lines of JavaScript. See editor.js in the shell.
//!
//! One convention: the caller allocates, writes UTF-8 into this module's
//! memory, calls `compile` or `title_of`, and reads the result back out of it.
//! Both return the length; `output_ptr` says where it starts and `ok` whether
//! it is a document or a diagnostic.
//!
//! Which renderer `compile` is depends on which feature this module was built
//! with, so markdown.wasm and typst.wasm share a loader and differ only in
//! what they do with a source.

/// Where the last result lives until the next call replaces it.
static mut OUTPUT: Option<Vec<u8>> = None;
static mut OK: bool = false;
#[cfg(feature = "typst")]
static mut TODAY: Option<crate::typst::Today> = None;

/// Reserves `len` bytes for the caller to write a source into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Releases what `alloc` reserved. The caller frees the source it wrote; the
/// output belongs to this module and is replaced on the next call.
///
/// # Safety
/// `pointer` and `len` must be exactly what a previous `alloc` returned.
#[no_mangle]
pub unsafe extern "C" fn dealloc(pointer: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(pointer, 0, len));
}

unsafe fn text_at<'a>(pointer: *const u8, len: usize) -> &'a str {
    if pointer.is_null() || len == 0 {
        return "";
    }
    std::str::from_utf8(std::slice::from_raw_parts(pointer, len)).unwrap_or("")
}

unsafe fn answer(result: Result<String, String>) -> usize {
    let (ok, text) = match result {
        Ok(html) => (true, html),
        Err(message) => (false, message),
    };
    let bytes = text.into_bytes();
    let length = bytes.len();
    OUTPUT = Some(bytes);
    OK = ok;
    length
}

/// Renders the `source_len` bytes of UTF-8 at `source` into the page a save
/// would store, titled with the `title_len` bytes at `title`, and returns the
/// length of the result. Read it from `output_ptr()`, and ask `ok()` whether it
/// is a document or the reason it is not.
///
/// # Safety
/// The pointers and lengths must describe UTF-8 written into this module's
/// memory.
#[no_mangle]
pub unsafe extern "C" fn compile(
    source: *const u8,
    source_len: usize,
    title: *const u8,
    title_len: usize,
) -> usize {
    let source = text_at(source, source_len);
    let title = text_at(title, title_len);
    answer(render(source, title))
}

#[cfg(feature = "typst")]
fn render(source: &str, title: &str) -> Result<String, String> {
    let today = unsafe { *std::ptr::addr_of!(TODAY) };
    crate::typst::render(source, title, "", &crate::typst::no_files, today)
}

#[cfg(all(feature = "markdown", not(feature = "typst")))]
fn render(source: &str, title: &str) -> Result<String, String> {
    Ok(crate::markdown::render(source, title))
}

/// The document's first heading, which names a document that was never given
/// a title of its own. Returned the same way `compile` returns its page.
///
/// # Safety
/// `source` and `len` must describe UTF-8 written into this module's memory.
#[no_mangle]
pub unsafe extern "C" fn title_of(source: *const u8, len: usize) -> usize {
    answer(Ok(heading(text_at(source, len))))
}

#[cfg(feature = "typst")]
fn heading(source: &str) -> String {
    crate::typst::title_of(source)
}

#[cfg(all(feature = "markdown", not(feature = "typst")))]
fn heading(source: &str) -> String {
    crate::markdown::title_of(source)
}

/// Tells the compiler what day it is, for `datetime.today()`: the engine has
/// no clock, so the browser hands it one. Ignored by the markdown module.
#[no_mangle]
pub extern "C" fn set_today(year: i32, month: u32, day: u32) {
    #[cfg(feature = "typst")]
    unsafe {
        TODAY = Some(crate::typst::Today {
            year,
            month: month as u8,
            day: day as u8,
        });
    }
    #[cfg(not(feature = "typst"))]
    let _ = (year, month, day);
}

/// Where the last result starts.
#[no_mangle]
pub extern "C" fn output_ptr() -> *const u8 {
    unsafe {
        match &*std::ptr::addr_of!(OUTPUT) {
            Some(bytes) => bytes.as_ptr(),
            None => std::ptr::null(),
        }
    }
}

/// Whether the last result is a document (1) or a diagnostic (0).
#[no_mangle]
pub extern "C" fn ok() -> u32 {
    unsafe { u32::from(*std::ptr::addr_of!(OK)) }
}
