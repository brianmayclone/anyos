//! Kernel syscall wrappers for libzip.

use core::arch::asm;

const SYS_EXIT: u64 = 1;
const SYS_WRITE: u64 = 2;
const SYS_READ: u64 = 3;
const SYS_OPEN: u64 = 4;
const SYS_CLOSE: u64 = 5;
const SYS_SBRK: u64 = 9;
const SYS_MKDIR: u64 = 11;
const SYS_LSEEK: u64 = 105;
const SYS_FSTAT: u64 = 106;
const SYS_READDIR: u64 = 7;
const SYS_STAT: u64 = 10;

pub const O_WRITE: u32 = 1;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;
pub const SEEK_SET: u32 = 0;

#[inline(always)]
fn syscall1(num: u64, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall2(num: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            in("r10") a2,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            in("r10") a2, in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn close(fd: u32) {
    syscall1(SYS_CLOSE, fd as u64);
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    let ret = syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    let ret = syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn file_size(fd: u32) -> u32 {
    let mut stat_buf = [0u32; 4];
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { 0 } else { stat_buf[1] }
}

pub fn mkdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall1(SYS_MKDIR, buf.as_ptr() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn stat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall2(SYS_STAT, buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    let ret = syscall3(SYS_READDIR, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

pub fn log(msg: &[u8]) {
    write(1, msg);
}
