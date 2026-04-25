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

#[test]
fn rlib_interface_exports_iterator_associated_item_impls() {
    let source = r#"
        pub enum Option<T> {
            Some(T),
            None,
        }

        pub trait Iterator {
            type Item;
            fn next(&mut self) -> Option<Self::Item>;
        }

        pub struct Values<K, V> {
            marker: K,
            value: V,
        }

        impl<K, V> Iterator for Values<K, V> {
            type Item = &V;
            fn next(&mut self) -> Option<Self::Item> {
                None
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

    assert!(meta.interface_source.contains("impl<K, V> Iterator for Values<K, V>"));
    assert!(meta.interface_source.contains("type Item = &V;"));
}

#[test]
fn rlib_interface_omits_inherent_impls_for_private_local_types() {
    let source = r#"
        mod inner {
            pub struct PublicThing;

            struct PrivateHelper;

            impl PublicThing {
                pub fn new() -> Self { PublicThing }
            }

            impl PrivateHelper {
                pub fn leaked() -> Self { PrivateHelper }
            }
        }

        pub use inner::PublicThing;
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

    assert!(meta.interface_source.contains("impl PublicThing"));
    assert!(!meta.interface_source.contains("impl PrivateHelper"));
}

#[test]
fn rlib_interface_includes_items_from_loaded_macro_rules_module() {
    let unique = format!(
        "anyrc_meta_macro_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("crate_root.rs"),
        r#"
            macro_rules! crate_root {
                () => {
                    pub mod de {
                        pub trait Deserialize<'de> {}
                        pub trait Visitor<'de> {}
                    }

                    pub mod ser {
                        pub trait Serialize {}
                    }

                    pub use crate::de::{Deserialize, Visitor};
                    pub use crate::ser::Serialize;
                };
            }
        "#,
    )
    .unwrap();
    let root = r#"
        #[macro_use]
        mod crate_root;

        crate_root!();
    "#;
    std::fs::write(dir.join("lib.rs"), root).unwrap();

    let options = CompileOptions {
        input: dir.join("lib.rs").to_string_lossy().into_owned(),
        output: dir.join("libsample.rlib").to_string_lossy().into_owned(),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("sample".to_string()),
        src_dir: Some(dir.to_string_lossy().into_owned()),
        ..CompileOptions::default()
    };

    let rlib = compile(root, "lib.rs", &options).expect("rlib compilation should succeed");
    let (_, meta) = unpack_rlib(&rlib).expect("rlib should unpack");

    assert!(meta.interface_source.contains("pub mod de"));
    assert!(meta.interface_source.contains("pub trait Deserialize"));
    assert!(meta.interface_source.contains("pub trait Visitor"));
    assert!(meta.interface_source.contains("pub trait Serialize"));
    assert!(meta
        .interface_source
        .contains("pub use crate::de::{Deserialize, Visitor};"));
    assert!(meta.interface_source.contains("pub use crate::ser::Serialize;"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rlib_interface_resolves_modules_created_by_macro_expansion() {
    let unique = format!(
        "anyrc_meta_macro_files_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("crate_root.rs"),
        r#"
            macro_rules! crate_root {
                () => {
                    macro_rules! tri {
                        ($expr:expr) => {
                            match $expr {
                                Ok(value) => value,
                                Err(err) => return Err(err),
                            }
                        };
                    }

                    pub mod de;
                    pub mod ser;

                    pub use crate::de::{Deserialize, Visitor};
                    pub use crate::ser::Serialize;
                };
            }
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("de.rs"),
        r#"
            pub trait Deserialize<'de> {}
            pub trait Visitor<'de> {}
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("ser.rs"),
        r#"
            pub trait Serialize {}
        "#,
    )
    .unwrap();
    let root = r#"
        #[macro_use]
        mod crate_root;

        crate_root!();
    "#;
    std::fs::write(dir.join("lib.rs"), root).unwrap();

    let options = CompileOptions {
        input: dir.join("lib.rs").to_string_lossy().into_owned(),
        output: dir.join("libsample.rlib").to_string_lossy().into_owned(),
        emit: EmitKind::Rlib,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("sample".to_string()),
        src_dir: Some(dir.to_string_lossy().into_owned()),
        ..CompileOptions::default()
    };

    let rlib = compile(root, "lib.rs", &options).expect("rlib compilation should succeed");
    let (_, meta) = unpack_rlib(&rlib).expect("rlib should unpack");

    assert!(meta.interface_source.contains("pub mod de"));
    assert!(meta.interface_source.contains("pub trait Deserialize"));
    assert!(meta.interface_source.contains("pub trait Visitor"));
    assert!(meta.interface_source.contains("pub mod ser"));
    assert!(meta.interface_source.contains("pub trait Serialize"));
    assert!(meta
        .interface_source
        .contains("pub use crate::de::{Deserialize, Visitor};"));
    assert!(meta.interface_source.contains("pub use crate::ser::Serialize;"));

    let _ = std::fs::remove_dir_all(&dir);
}

fn push_str16(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}
