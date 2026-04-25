use anyrc::loader::{
    deserialize_metadata, serialize_metadata, CrateInterface, CrateMetadata, ExportKind,
    ExportedSymbol, InterfaceItem, InterfaceItemKind,
};
use anyrc::{
    driver::{compile, CompileOptions, CrateType, EmitKind},
    loader::unpack_rlib,
};

#[test]
fn metadata_roundtrips_structured_interface() {
    let meta = CrateMetadata {
        name: "sample".to_string(),
        version: "0.1.0".to_string(),
        exports: vec![ExportedSymbol {
            name: "answer".to_string(),
            kind: ExportKind::Function,
        }],
        deps: vec!["core".to_string()],
        interface_source: "pub fn answer() -> i32;\n".to_string(),
        interface: CrateInterface {
            items: vec![InterfaceItem {
                name: "answer".to_string(),
                kind: InterfaceItemKind::Function,
                signature: "pub fn answer() -> i32;\n".to_string(),
            }],
        },
    };

    let encoded = serialize_metadata(&meta);
    let decoded = deserialize_metadata(&encoded).expect("metadata should decode");

    assert_eq!(decoded.name, "sample");
    assert_eq!(decoded.interface.items.len(), 1);
    assert_eq!(decoded.interface.items[0].name, "answer");
    assert!(matches!(
        decoded.interface.items[0].kind,
        InterfaceItemKind::Function
    ));
    assert!(decoded.interface.items[0].signature.contains("answer"));
}

#[test]
fn metadata_without_structured_interface_still_loads() {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(b"ARCM");
    push_str16(&mut encoded, "legacy");
    push_str16(&mut encoded, "0.1.0");
    encoded.extend_from_slice(&0u16.to_le_bytes());
    encoded.extend_from_slice(&0u16.to_le_bytes());
    let iface = "pub struct Legacy;\n";
    encoded.extend_from_slice(&(iface.len() as u32).to_le_bytes());
    encoded.extend_from_slice(iface.as_bytes());

    let decoded = deserialize_metadata(&encoded).expect("legacy metadata should decode");

    assert_eq!(decoded.name, "legacy");
    assert_eq!(decoded.interface_source, iface);
    assert!(decoded.interface.items.is_empty());
}

#[test]
fn rlib_emit_includes_structured_public_interface() {
    let source = r#"
        pub struct PublicThing {
            pub value: i32,
        }

        struct PrivateThing {
            value: i32,
        }

        pub fn answer() -> i32 { 42 }
    "#;
    let options = CompileOptions {
        input: "lib.rs".to_string(),
        output: "libsample.rlib".to_string(),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("sample".to_string()),
        ..CompileOptions::default()
    };

    let rlib = compile(source, "lib.rs", &options).expect("rlib compilation should succeed");
    let (_, meta) = unpack_rlib(&rlib).expect("rlib should unpack");

    assert!(meta
        .interface
        .items
        .iter()
        .any(|item| item.name == "PublicThing"
            && matches!(item.kind, InterfaceItemKind::Struct)));
    assert!(meta
        .interface
        .items
        .iter()
        .any(|item| item.name == "answer" && matches!(item.kind, InterfaceItemKind::Function)));
    assert!(!meta
        .interface
        .items
        .iter()
        .any(|item| item.name == "PrivateThing"));
}

#[test]
fn rlib_interface_exports_conversion_and_comparison_trait_impls() {
    let source = r#"
        pub trait From<T> {
            fn from(value: T) -> Self;
        }

        pub trait PartialEq<Rhs> {
            fn eq(&self, rhs: &Rhs) -> bool;
        }

        pub struct TokenTree;
        pub struct Ident;

        impl From<Ident> for TokenTree {
            fn from(value: Ident) -> Self {
                TokenTree
            }
        }

        impl<T> PartialEq<T> for Ident {
            fn eq(&self, rhs: &T) -> bool {
                true
            }
        }
    "#;
    let options = CompileOptions {
        input: "lib.rs".to_string(),
        output: "libsample.rlib".to_string(),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("sample".to_string()),
        ..CompileOptions::default()
    };

    let rlib = compile(source, "lib.rs", &options).expect("rlib compilation should succeed");
    let (_, meta) = unpack_rlib(&rlib).expect("rlib should unpack");

    assert!(meta.interface_source.contains("impl From<Ident> for TokenTree"));
    assert!(meta.interface_source.contains("impl<T> PartialEq<T> for Ident"));
}

fn push_str16(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}
