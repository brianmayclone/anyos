use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::icons;
use crate::types::{IconEntry, ICON_SIZE};

pub fn load_app_icon(name: &str) -> Vec<u32> {
    let bin_path = {
        let app_path = alloc::format!("/Applications/{}.app", name);
        let mut stat_buf = [0u32; 7];
        if fs::stat(&app_path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            app_path
        } else {
            alloc::format!("/System/bin/{}", name)
        }
    };
    let icon_path = icons::app_icon_path(&bin_path);

    let fd = fs::open(&icon_path, 0);
    if fd == u32::MAX { return Vec::new(); }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    if data.is_empty() { return Vec::new(); }

    let info = match libimage_client::probe_ico_size(&data, ICON_SIZE) {
        Some(i) => i,
        None => match libimage_client::probe(&data) {
            Some(i) => i,
            None => return Vec::new(),
        },
    };

    let src_w = info.width;
    let src_h = info.height;
    let mut pixels = vec![0u32; (src_w * src_h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    let ok = if info.format == libimage_client::FMT_ICO {
        libimage_client::decode_ico_size(&data, ICON_SIZE, &mut pixels, &mut scratch).is_ok()
    } else {
        libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok()
    };
    if !ok { return Vec::new(); }

    if src_w == ICON_SIZE && src_h == ICON_SIZE { return pixels; }

    let mut dst = vec![0u32; (ICON_SIZE * ICON_SIZE) as usize];
    libimage_client::scale_image(
        &pixels, src_w, src_h,
        &mut dst, ICON_SIZE, ICON_SIZE,
        libimage_client::MODE_SCALE,
    );
    dst
}

pub fn ensure_icon_cached(cache: &mut Vec<IconEntry>, name: &str) {
    if cache.iter().any(|e| e.name == name) { return; }
    let pixels = load_app_icon(name);
    cache.push(IconEntry { name: String::from(name), pixels });
}

pub fn find_icon<'a>(cache: &'a [IconEntry], name: &str) -> Option<&'a [u32]> {
    cache.iter()
        .find(|e| e.name == name)
        .map(|e| e.pixels.as_slice())
        .filter(|p| !p.is_empty())
}
