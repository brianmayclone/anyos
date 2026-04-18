use anyos_std::ipc;
use anyos_std::permissions;
use anyos_std::process;

const PERM_DIALOG_PATH: &str = "/System/permdialog";

/// Launch an app from a shared memory path buffer.
///
/// The SHM contains: "path\x1Fargs" (args optional).
/// Returns the spawned TID, or u32::MAX on failure.
pub fn launch_app(shm_id: u32) -> u32 {
    anyos_std::println!("[sessionhost/launch] start shm_id={}", shm_id);
    if shm_id == 0 {
        anyos_std::println!("[sessionhost/launch] invalid shm_id=0");
        return u32::MAX;
    }

    // Map the shared memory to read the path
    let addr = ipc::shm_map(shm_id);
    if addr == 0 {
        anyos_std::println!("[sessionhost/launch] shm_map failed shm_id={}", shm_id);
        return u32::MAX;
    }
    anyos_std::println!("[sessionhost/launch] shm mapped addr=0x{:x}", addr);

    // Read path + args from SHM (max 512 bytes)
    let mut buf = [0u8; 512];
    let shm_ptr = addr as *const u8;
    let mut len = 0usize;
    for i in 0..512 {
        let b = unsafe { shm_ptr.add(i).read_volatile() };
        if b == 0 {
            break;
        }
        buf[i] = b;
        len = i + 1;
    }

    ipc::shm_unmap(shm_id);
    ipc::shm_destroy(shm_id);

    if len == 0 {
        anyos_std::println!("[sessionhost/launch] empty request");
        return u32::MAX;
    }

    // Split on \x1F: "path\x1Fargs"
    let sep = buf[..len].iter().position(|&b| b == 0x1F);
    let (path_bytes, args_bytes) = match sep {
        Some(pos) => (&buf[..pos], &buf[pos + 1..len]),
        None => (&buf[..len], &[] as &[u8]),
    };

    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("[sessionhost/launch] invalid utf8 path");
            return u32::MAX;
        }
    };
    let args = core::str::from_utf8(args_bytes).unwrap_or("");
    anyos_std::println!(
        "[sessionhost/launch] spawn path='{}' args='{}'",
        path, args
    );

    // Try to spawn
    let tid = process::spawn_piped(path, args, 0);
    anyos_std::println!("[sessionhost/launch] spawn_piped returned {}", tid);

    if tid == permissions::PERM_NEEDED {
        anyos_std::println!("[sessionhost/launch] permission required");
        // Read pending info and launch PermDialog
        let mut info_buf = [0u8; 512];
        let info_len = permissions::perm_pending_info(&mut info_buf);
        if info_len == 0 {
            anyos_std::println!("[sessionhost/launch] perm_pending_info empty");
            return u32::MAX;
        }
        let info = match core::str::from_utf8(&info_buf[..info_len as usize]) {
            Ok(s) => s,
            Err(_) => {
                anyos_std::println!("[sessionhost/launch] invalid permission info utf8");
                return u32::MAX;
            }
        };

        let dialog_tid = process::spawn_piped(PERM_DIALOG_PATH, info, 0);
        anyos_std::println!("[sessionhost/launch] permdialog tid={}", dialog_tid);
        if dialog_tid == u32::MAX || dialog_tid == permissions::PERM_NEEDED {
            return u32::MAX;
        }

        // Wait for user decision
        let exit_code = process::waitpid(dialog_tid);
        anyos_std::println!("[sessionhost/launch] permdialog exit={}", exit_code);
        if exit_code != 0 {
            return u32::MAX; // User denied
        }

        // Retry — permissions should now be stored
        let retry_tid = process::spawn_piped(path, args, 0);
        anyos_std::println!("[sessionhost/launch] retry spawn returned {}", retry_tid);
        return retry_tid;
    }

    anyos_std::println!("[sessionhost/launch] done tid={}", tid);
    tid
}
