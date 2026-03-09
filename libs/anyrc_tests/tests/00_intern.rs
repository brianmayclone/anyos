use anyrc::intern::Interner;

#[test]
fn intern_returns_same_symbol_for_same_string() {
    let mut interner = Interner::new();
    let s1 = interner.intern("hello");
    let s2 = interner.intern("hello");
    assert_eq!(s1, s2);
}

#[test]
fn intern_returns_different_symbols_for_different_strings() {
    let mut interner = Interner::new();
    let s1 = interner.intern("hello");
    let s2 = interner.intern("world");
    assert_ne!(s1, s2);
}

#[test]
fn resolve_returns_original_string() {
    let mut interner = Interner::new();
    let sym = interner.intern("test_string");
    assert_eq!(interner.resolve(sym), "test_string");
}
