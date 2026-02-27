fn main() {
    // No special linker script needed — anyld handles the final layout.
    // Cargo produces a .a static archive; anyld links it into an ET_DYN .so.
}
