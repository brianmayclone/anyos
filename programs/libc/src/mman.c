#include <sys/mman.h>
#include <errno.h>

/* Stub implementations — anyOS doesn't support mmap yet */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, long offset) {
    (void)addr; (void)length; (void)prot; (void)flags; (void)fd; (void)offset;
    errno = ENOSYS;
    return MAP_FAILED;
}

int munmap(void *addr, size_t length) {
    (void)addr; (void)length;
    errno = ENOSYS;
    return -1;
}

int mprotect(void *addr, size_t length, int prot) {
    (void)addr; (void)length; (void)prot;
    errno = ENOSYS;
    return -1;
}
