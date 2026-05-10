use super::*;

pub(super) fn linux_shmget(_key: u64, _size: u64, _flags: u64) -> u64 {
    linux_err(ENOSYS)
}

pub(super) fn linux_shmat(_shmid: u64, _shmaddr: u64, _flags: u64) -> u64 {
    linux_err(EINVAL)
}

pub(super) fn linux_shmctl(_shmid: u64, _cmd: u64, _buf: u64) -> u64 {
    linux_err(EINVAL)
}

pub(super) fn linux_shmdt(_shmaddr: u64) -> u64 {
    linux_err(EINVAL)
}

pub(super) fn linux_semget(_key: u64, _nsems: u64, _flags: u64) -> u64 {
    linux_err(ENOSYS)
}

pub(super) fn linux_semop(_semid: u64, _sops: u64, _nsops: u64) -> u64 {
    linux_err(EINVAL)
}

pub(super) fn linux_semctl(_semid: u64, _semnum: u64, _cmd: u64, _arg: u64) -> u64 {
    linux_err(EINVAL)
}

pub(super) fn linux_msgget(_key: u64, _flags: u64) -> u64 {
    linux_err(ENOSYS)
}

pub(super) fn linux_msgsnd(_msqid: u64, msgp: u64, msgsz: u64, _flags: u64) -> u64 {
    if msgp == 0 || !handlers::helpers::is_user_range_accessible(msgp, msgsz.saturating_add(8)) {
        return linux_err(EFAULT);
    }
    linux_err(EINVAL)
}

pub(super) fn linux_msgrcv(_msqid: u64, msgp: u64, msgsz: u64, _msgtyp: u64, _flags: u64) -> u64 {
    if msgp == 0 || !handlers::helpers::is_user_range_accessible(msgp, msgsz.saturating_add(8)) {
        return linux_err(EFAULT);
    }
    linux_err(ENOMSG)
}

pub(super) fn linux_msgctl(_msqid: u64, _cmd: u64, _buf: u64) -> u64 {
    linux_err(EINVAL)
}
