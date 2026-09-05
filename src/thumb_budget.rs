//! Memory budget for the thumbnail pipeline.
//!
//! What is left of the upstream `large_image` module after the gallery mode moved out to
//! wmde-photos. The viewer owns decoding now; the file manager only needs to decide whether a
//! picture fits the thumbnail budget and which worker should build it.

/// Bytes per pixel in RGBA format (Red, Green, Blue, Alpha = 4 bytes)
const RGBA_BYTES_PER_PIXEL: u64 = 4;

/// Side of one atlas fragment in iced, `MAX_SIZE` in
/// `libcosmic/iced/wgpu/src/image/atlas.rs`. A picture larger than this is split across
/// several atlas layers, one draw instance each. Upstream had 4096 here and a comment
/// claiming the two matched; they did not, and everything between 2049 and 4096 pixels was
/// treated as small.
const ATLAS_FRAGMENT_SIZE: u32 = 2048;

/// Conversion factor: 1 MB = 1024 * 1024 bytes (binary megabyte, used for RAM calculations)
const MB_TO_BYTES: u64 = 1024 * 1024;

/// Check if an image's dimensions would exceed the available memory budget.
/// Returns true if the image is too large to decode.
pub fn exceeds_memory_limit(width: u32, height: u32, memory_limit_mb: u64) -> bool {
    let Some(bytes_needed) = calculate_image_memory(width, height) else {
        // Overflow in calculation means it definitely exceeds any reasonable limit
        return true;
    };

    let max_bytes = memory_limit_mb * MB_TO_BYTES;
    bytes_needed > max_bytes
}
/// Check if an image should use GPU tiling for display.
/// Images larger than the atlas fragment size need to be split into tiles for GPU upload.
pub fn should_use_tiling(width: u32, height: u32) -> bool {
    width > ATLAS_FRAGMENT_SIZE || height > ATLAS_FRAGMENT_SIZE
}
/// Determine if an image should use the dedicated worker for thumbnail generation.
/// Returns (use_dedicated_worker, effective_max_mb, effective_jobs).
///
/// Large images that exceed per-worker memory budget get routed to a dedicated worker
/// with full memory budget. Smaller images use the normal parallel worker pool.
pub fn should_use_dedicated_worker(
    width: u32,
    height: u32,
    total_budget_mb: u64,
    parallel_workers: usize,
) -> (bool, u64, usize) {
    if width == 0 || height == 0 {
        log::warn!(
            "Invalid image dimensions {}x{}, using normal queue",
            width,
            height
        );
        return (false, total_budget_mb, parallel_workers);
    }

    let Some(bytes_needed) = calculate_image_memory(width, height) else {
        log::warn!(
            "Image dimensions {}x{} overflow memory calculation, using normal queue",
            width,
            height
        );
        return (false, total_budget_mb, parallel_workers);
    };

    let mb_needed = bytes_needed / MB_TO_BYTES;
    let per_worker_budget_mb = total_budget_mb / parallel_workers as u64;

    if mb_needed > per_worker_budget_mb {
        log::info!(
            "Large image {}x{} needs {}MB (exceeds per-worker {}MB), using dedicated worker",
            width,
            height,
            mb_needed,
            per_worker_budget_mb
        );
        // Use dedicated worker with full budget
        (true, total_budget_mb, 1)
    } else {
        log::debug!(
            "Normal image {}x{} needs {}MB (within per-worker {}MB), using parallel workers",
            width,
            height,
            mb_needed,
            per_worker_budget_mb
        );
        // Use parallel worker pool with shared budget
        (false, total_budget_mb, parallel_workers)
    }
}
/// Calculate the memory required to decode an image in bytes.
/// Returns None if the calculation overflows.
fn calculate_image_memory(width: u32, height: u32) -> Option<u64> {
    let pixels = (width as u64).checked_mul(height as u64)?;
    pixels.checked_mul(RGBA_BYTES_PER_PIXEL)
}
