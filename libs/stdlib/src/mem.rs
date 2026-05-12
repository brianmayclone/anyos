#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    for i in 0..n {
        let byte = src.add(i).read_volatile();
        dest.add(i).write_volatile(byte);
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let dest_addr = dest as usize;
    let src_addr = src as usize;
    if dest_addr <= src_addr || dest_addr >= src_addr.saturating_add(n) {
        for i in 0..n {
            let byte = src.add(i).read_volatile();
            dest.add(i).write_volatile(byte);
        }
    } else {
        let mut i = n;
        while i != 0 {
            i -= 1;
            let byte = src.add(i).read_volatile();
            dest.add(i).write_volatile(byte);
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut u8, value: i32, n: usize) -> *mut u8 {
    let byte = value as u8;
    for i in 0..n {
        dest.add(i).write_volatile(byte);
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let a = *left.add(i);
        let b = *right.add(i);
        if a != b {
            return a as i32 - b as i32;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn bcmp(left: *const u8, right: *const u8, n: usize) -> i32 {
    memcmp(left, right, n)
}
