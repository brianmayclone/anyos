# GDB script for debugging anyOS freezes
# Usage: gdb -x scripts/debug-freeze.gdb
#
# 1. Start QEMU: ninja -C build run-vmware-debug
# 2. When the system freezes, in another terminal: gdb -x scripts/debug-freeze.gdb
# 3. The script connects and dumps all CPU states automatically.

set pagination off
set confirm off

# Connect to QEMU's GDB server
target remote :1234

# Load kernel symbols
symbol-file build/kernel/x86_64-anyos/release/anyos_kernel.elf

# Show all CPU threads
echo \n=== ALL CPU THREADS ===\n
info threads

# Dump registers for each CPU (use eflags, not rflags — GDB naming)
echo \n=== CPU STATE DUMP ===\n
thread apply all info registers rip rsp rbp eflags cr3

echo \n=== BACKTRACES ===\n
thread apply all bt 20

echo \n=== DONE - Use 'thread N' to inspect individual CPUs ===\n
echo === Use 'x/20gx $rsp' to dump stack, 'disas $rip,+32' for code ===\n
