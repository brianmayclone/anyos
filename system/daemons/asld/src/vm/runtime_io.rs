use crate::errors::AsldError;

const PAGE_SIZE: usize = 0x1000;
const MIN_GUEST_MEMORY_MB: usize = 16;

pub(super) fn align_guest_memory_size(memory_mb: u32) -> usize {
    let requested = (memory_mb as usize).max(MIN_GUEST_MEMORY_MB) * 1024 * 1024;
    (requested + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1)
}

pub(super) fn ensure_pipe(pipe_name: &str) -> Result<(), AsldError> {
    let existing = anyos_std::ipc::pipe_open(pipe_name);
    if existing != 0 && existing != u32::MAX {
        let _ = anyos_std::ipc::pipe_close(existing);
    }
    let created = anyos_std::ipc::pipe_create(pipe_name);
    if created == 0 || created == u32::MAX {
        return Err(AsldError::BackendUnavailable(
            "console pipe provisioning failed",
        ));
    }
    Ok(())
}
