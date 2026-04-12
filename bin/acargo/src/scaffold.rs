//! Project scaffolding: `ccargo new` and `ccargo init`.

use crate::prelude::*;
use anyos_std::println;
use crate::fs;

const ANYOS_STD_PATH: &str = "/Libraries/system/src/anyos/libs/stdlib";
const LIBSTD_PATH: &str = "/Libraries/system/src/anyos/libs/libstd";

/// Create a new project directory with standard layout.
pub fn new_project(name: &str, is_lib: bool) {
    if fs::dir_exists(name) {
        println!("ccargo: error: directory `{}` already exists", name);
        return;
    }

    let src_dir = format!("{}/src", name);
    fs::mkdir_p(name);
    fs::mkdir_p(&src_dir);

    // Generate Cargo.toml
    let cargo_toml = if is_lib {
        format!(
            concat!(
                "[package]\n",
                "name = \"{name}\"\n",
                "version = \"0.1.0\"\n",
                "edition = \"2021\"\n",
                "\n",
                "[lib]\n",
                "name = \"{lib_name}\"\n",
                "\n",
                "[dependencies]\n",
                "anyos_std = {{ path = \"{anyos_std_path}\" }}\n",
                "# Optional std shim for std-like crates:\n",
                "# libstd = {{ path = \"{libstd_path}\" }}\n",
                "\n",
                "[profile.dev]\n",
                "panic = \"abort\"\n",
                "opt-level = 2\n",
                "\n",
                "[profile.release]\n",
                "panic = \"abort\"\n",
            ),
            name = name,
            lib_name = name.replace('-', "_"),
            anyos_std_path = ANYOS_STD_PATH,
            libstd_path = LIBSTD_PATH,
        )
    } else {
        format!(
            concat!(
                "[package]\n",
                "name = \"{name}\"\n",
                "version = \"0.1.0\"\n",
                "edition = \"2021\"\n",
                "\n",
                "[dependencies]\n",
                "anyos_std = {{ path = \"{anyos_std_path}\" }}\n",
                "# Optional std shim for std-like crates:\n",
                "# libstd = {{ path = \"{libstd_path}\" }}\n",
                "\n",
                "[profile.dev]\n",
                "panic = \"abort\"\n",
                "opt-level = 2\n",
                "\n",
                "[profile.release]\n",
                "panic = \"abort\"\n",
            ),
            name = name,
            anyos_std_path = ANYOS_STD_PATH,
            libstd_path = LIBSTD_PATH,
        )
    };

    fs::write_file(&format!("{}/Cargo.toml", name), cargo_toml.as_bytes());

    // Generate source file
    if is_lib {
        let lib_rs = concat!(
            "#![no_std]\n",
            "\n",
            "pub fn hello() -> &'static str {\n",
            "    \"Hello from library!\"\n",
            "}\n",
        );
        fs::write_file(&format!("{}/src/lib.rs", name), lib_rs.as_bytes());
    } else {
        let main_rs = concat!(
            "#![no_std]\n",
            "#![no_main]\n",
            "\n",
            "anyos_std::entry!(main);\n",
            "\n",
            "fn main() {\n",
            "    anyos_std::println!(\"Hello, anyOS!\");\n",
            "}\n",
        );
        fs::write_file(&format!("{}/src/main.rs", name), main_rs.as_bytes());
    }

    println!("     Created {} `{}` package", if is_lib { "library" } else { "binary (application)" }, name);
}

/// Initialize a project in an existing directory.
pub fn init_project(dir: &str, is_lib: bool) {
    let cargo_path = format!("{}/Cargo.toml", dir);
    if fs::file_exists(&cargo_path) {
        println!("ccargo: error: `Cargo.toml` already exists in {}", dir);
        return;
    }

    // Extract project name from directory
    let name = if let Some(pos) = dir.rfind('/') {
        &dir[pos + 1..]
    } else {
        dir
    };

    let src_dir = format!("{}/src", dir);
    fs::mkdir_p(&src_dir);

    let cargo_toml = format!(
        concat!(
            "[package]\n",
            "name = \"{name}\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2021\"\n",
            "\n",
            "[dependencies]\n",
            "anyos_std = {{ path = \"{anyos_std_path}\" }}\n",
            "# Optional std shim for std-like crates:\n",
            "# libstd = {{ path = \"{libstd_path}\" }}\n",
            "\n",
            "[profile.dev]\n",
            "panic = \"abort\"\n",
            "opt-level = 2\n",
            "\n",
            "[profile.release]\n",
            "panic = \"abort\"\n",
        ),
        name = name,
        anyos_std_path = ANYOS_STD_PATH,
        libstd_path = LIBSTD_PATH,
    );

    fs::write_file(&cargo_path, cargo_toml.as_bytes());

    // Create source file if it doesn't exist
    let src_path = if is_lib {
        format!("{}/src/lib.rs", dir)
    } else {
        format!("{}/src/main.rs", dir)
    };

    if !fs::file_exists(&src_path) {
        if is_lib {
            fs::write_file(&src_path, b"#![no_std]\n\npub fn hello() -> &'static str {\n    \"Hello!\"\n}\n");
        } else {
            fs::write_file(&src_path, b"#![no_std]\n#![no_main]\n\nanyos_std::entry!(main);\n\nfn main() {\n    anyos_std::println!(\"Hello, anyOS!\");\n}\n");
        }
    }

    println!("     Created {} package in `{}`", if is_lib { "library" } else { "binary" }, dir);
}
