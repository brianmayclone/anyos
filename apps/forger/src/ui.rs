use alloc::format;
use alloc::string::String;

/// Format stats into a window title string.
pub fn format_title(fps: u32, x: f32, y: f32, z: f32, block_name: &str, fog_chunks: f32) -> String {
    format!(
        "Forger | FPS: {} | Pos: {:.1}, {:.1}, {:.1} | Block: {} | View: {:.0} chunks",
        fps, x, y, z, block_name, fog_chunks
    )
}
